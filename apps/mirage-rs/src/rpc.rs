//! JSON-RPC server surface for `mirage-rs`.

// TODO(UX42-followup): remove `missing_panics_doc` once the remaining
// RPC handlers have accurate panic documentation.
#![allow(
    clippy::default_trait_access,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value,
    clippy::significant_drop_tightening,
    clippy::too_many_lines,
    clippy::uninlined_format_args
)]

use std::{
    convert::Infallible,
    net::{SocketAddr, ToSocketAddrs},
    num::NonZeroUsize,
    sync::{Arc, LazyLock},
    time::Duration,
};

use alloy_primitives::{Address, B256, Bytes, U256, hex, keccak256};
use axum::{
    Router,
    body::Body,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, Request, Response, StatusCode, Uri},
    response::IntoResponse,
    routing::{any, delete, get},
};
use futures_util::{SinkExt, StreamExt};
use jsonrpsee::{
    RpcModule,
    core::{RegisterMethodError, SubscriptionError},
    server::{ServerBuilder, ServerHandle},
    types::ErrorObjectOwned,
};
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use parking_lot::RwLock;
use reqwest::header::{
    CONNECTION, CONTENT_LENGTH, HOST, HeaderName as ReqwestHeaderName, TRANSFER_ENCODING, UPGRADE,
};
use tokio::sync::broadcast;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        Message as TungsteniteMessage,
        client::IntoClientRequest,
        protocol::{CloseFrame as TungsteniteCloseFrame, frame::coding::CloseCode},
    },
};
use tower::Service;
use tower_http::cors::CorsLayer;

use crate::{
    Bytecode, MirageError, Result, TransactionRequest,
    events::MirageTelemetryEvent,
    fork::{
        ClassificationConfig, DiffClassifier, EthFilter, EvmExecutor, ForkState, HybridDB,
        LocalBlock, LocalReceipt, LocalTransaction, MirageFork, MirageState, WatchSource,
        lock_state_writes, with_state_write,
    },
    integration::{
        EventFilter, EventSource, MIRAGE_BEGIN_SCENARIO_SET_METHOD, MIRAGE_DEFINE_SCENARIO_METHOD,
        MIRAGE_GET_POSITION_METHOD, MIRAGE_GET_RESOURCE_USAGE_METHOD,
        MIRAGE_GET_SCENARIO_RESULTS_METHOD, MIRAGE_RUN_SCENARIO_SET_METHOD, MIRAGE_SHUTDOWN_METHOD,
        MIRAGE_STATUS_METHOD, MIRAGE_SUBSCRIBE_EVENTS_METHOD, MIRAGE_WATCH_CONTRACT_METHOD,
        MirageEvent, PositionRequest, PositionSnapshot,
    },
    provider::{BlockTag, UpstreamRpc},
    resources::{MirageMode, PressureAction, Profile, ResourceModel},
    scenario::{
        JobStatus, RunMode, Scenario, ScenarioJob, ScenarioRunner, ScenarioSet, ScenarioSetStatus,
        rank_scenario_results,
    },
};

#[derive(Clone)]
struct ServerContext {
    state: Arc<RwLock<MirageState>>,
    shutdown: broadcast::Sender<()>,
    /// Optional chain substrate for `chain_*` RPC methods. Present iff the
    /// server was started via `start_rpc_server_with_chain`.
    #[cfg(feature = "chain")]
    chain: Option<Arc<RwLock<crate::chain_rpc::ChainContext>>>,
    /// Optional subscription manager for `chain_subscribe*` WS streams
    /// (§38.d). Present when the attached `ChainContext` carries buses.
    #[cfg(feature = "roko")]
    chain_subs: Option<crate::chain_rpc::SubscriptionManager>,
}

#[derive(Debug)]
struct StagedErc20Mint {
    owner: Address,
    balance: U256,
    balance_slot: Option<U256>,
    storage_writes: Vec<(U256, U256)>,
}

#[derive(Clone, Debug)]
struct RelayProxyState {
    upstream_http: String,
    client: reqwest::Client,
}

impl RelayProxyState {
    fn from_env() -> Self {
        let upstream_http = std::env::var("ROKO_AGENT_RELAY_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:9011".to_owned());
        Self {
            upstream_http,
            client: reqwest::Client::new(),
        }
    }

    fn upstream_http_url(&self, uri: &Uri) -> String {
        format!("{}{}", self.upstream_http.trim_end_matches('/'), uri)
    }

    fn upstream_ws_url(&self, uri: &Uri) -> String {
        let base = if let Some(rest) = self.upstream_http.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = self.upstream_http.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            format!("ws://{}", self.upstream_http.trim_start_matches('/'))
        };
        format!("{}{}", base.trim_end_matches('/'), uri)
    }
}

const RELAY_PROXY_BODY_LIMIT: usize = 8 * 1024 * 1024;

/// Canonical ERC-8004 Identity Registry address used by the Ethereum-fork demo path.
pub const ERC8004_IDENTITY_REGISTRY: Address =
    alloy_primitives::address!("0x8004A818BFB912233c491871b3d84c89A494BD9e");
/// Canonical ERC-8004 Reputation Registry address used by the Ethereum-fork demo path.
pub const ERC8004_REPUTATION_REGISTRY: Address =
    alloy_primitives::address!("0x8004A818BFB912233c491871b3d84c89A494Bd9F");
/// Canonical ERC-8004 Validation Registry address used by the Ethereum-fork demo path.
pub const ERC8004_VALIDATION_REGISTRY: Address =
    alloy_primitives::address!("0x8004a818bfb912233c491871B3D84C89A494Bda0");
/// Mirage-local alias for the identity registry documented in the Korai chain emulation docs.
pub const MIRAGE_IDENTITY_REGISTRY_ALIAS: Address =
    alloy_primitives::address!("0x000000000000000000000000000000000000A100");
/// Mirage-local alias for the reputation registry documented in the Korai chain emulation docs.
pub const MIRAGE_REPUTATION_REGISTRY_ALIAS: Address =
    alloy_primitives::address!("0x000000000000000000000000000000000000A200");
/// Mirage-local alias for the validation registry documented in the Korai chain emulation docs.
pub const MIRAGE_VALIDATION_REGISTRY_ALIAS: Address =
    alloy_primitives::address!("0x000000000000000000000000000000000000A300");

static ERC8004_IDENTITY_RUNTIME: LazyLock<Bytecode> = LazyLock::new(|| {
    decode_boot_bytecode(
        "erc8004_identity",
        "608060405234801561000f575f5ffd5b50600436106101cd575f3560e01c8063656afdee11610102578063b88d4fde116100a0578063d19c7f5f1161006f578063d19c7f5f14610579578063e0af096214610595578063e8853111146105c5578063fb3551ff146105e1576101cd565b8063b88d4fde146104da578063c5e32694146104f6578063c87b56dd14610514578063cb22f5fd14610544576101cd565b80639f5679f4116100dc5780639f5679f41461043d5780639f8a13d714610472578063a10d7de6146104a2578063a22cb465146104be576101cd565b8063656afdee146103c157806370a08231146103dd578063891df6711461040d576101cd565b8063292763341161016f5780633defb962116101495780633defb9621461033b57806342842e0e146103455780634f062c5a146103615780636352211e14610391576101cd565b806329276334146102e757806332232ada1461030357806332a9744e1461031f576101cd565b80631a04943d116101ab5780631a04943d1461024d578063210ff9bb1461027d57806323b872dd1461029b57806324638f59146102b7576101cd565b8063047b6e4a146101d1578063095ea7b314610201578063142a7de41461021d575b5f5ffd5b6101eb60048036038101906101e69190611903565b610611565b6040516101f8919061195b565b60405180910390f35b61021b600480360381019061021691906119ce565b610659565b005b61023760048036038101906102329190611a3f565b61068b565b6040516102449190611ac5565b60405180910390f35b61026760048036038101906102629190611b14565b61073a565b604051610274919061195b565b60405180910390f35b610285610780565b6040516102929190611ac5565b60405180910390f35b6102b560048036038101906102b09190611b52565b61078c565b005b6102d160048036038101906102cc9190611c03565b6107be565b6040516102de9190611ac5565b60405180910390f35b61030160048036038101906102fc9190611c99565b6108a4565b005b61031d60048036038101906103189190611c99565b61098e565b005b61033960048036038101906103349190611903565b610a78565b005b610343610b4a565b005b61035f600480360381019061035a9190611b52565b610c4d565b005b61037b60048036038101906103769190611cd7565b610c7f565b6040516103889190611d11565b60405180910390f35b6103ab60048036038101906103a69190611cd7565b610ca8565b6040516103b89190611d39565b60405180910390f35b6103db60048036038101906103d69190611d52565b610ce1565b005b6103f760048036038101906103f29190611daf565b610dc1565b6040516104049190611ac5565b60405180910390f35b61042760048036038101906104229190611cd7565b610e19565b6040516104349190611d39565b60405180910390f35b61045760048036038101906104529190611cd7565b610e5d565b60405161046996959493929190611e68565b60405180910390f35b61048c60048036038101906104879190611daf565b610f53565b604051610499919061195b565b60405180910390f35b6104bc60048036038101906104b79190611ece565b610ff7565b005b6104d860048036038101906104d39190611f43565b6110e6565b005b6104f460048036038101906104ef9190611fd6565b611118565b005b6104fe61114a565b60405161050b919061205a565b60405180910390f35b61052e60048036038101906105299190611cd7565b61114f565b60405161053b9190612073565b60405180910390f35b61055e60048036038101906105599190611cd7565b6111f3565b60405161057096959493929190611e68565b60405180910390f35b610593600480360381019061058e9190612093565b6112d0565b005b6105af60048036038101906105aa9190611daf565b6112df565b6040516105bc9190611ac5565b60405180910390f35b6105df60048036038101906105da9190611c99565b6112f4565b005b6105fb60048036038101906105f69190611daf565b611302565b60405161060891906121cb565b60405180910390f35b5f8167ffffffffffffffff168260025f8681526020019081526020015f205f015f9054906101000a900467ffffffffffffffff161667ffffffffffffffff1614905092915050565b6040517fa4420a9500000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b5f5f60035f8873ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205414610702576040517f3a81d6fc00000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b61072f86866003878760405180602001604052805f81525060405180602001604052805f8152508b61146f565b905095945050505050565b5f5f8260ff166001901b60025f8681526020019081526020015f205f015f9054906101000a900467ffffffffffffffff161667ffffffffffffffff161415905092915050565b5f600180549050905090565b6040517fa4420a9500000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b5f5f60035f8973ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205414610835576040517f3a81d6fc00000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b610898878787875f5f1b88888080601f0160208091040260200160405190810160405280939291908181526020018383808284375f81840152601f19601f8201169050808301925050505050505060405180602001604052805f8152508b61146f565b90509695505050505050565b3373ffffffffffffffffffffffffffffffffffffffff1660045f8481526020019081526020015f205f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1614610939576040517ff147d5a800000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b8060025f8481526020019081526020015f2060020181905550817fc447b20909591b186acb2180f62bcdd7bad2a6954d7f2e31a7ce76f26728e5288260405161098291906121eb565b60405180910390a25050565b3373ffffffffffffffffffffffffffffffffffffffff1660045f8481526020019081526020015f205f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1614610a23576040517ff147d5a800000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b8060025f8481526020019081526020015f2060010181905550817f3de9408b25e68ef7822dd91f8f63676f98a2b1779bf3314fa3987c23b0ff3de582604051610a6c91906121eb565b60405180910390a25050565b3373ffffffffffffffffffffffffffffffffffffffff1660045f8481526020019081526020015f205f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1614610b0d576040517ff147d5a800000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b8060025f8481526020019081526020015f205f015f6101000a81548167ffffffffffffffff021916908367ffffffffffffffff1602179055505050565b5f5f5f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2090508060020160109054906101000a900460ff16610bd1576040517faba4733900000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b438160020160086101000a81548167ffffffffffffffff021916908367ffffffffffffffff1602179055503373ffffffffffffffffffffffffffffffffffffffff167f290c71822ca7388f807d9ce3f9f023fe93b8000082daa4de512bf7ff84244dd243604051610c42919061205a565b60405180910390a250565b6040517fa4420a9500000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b5f60025f8381526020019081526020015f205f0160089054906101000a900460ff169050919050565b5f60045f8381526020019081526020015f205f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff169050919050565b5f60035f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205414610d57576040517f3a81d6fc00000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b610dbb335f6003845f5f1b60405180602001604052805f81525089898080601f0160208091040260200160405190810160405280939291908181526020018383808284375f81840152601f19601f820116905080830192505050505050508861146f565b50505050565b5f5f60035f8473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205414610e0d576001610e0f565b5f5b60ff169050919050565b5f60018281548110610e2e57610e2d612204565b5b905f5260205f20015f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff169050919050565b5f5f5f5f5f60605f60025f8981526020019081526020015f209050805f015f9054906101000a900467ffffffffffffffff16815f0160089054906101000a900460ff1682600101548360020154846003015485600401808054610ebf9061225e565b80601f0160208091040260200160405190810160405280929190818152602001828054610eeb9061225e565b8015610f365780601f10610f0d57610100808354040283529160200191610f36565b820191905f5260205f20905b815481529060010190602001808311610f1957829003601f168201915b505050505090509650965096509650965096505091939550919395565b5f5f5f5f8473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2090508060020160109054906101000a900460ff16610fb2575f915050610ff2565b60c867ffffffffffffffff168160020160089054906101000a900467ffffffffffffffff1667ffffffffffffffff1643610fec91906122bb565b11159150505b919050565b5f5f5f3373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2090508060020160109054906101000a900460ff1661107e576040517faba4733900000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b8282825f0191826110909291906124c5565b503373ffffffffffffffffffffffffffffffffffffffff167fcd962bc3c4bc4adc020e5dd8dd8e26ed49da802778485194f204bf85efd64c3d84846040516110d99291906125cc565b60405180910390a2505050565b6040517fa4420a9500000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b6040517fa4420a9500000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b60c881565b606060025f8381526020019081526020015f2060040180546111709061225e565b80601f016020809104026020016040519081016040528092919081815260200182805461119c9061225e565b80156111e75780601f106111be576101008083540402835291602001916111e7565b820191905f5260205f20905b8154815290600101906020018083116111ca57829003601f168201915b50505050509050919050565b6002602052805f5260405f205f91509050805f015f9054906101000a900467ffffffffffffffff1690805f0160089054906101000a900460ff169080600101549080600201549080600301549080600401805461124f9061225e565b80601f016020809104026020016040519081016040528092919081815260200182805461127b9061225e565b80156112c65780601f1061129d576101008083540402835291602001916112c6565b820191905f5260205f20905b8154815290600101906020018083116112a957829003601f168201915b5050505050905086565b6112da83836108a4565b505050565b6003602052805f5260405f205f915090505481565b6112fe828261098e565b5050565b61130a61184a565b5f5f8373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f206040518060a00160405290815f820180546113609061225e565b80601f016020809104026020016040519081016040528092919081815260200182805461138c9061225e565b80156113d75780601f106113ae576101008083540402835291602001916113d7565b820191905f5260205f20905b8154815290600101906020018083116113ba57829003601f168201915b5050505050815260200160018201548152602001600282015f9054906101000a900467ffffffffffffffff1667ffffffffffffffff1667ffffffffffffffff1681526020016002820160089054906101000a900467ffffffffffffffff1667ffffffffffffffff1667ffffffffffffffff1681526020016002820160109054906101000a900460ff1615151515815250509050919050565b5f60038760ff1611156114ae576040517fe142361700000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b60055f8154809291906114c0906125ee565b9190505590506040518060c001604052808967ffffffffffffffff1681526020018860ff1681526020018781526020018681526020014381526020018581525060025f8381526020019081526020015f205f820151815f015f6101000a81548167ffffffffffffffff021916908367ffffffffffffffff1602179055506020820151815f0160086101000a81548160ff021916908360ff16021790555060408201518160010155606082015181600201556080820151816003015560a08201518160040190816115909190612635565b509050508060035f8b73ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20819055508860045f8381526020019081526020015f205f6101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055506040518060a001604052808481526020018381526020014367ffffffffffffffff1681526020014367ffffffffffffffff168152602001600115158152505f5f8b73ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f820151815f0190816116b29190612635565b50602082015181600101556040820151816002015f6101000a81548167ffffffffffffffff021916908367ffffffffffffffff16021790555060608201518160020160086101000a81548167ffffffffffffffff021916908367ffffffffffffffff16021790555060808201518160020160106101000a81548160ff021916908315150217905550905050600189908060018154018082558091505060019003905f5260205f20015f9091909190916101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055508873ffffffffffffffffffffffffffffffffffffffff167f44beda3d7d201bb876725c6a34391cbd97bb0b57b82c15cd856080b7e152b81b83856040516117e5929190612704565b60405180910390a2808973ffffffffffffffffffffffffffffffffffffffff167f4db3a5617370d36215eeb4e9d647271113f60967114f0ba75dfc914e83a6a50e898b604051611836929190612732565b60405180910390a398975050505050505050565b6040518060a00160405280606081526020015f81526020015f67ffffffffffffffff1681526020015f67ffffffffffffffff1681526020015f151581525090565b5f5ffd5b5f5ffd5b5f819050919050565b6118a581611893565b81146118af575f5ffd5b50565b5f813590506118c08161189c565b92915050565b5f67ffffffffffffffff82169050919050565b6118e2816118c6565b81146118ec575f5ffd5b50565b5f813590506118fd816118d9565b92915050565b5f5f604083850312156119195761191861188b565b5b5f611926858286016118b2565b9250506020611937858286016118ef565b9150509250929050565b5f8115159050919050565b61195581611941565b82525050565b5f60208201905061196e5f83018461194c565b92915050565b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f61199d82611974565b9050919050565b6119ad81611993565b81146119b7575f5ffd5b50565b5f813590506119c8816119a4565b92915050565b5f5f604083850312156119e4576119e361188b565b5b5f6119f1858286016119ba565b9250506020611a02858286016118b2565b9150509250929050565b5f819050919050565b611a1e81611a0c565b8114611a28575f5ffd5b50565b5f81359050611a3981611a15565b92915050565b5f5f5f5f5f60a08688031215611a5857611a5761188b565b5b5f611a65888289016119ba565b9550506020611a76888289016118ef565b9450506040611a8788828901611a2b565b9350506060611a9888828901611a2b565b9250506080611aa9888289016118ef565b9150509295509295909350565b611abf81611893565b82525050565b5f602082019050611ad85f830184611ab6565b92915050565b5f60ff82169050919050565b611af381611ade565b8114611afd575f5ffd5b50565b5f81359050611b0e81611aea565b92915050565b5f5f60408385031215611b2a57611b2961188b565b5b5f611b37858286016118b2565b9250506020611b4885828601611b00565b9150509250929050565b5f5f5f60608486031215611b6957611b6861188b565b5b5f611b76868287016119ba565b9350506020611b87868287016119ba565b9250506040611b98868287016118b2565b9150509250925092565b5f5ffd5b5f5ffd5b5f5ffd5b5f5f83601f840112611bc357611bc2611ba2565b5b8235905067ffffffffffffffff811115611be057611bdf611ba6565b5b602083019150836001820283011115611bfc57611bfb611baa565b5b9250929050565b5f5f5f5f5f5f60a08789031215611c1d57611c1c61188b565b5b5f611c2a89828a016119ba565b9650506020611c3b89828a016118ef565b9550506040611c4c89828a01611b00565b9450506060611c5d89828a01611a2b565b935050608087013567ffffffffffffffff811115611c7e57611c7d61188f565b5b611c8a89828a01611bae565b92509250509295509295509295565b5f5f60408385031215611caf57611cae61188b565b5b5f611cbc858286016118b2565b9250506020611ccd85828601611a2b565b9150509250929050565b5f60208284031215611cec57611ceb61188b565b5b5f611cf9848285016118b2565b91505092915050565b611d0b81611ade565b82525050565b5f602082019050611d245f830184611d02565b92915050565b611d3381611993565b82525050565b5f602082019050611d4c5f830184611d2a565b92915050565b5f5f5f60408486031215611d6957611d6861188b565b5b5f84013567ffffffffffffffff811115611d8657611d8561188f565b5b611d9286828701611bae565b93509350506020611da586828701611a2b565b9150509250925092565b5f60208284031215611dc457611dc361188b565b5b5f611dd1848285016119ba565b91505092915050565b611de3816118c6565b82525050565b611df281611a0c565b82525050565b5f81519050919050565b5f82825260208201905092915050565b8281835e5f83830152505050565b5f601f19601f8301169050919050565b5f611e3a82611df8565b611e448185611e02565b9350611e54818560208601611e12565b611e5d81611e20565b840191505092915050565b5f60c082019050611e7b5f830189611dda565b611e886020830188611d02565b611e956040830187611de9565b611ea26060830186611de9565b611eaf6080830185611ab6565b81810360a0830152611ec18184611e30565b9050979650505050505050565b5f5f60208385031215611ee457611ee361188b565b5b5f83013567ffffffffffffffff811115611f0157611f0061188f565b5b611f0d85828601611bae565b92509250509250929050565b611f2281611941565b8114611f2c575f5ffd5b50565b5f81359050611f3d81611f19565b92915050565b5f5f60408385031215611f5957611f5861188b565b5b5f611f66858286016119ba565b9250506020611f7785828601611f2f565b9150509250929050565b5f5f83601f840112611f9657611f95611ba2565b5b8235905067ffffffffffffffff811115611fb357611fb2611ba6565b5b602083019150836001820283011115611fcf57611fce611baa565b5b9250929050565b5f5f5f5f5f60808688031215611fef57611fee61188b565b5b5f611ffc888289016119ba565b955050602061200d888289016119ba565b945050604061201e888289016118b2565b935050606086013567ffffffffffffffff81111561203f5761203e61188f565b5b61204b88828901611f81565b92509250509295509295909350565b5f60208201905061206d5f830184611dda565b92915050565b5f6020820190508181035f83015261208b8184611e30565b905092915050565b5f5f5f606084860312156120aa576120a961188b565b5b5f6120b7868287016118b2565b93505060206120c886828701611a2b565b92505060406120d9868287016118ef565b9150509250925092565b5f82825260208201905092915050565b5f6120fd82611df8565b61210781856120e3565b9350612117818560208601611e12565b61212081611e20565b840191505092915050565b61213481611a0c565b82525050565b612143816118c6565b82525050565b61215281611941565b82525050565b5f60a083015f8301518482035f86015261217282826120f3565b9150506020830151612187602086018261212b565b50604083015161219a604086018261213a565b5060608301516121ad606086018261213a565b5060808301516121c06080860182612149565b508091505092915050565b5f6020820190508181035f8301526121e38184612158565b905092915050565b5f6020820190506121fe5f830184611de9565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b7f4e487b71000000000000000000000000000000000000000000000000000000005f52602260045260245ffd5b5f600282049050600182168061227557607f821691505b60208210810361228857612287612231565b5b50919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f6122c582611893565b91506122d083611893565b92508282039050818111156122e8576122e761228e565b5b92915050565b5f82905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f819050815f5260205f209050919050565b5f6020601f8301049050919050565b5f82821b905092915050565b5f600883026123817fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82612346565b61238b8683612346565b95508019841693508086168417925050509392505050565b5f819050919050565b5f6123c66123c16123bc84611893565b6123a3565b611893565b9050919050565b5f819050919050565b6123df836123ac565b6123f36123eb826123cd565b848454612352565b825550505050565b5f5f905090565b61240a6123fb565b6124158184846123d6565b505050565b5b818110156124385761242d5f82612402565b60018101905061241b565b5050565b601f82111561247d5761244e81612325565b61245784612337565b81016020851015612466578190505b61247a61247285612337565b83018261241a565b50505b505050565b5f82821c905092915050565b5f61249d5f1984600802612482565b1980831691505092915050565b5f6124b5838361248e565b9150826002028217905092915050565b6124cf83836122ee565b67ffffffffffffffff8111156124e8576124e76122f8565b5b6124f2825461225e565b6124fd82828561243c565b5f601f83116001811461252a575f8415612518578287013590505b61252285826124aa565b865550612589565b601f19841661253886612325565b5f5b8281101561255f5784890135825560018201915060208501945060208101905061253a565b8683101561257c5784890135612578601f89168261248e565b8355505b6001600288020188555050505b50505050505050565b828183375f83830152505050565b5f6125ab8385611e02565b93506125b8838584612592565b6125c183611e20565b840190509392505050565b5f6020820190508181035f8301526125e58184866125a0565b90509392505050565b5f6125f882611893565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff820361262a5761262961228e565b5b600182019050919050565b61263e82611df8565b67ffffffffffffffff811115612657576126566122f8565b5b612661825461225e565b61266c82828561243c565b5f60209050601f83116001811461269d575f841561268b578287015190505b61269585826124aa565b8655506126fc565b601f1984166126ab86612325565b5f5b828110156126d2578489015182556001820191506020850194506020810190506126ad565b868310156126ef57848901516126eb601f89168261248e565b8355505b6001600288020188555050505b505050505050565b5f6040820190506127175f830185611de9565b81810360208301526127298184611e30565b90509392505050565b5f6040820190506127455f830185611d02565b6127526020830184611dda565b939250505056fea2646970667358221220a176a4391e901a1bf400f218918f6ddc3e47d1b5fe44f782f00149b4bd3a623d64736f6c634300081e0033",
    )
});
static ERC8004_UNSUPPORTED_RUNTIME: LazyLock<Bytecode> = LazyLock::new(|| {
    decode_boot_bytecode(
        "erc8004_unsupported",
        "608060405236610044576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161003b906100ff565b60405180910390fd5b6040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610076906100ff565b60405180910390fd5b5f82825260208201905092915050565b7f455243383030342072656769737472792073757266616365206e6f7420696d705f8201527f6c656d656e74656420696e206d69726167650000000000000000000000000000602082015250565b5f6100e960328361007f565b91506100f48261008f565b604082019050919050565b5f6020820190508181035f830152610116816100dd565b905091905056fea26469706673582212201e20d0afe57c74ffb9bbabb2d6d987d479801da8c0bf1eb0e79d66a3ffe197c164736f6c634300081e0033",
    )
});
static ERC8004_IDENTITY_ALIAS_RUNTIME: LazyLock<Bytecode> = LazyLock::new(|| {
    decode_boot_bytecode(
        "erc8004_identity_alias",
        "608060405236603b575f738004a818bfb912233c491871b3d84c89a494bd9e90505f5f5f5f34855af13d5f5f3e805f81146037573d5ff35b3d5ffd5b5f738004a818bfb912233c491871b3d84c89a494bd9e9050365f5f375f5f365f34855af13d5f5f3e805f8114606e573d5ff35b3d5ffdfea264697066735822122024c32fbaf18837eadc409ee0f2ae757f22c1230a73ebf9d9f21a2f60b8b88ec264736f6c634300081e0033",
    )
});
static ERC8004_REPUTATION_ALIAS_RUNTIME: LazyLock<Bytecode> = LazyLock::new(|| {
    decode_boot_bytecode(
        "erc8004_reputation_alias",
        "608060405236603b575f738004a818bfb912233c491871b3d84c89a494bd9f90505f5f5f5f34855af13d5f5f3e805f81146037573d5ff35b3d5ffd5b5f738004a818bfb912233c491871b3d84c89a494bd9f9050365f5f375f5f365f34855af13d5f5f3e805f8114606e573d5ff35b3d5ffdfea26469706673582212207e468f4bbcfcc4e8a9960f74b9998c0475976b5d12184ce3e8a871c1094160b564736f6c634300081e0033",
    )
});
static ERC8004_VALIDATION_ALIAS_RUNTIME: LazyLock<Bytecode> = LazyLock::new(|| {
    decode_boot_bytecode(
        "erc8004_validation_alias",
        "608060405236603b575f738004a818bfb912233c491871b3d84c89a494bda090505f5f5f5f34855af13d5f5f3e805f81146037573d5ff35b3d5ffd5b5f738004a818bfb912233c491871b3d84c89a494bda09050365f5f375f5f365f34855af13d5f5f3e805f8114606e573d5ff35b3d5ffdfea2646970667358221220f39e6f28720437fe3780b7568e658aa14d4382a48ff7e452ab29b3bf546ae20564736f6c634300081e0033",
    )
});

fn decode_boot_bytecode(label: &'static str, hex_bytes: &'static str) -> Bytecode {
    Bytecode::new_raw(Bytes::from(
        hex::decode(hex_bytes).unwrap_or_else(|error| panic!("invalid {label} bytecode: {error}")),
    ))
}

fn boot_contract_specs() -> [(&'static str, Address, &'static LazyLock<Bytecode>); 6] {
    [
        (
            "erc8004_identity_registry",
            ERC8004_IDENTITY_REGISTRY,
            &ERC8004_IDENTITY_RUNTIME,
        ),
        (
            "erc8004_reputation_registry",
            ERC8004_REPUTATION_REGISTRY,
            &ERC8004_UNSUPPORTED_RUNTIME,
        ),
        (
            "erc8004_validation_registry",
            ERC8004_VALIDATION_REGISTRY,
            &ERC8004_UNSUPPORTED_RUNTIME,
        ),
        (
            "mirage_identity_registry_alias",
            MIRAGE_IDENTITY_REGISTRY_ALIAS,
            &ERC8004_IDENTITY_ALIAS_RUNTIME,
        ),
        (
            "mirage_reputation_registry_alias",
            MIRAGE_REPUTATION_REGISTRY_ALIAS,
            &ERC8004_REPUTATION_ALIAS_RUNTIME,
        ),
        (
            "mirage_validation_registry_alias",
            MIRAGE_VALIDATION_REGISTRY_ALIAS,
            &ERC8004_VALIDATION_ALIAS_RUNTIME,
        ),
    ]
}

fn account_has_code(fork: &mut ForkState, address: Address) -> Result<bool> {
    let Some(info) = fork.db.basic(address)? else {
        return Ok(false);
    };
    if let Some(code) = info.code {
        return Ok(!code.bytecode().is_empty());
    }
    Ok(info.code_hash != Bytecode::default().hash_slow())
}

/// Returns the boot-time ERC-8004 address book exposed by mirage.
#[must_use]
pub fn erc8004_boot_address_book() -> serde_json::Value {
    serde_json::json!({
        "identity": {
            "canonical": format!("{ERC8004_IDENTITY_REGISTRY:#x}"),
            "localAlias": format!("{MIRAGE_IDENTITY_REGISTRY_ALIAS:#x}"),
        },
        "reputation": {
            "canonical": format!("{ERC8004_REPUTATION_REGISTRY:#x}"),
            "localAlias": format!("{MIRAGE_REPUTATION_REGISTRY_ALIAS:#x}"),
        },
        "validation": {
            "canonical": format!("{ERC8004_VALIDATION_REGISTRY:#x}"),
            "localAlias": format!("{MIRAGE_VALIDATION_REGISTRY_ALIAS:#x}"),
        }
    })
}

/// Ensures the ERC-8004 registry surface exists in local fork state when absent upstream.
///
/// Installs the canonical mainnet/demo addresses plus mirage-local aliases documented by the
/// Korai chain emulation surface. The Rust `chain::AgentRegistry` remains runtime-only metadata;
/// the durable identity source is this on-chain contract surface.
///
/// # Errors
///
/// Returns account-read errors if the existing fork state cannot be inspected.
pub fn ensure_erc8004_boot_contracts(fork: &mut ForkState) -> Result<Vec<&'static str>> {
    let mut installed = Vec::new();
    for (label, address, bytecode) in boot_contract_specs() {
        if account_has_code(fork, address)? {
            continue;
        }
        fork.db.set_code(address, (*bytecode).clone());
        fork.db.set_nonce(address, 1);
        installed.push(label);
    }
    Ok(installed)
}

/// Starts a JSON-RPC server on the provided address.
///
/// # Errors
///
/// Returns address-resolution errors, [`MirageError::Unsupported`] if the RPC
/// module cannot be built, or [`MirageError::BindFailed`] if the listener
/// cannot bind the chosen port.
///
/// When the `chain` feature is enabled, use [`start_rpc_server_with_chain`] to
/// attach an [`crate::chain_rpc::ChainContext`] that exposes `chain_*` methods.
pub async fn start_rpc_server(
    address: impl ToSocketAddrs,
    mirage: MirageFork,
    shutdown: broadcast::Sender<()>,
) -> Result<(SocketAddr, ServerHandle)> {
    let address = address
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| MirageError::Unsupported("no socket address resolved".to_owned()))?;
    let local_state = mirage.state();
    let module = build_rpc_module(ServerContext {
        state: Arc::clone(&local_state),
        shutdown,
        #[cfg(feature = "chain")]
        chain: None,
        #[cfg(feature = "roko")]
        chain_subs: None,
    })
    .map_err(|error| MirageError::Unsupported(error.to_string()))?;
    tracing::info!(
        "dashboard-compatible REST surface disabled in pure EVM mode; use roko-serve /api for dashboard traffic"
    );
    finish_start_rpc_server(address, module, local_state, None).await
}

/// Starts a JSON-RPC server with an attached chain substrate.
///
/// # Errors
///
/// Returns address-resolution errors, [`MirageError::Unsupported`] if the RPC
/// module cannot be built, or [`MirageError::BindFailed`] if the listener
/// cannot bind the chosen port.
///
/// Registers all `chain_*` methods described in [`crate::chain_rpc`] on top
/// of the standard `eth_*` / `mirage_*` surface.
#[cfg(feature = "chain")]
pub async fn start_rpc_server_with_chain(
    address: impl ToSocketAddrs,
    mirage: MirageFork,
    shutdown: broadcast::Sender<()>,
    chain: Arc<RwLock<crate::chain_rpc::ChainContext>>,
) -> Result<(SocketAddr, ServerHandle)> {
    let address = address
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| MirageError::Unsupported("no socket address resolved".to_owned()))?;
    let local_state = mirage.state();
    #[cfg(feature = "roko")]
    let chain_subs = {
        let guard = chain.read();
        match (guard.pheromone_bus.clone(), guard.insight_bus.clone()) {
            (Some(p), Some(i)) => Some(crate::chain_rpc::SubscriptionManager::new(p, i)),
            _ => None,
        }
    };
    #[cfg(feature = "legacy-api")]
    let api_router = {
        tracing::info!(
            "legacy mirage /api surface enabled for migration compatibility; prefer roko-serve /api for dashboard traffic"
        );
        let block_state = Arc::clone(&local_state);
        let api_state = crate::http_api::ApiState {
            chain: chain.clone(),
            current_block: Arc::new(move || block_state.read().fork.local_block_number),
            projection_cache: crate::http_api::ProjectionCache::new(4096),
            started_at: std::time::Instant::now(),
            #[cfg(feature = "roko")]
            subs: chain_subs.clone(),
        };
        Some(crate::http_api::build_router(api_state))
    };
    #[cfg(not(feature = "legacy-api"))]
    let api_router = {
        tracing::info!(
            "legacy mirage /api surface disabled; use roko-serve /api aggregator for dashboard-compatible routes"
        );
        None
    };
    let module = build_rpc_module(ServerContext {
        state: Arc::clone(&local_state),
        shutdown,
        chain: Some(chain),
        #[cfg(feature = "roko")]
        chain_subs,
    })
    .map_err(|error| MirageError::Unsupported(error.to_string()))?;
    finish_start_rpc_server(address, module, local_state, api_router).await
}

/// Middleware that adds `Cache-Control: no-cache` headers to `/dashboard` responses.
async fn dashboard_cache_control(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response<Body> {
    let is_dashboard = request.uri().path().starts_with("/dashboard");
    let mut response = next.run(request).await;
    if is_dashboard {
        response.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache, must-revalidate"),
        );
    }
    response
}

fn build_relay_proxy_router() -> Router<()> {
    build_relay_proxy_router_with_state(RelayProxyState::from_env())
}

/// Builds the same-origin `/relay/*` proxy router against an explicit relay upstream.
///
/// This exists so integration tests can exercise the real mirage relay-forwarding
/// handlers without relying on process-global environment mutation.
#[doc(hidden)]
#[must_use]
pub fn build_relay_proxy_router_for_tests(upstream_http: String) -> Router<()> {
    build_relay_proxy_router_with_state(RelayProxyState {
        upstream_http,
        client: reqwest::Client::new(),
    })
}

fn build_relay_proxy_router_with_state(state: RelayProxyState) -> Router<()> {
    Router::new()
        .route("/relay/agents/ws", get(relay_proxy_ws))
        .route("/relay/events/ws", get(relay_proxy_ws))
        .route("/relay", any(relay_proxy_http))
        .route("/relay/{*path}", any(relay_proxy_http))
        .with_state(state)
}

async fn relay_proxy_http(
    State(state): State<RelayProxyState>,
    request: Request<Body>,
) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let upstream_url = state.upstream_http_url(&parts.uri);
    let body = match axum::body::to_bytes(body, RELAY_PROXY_BODY_LIMIT).await {
        Ok(body) => body,
        Err(error) => {
            tracing::warn!(%error, "failed to read relay proxy request body");
            return relay_proxy_error(StatusCode::BAD_REQUEST, "invalid relay request body");
        }
    };

    let mut upstream = state.client.request(parts.method.clone(), upstream_url);
    for (name, value) in &parts.headers {
        if is_hop_by_hop_header(name) {
            continue;
        }
        upstream = upstream.header(name, value);
    }

    let upstream_response = match upstream.body(body).send().await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(%error, "relay proxy upstream request failed");
            return relay_proxy_error(StatusCode::BAD_GATEWAY, "relay upstream unavailable");
        }
    };

    let status = upstream_response.status();
    let headers = upstream_response.headers().clone();
    let body = match upstream_response.bytes().await {
        Ok(body) => body,
        Err(error) => {
            tracing::warn!(%error, "failed to read relay proxy upstream response body");
            return relay_proxy_error(StatusCode::BAD_GATEWAY, "relay upstream response invalid");
        }
    };

    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    for (name, value) in &headers {
        if is_hop_by_hop_header(name) {
            continue;
        }
        response.headers_mut().append(name, value.clone());
    }
    response
}

async fn relay_proxy_ws(
    State(state): State<RelayProxyState>,
    ws: WebSocketUpgrade,
    uri: Uri,
    headers: HeaderMap,
) -> Response<Body> {
    let upstream_url = state.upstream_ws_url(&uri);
    let mut upstream_request = match upstream_url.clone().into_client_request() {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!(%error, "failed to construct relay websocket request");
            return relay_proxy_error(StatusCode::BAD_GATEWAY, "relay websocket request invalid");
        }
    };
    if let Some(protocol) = headers.get(axum::http::header::SEC_WEBSOCKET_PROTOCOL) {
        upstream_request
            .headers_mut()
            .insert(axum::http::header::SEC_WEBSOCKET_PROTOCOL, protocol.clone());
    }

    let upstream_socket = match connect_async(upstream_request).await {
        Ok((socket, _response)) => socket,
        Err(error) => {
            tracing::warn!(%error, %upstream_url, "failed to connect relay websocket upstream");
            return relay_proxy_error(StatusCode::BAD_GATEWAY, "relay websocket unavailable");
        }
    };

    ws.on_upgrade(move |socket| bridge_relay_websocket(socket, upstream_socket))
        .into_response()
}

async fn bridge_relay_websocket(
    downstream: WebSocket,
    upstream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    let (mut downstream_tx, mut downstream_rx) = downstream.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();

    loop {
        tokio::select! {
            message = downstream_rx.next() => {
                match message {
                    Some(Ok(message)) => {
                        if forward_downstream_ws(message, &mut upstream_tx).await.is_err() {
                            break;
                        }
                    }
                    Some(Err(error)) => {
                        tracing::warn!(%error, "relay proxy downstream websocket receive failed");
                        break;
                    }
                    None => break,
                }
            }
            message = upstream_rx.next() => {
                match message {
                    Some(Ok(message)) => {
                        if forward_upstream_ws(message, &mut downstream_tx).await.is_err() {
                            break;
                        }
                    }
                    Some(Err(error)) => {
                        tracing::warn!(%error, "relay proxy upstream websocket receive failed");
                        break;
                    }
                    None => break,
                }
            }
        }
    }
}

async fn forward_downstream_ws(
    message: Message,
    upstream_tx: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        TungsteniteMessage,
    >,
) -> std::result::Result<(), tokio_tungstenite::tungstenite::Error> {
    match message {
        Message::Text(text) => {
            upstream_tx
                .send(TungsteniteMessage::Text(text.to_string().into()))
                .await
        }
        Message::Binary(binary) => upstream_tx.send(TungsteniteMessage::Binary(binary)).await,
        Message::Ping(payload) => upstream_tx.send(TungsteniteMessage::Ping(payload)).await,
        Message::Pong(payload) => upstream_tx.send(TungsteniteMessage::Pong(payload)).await,
        Message::Close(frame) => {
            let frame = frame.map(|frame| TungsteniteCloseFrame {
                code: axum_close_code_to_tungstenite(frame.code),
                reason: frame.reason.to_string().into(),
            });
            upstream_tx.send(TungsteniteMessage::Close(frame)).await
        }
    }
}

async fn forward_upstream_ws(
    message: TungsteniteMessage,
    downstream_tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> std::result::Result<(), axum::Error> {
    match message {
        TungsteniteMessage::Text(text) => {
            downstream_tx
                .send(Message::Text(text.to_string().into()))
                .await
        }
        TungsteniteMessage::Binary(binary) => downstream_tx.send(Message::Binary(binary)).await,
        TungsteniteMessage::Ping(payload) => downstream_tx.send(Message::Ping(payload)).await,
        TungsteniteMessage::Pong(payload) => downstream_tx.send(Message::Pong(payload)).await,
        TungsteniteMessage::Close(frame) => {
            let frame = frame.map(|frame| axum::extract::ws::CloseFrame {
                code: tungstenite_close_code_to_axum(frame.code),
                reason: frame.reason.to_string().into(),
            });
            downstream_tx.send(Message::Close(frame)).await
        }
        TungsteniteMessage::Frame(_) => Ok(()),
    }
}

fn relay_proxy_error(status: StatusCode, message: &'static str) -> Response<Body> {
    let mut response = Response::new(Body::from(message));
    *response.status_mut() = status;
    response
}

fn is_hop_by_hop_header(name: &ReqwestHeaderName) -> bool {
    matches!(
        name,
        &CONNECTION | &CONTENT_LENGTH | &HOST | &TRANSFER_ENCODING | &UPGRADE
    )
}

fn axum_close_code_to_tungstenite(code: u16) -> CloseCode {
    match code {
        1000 => CloseCode::Normal,
        1001 => CloseCode::Away,
        1002 => CloseCode::Protocol,
        1003 => CloseCode::Unsupported,
        1005 => CloseCode::Status,
        1006 => CloseCode::Abnormal,
        1007 => CloseCode::Invalid,
        1008 => CloseCode::Policy,
        1009 => CloseCode::Size,
        1010 => CloseCode::Extension,
        1011 => CloseCode::Error,
        1012 => CloseCode::Restart,
        1013 => CloseCode::Again,
        1015 => CloseCode::Tls,
        value => CloseCode::Library(value),
    }
}

fn tungstenite_close_code_to_axum(code: CloseCode) -> u16 {
    match code {
        CloseCode::Normal => 1000,
        CloseCode::Away => 1001,
        CloseCode::Protocol => 1002,
        CloseCode::Unsupported => 1003,
        CloseCode::Status => 1005,
        CloseCode::Abnormal => 1006,
        CloseCode::Invalid => 1007,
        CloseCode::Policy => 1008,
        CloseCode::Size => 1009,
        CloseCode::Extension => 1010,
        CloseCode::Error => 1011,
        CloseCode::Restart => 1012,
        CloseCode::Again => 1013,
        CloseCode::Tls => 1015,
        CloseCode::Reserved(value) | CloseCode::Iana(value) | CloseCode::Library(value) => value,
        CloseCode::Bad(value) => value,
    }
}

async fn finish_start_rpc_server(
    address: SocketAddr,
    module: RpcModule<ServerContext>,
    local_state: Arc<RwLock<MirageState>>,
    api_router: Option<Router>,
) -> Result<(SocketAddr, ServerHandle)> {
    let (stop_handle, server_handle) = jsonrpsee::server::stop_channel();
    let rpc_service = ServerBuilder::default()
        .to_service_builder()
        .build(module, stop_handle);
    let rpc_fallback = tower::service_fn(move |request: Request<Body>| {
        let mut rpc_service = rpc_service.clone();
        async move {
            match Service::call(&mut rpc_service, request).await {
                Ok(response) => Ok::<Response<Body>, Infallible>(response.map(Body::new)),
                Err(error) => {
                    tracing::warn!("jsonrpsee rpc service failed: {error}");
                    let mut response =
                        Response::new(Body::from("internal server error".to_owned()));
                    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                    Ok(response)
                }
            }
        }
    });
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|_| MirageError::BindFailed(address.port()))?;
    let local_addr = listener
        .local_addr()
        .map_err(|_| MirageError::BindFailed(address.port()))?;
    let mut app = Router::new()
        .route("/health", get(health_handler))
        .route("/events/{stream_id}", get(event_ws_handler))
        .route("/events/{stream_id}", delete(unsubscribe_event_handler))
        .with_state(local_state);
    app = app.merge(build_relay_proxy_router());
    if let Some(api) = api_router {
        app = app.nest("/api", api);
    }
    // The JSON-RPC fallback must only handle POST — otherwise it catches GET
    // requests to /api/* before the nested router can match them, causing all
    // REST endpoints to return the JSON-RPC "POST is required" error.
    let rpc_post_only = tower::service_fn(move |request: Request<Body>| {
        let mut rpc = rpc_fallback.clone();
        async move {
            let is_ws_upgrade = request
                .headers()
                .get(axum::http::header::UPGRADE)
                .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"));
            if request.method() != axum::http::Method::POST && !is_ws_upgrade {
                let mut response = Response::new(Body::from("Not Found"));
                *response.status_mut() = StatusCode::NOT_FOUND;
                return Ok::<Response<Body>, Infallible>(response);
            }
            rpc.call(request).await
        }
    });
    app = app.fallback_service(rpc_post_only);
    // Serve the dashboard UI from the static/ directory if present.
    // Checks: $MIRAGE_DASHBOARD_DIR, ./static/, and the binary's sibling static/.
    let dashboard_dir = std::env::var("MIRAGE_DASHBOARD_DIR").ok().or_else(|| {
        let candidates = [
            std::path::PathBuf::from("static"),
            std::path::PathBuf::from("apps/mirage-rs/static"),
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("static")))
                .unwrap_or_default(),
        ];
        candidates
            .into_iter()
            .find(|p| p.join("index.html").exists())
            .map(|p| p.to_string_lossy().into_owned())
    });
    if let Some(dir) = dashboard_dir {
        let serve_dir =
            tower_http::services::ServeDir::new(&dir).append_index_html_on_directories(true);
        app = app.nest_service("/dashboard", serve_dir);
        // Add no-cache middleware for dashboard static files
        app = app.layer(axum::middleware::from_fn(dashboard_cache_control));
        tracing::info!("dashboard UI served at /dashboard from {dir}");
    }
    let app = app.layer(CorsLayer::permissive());
    let shutdown_handle = server_handle.clone();
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app.into_make_service())
            .with_graceful_shutdown(shutdown_handle.stopped())
            .await
        {
            tracing::warn!("mirage http server exited with error: {error}");
        }
    });
    Ok((local_addr, server_handle))
}

/// Starts an ephemeral server for tests.
///
/// # Errors
///
/// Returns the same errors as [`start_rpc_server`].
pub async fn spawn_rpc_server_for_tests() -> Result<(String, ServerHandle)> {
    let upstream = Arc::new(UpstreamRpc::mock(1));
    let db = HybridDB::new(upstream, 64, Duration::from_secs(12), NonZeroUsize::MIN, 1);
    let fork = ForkState::new(db, 0, 1);
    let mirage = MirageFork::new(
        fork,
        ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
        MirageMode::Live,
    );
    let (shutdown, _) = broadcast::channel(4);
    let (addr, handle) = start_rpc_server("127.0.0.1:0", mirage, shutdown).await?;
    Ok((format!("http://{addr}"), handle))
}

/// Maps an Ethereum JSON-RPC block selector to [`ForkState`]'s pinned upstream block for reads.
fn apply_eth_block_param(fork: &mut ForkState, block: &serde_json::Value) -> Result<()> {
    fork.db.pinned_block = match block {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => {
            let s = s.trim();
            if matches!(s, "latest" | "pending" | "finalized" | "safe") {
                None
            } else if s == "earliest" {
                Some(0)
            } else {
                Some(parse_hex_quantity(s)?)
            }
        }
        serde_json::Value::Number(n) => {
            let n = n.as_u64().ok_or_else(|| {
                MirageError::InvalidParams("block number does not fit in u64".to_owned())
            })?;
            Some(n)
        }
        _ => {
            return Err(MirageError::InvalidParams(
                "invalid block parameter".to_owned(),
            ));
        }
    };
    Ok(())
}

/// Resolves bytecode for `eth_getCode`: prefer embedded code, else [`HybridDB::code_by_hash`].
fn bytecode_for_eth_get_code(fork: &mut ForkState, address: Address) -> Result<Bytecode> {
    let info = fork.db.basic(address)?.unwrap_or_default();
    if let Some(code) = info.code {
        return Ok(code);
    }
    if info.code_hash.is_zero() {
        return Ok(Bytecode::default());
    }
    fork.db.code_by_hash(info.code_hash)
}

/// Stub `eth_feeHistory` payload with array lengths matching EIP-1559 client expectations.
fn build_fee_history_response(
    block_count_raw: serde_json::Value,
    _newest_block: serde_json::Value,
    reward_percentiles: Option<serde_json::Value>,
) -> Result<serde_json::Value> {
    let block_count = match block_count_raw {
        serde_json::Value::Number(n) => {
            let raw = n
                .as_u64()
                .ok_or_else(|| MirageError::InvalidParams("feeHistory blockCount".to_owned()))?;
            usize::try_from(raw.clamp(1, 1024))
                .map_err(|_| MirageError::InvalidParams("feeHistory blockCount".to_owned()))?
        }
        serde_json::Value::String(s) => {
            let raw = parse_hex_quantity(s.trim())?;
            usize::try_from(raw.clamp(1, 1024))
                .map_err(|_| MirageError::InvalidParams("feeHistory blockCount".to_owned()))?
        }
        _ => {
            return Err(MirageError::InvalidParams(
                "feeHistory blockCount must be a number or hex quantity".to_owned(),
            ));
        }
    };
    let reward_tiers = reward_percentiles
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|a| a.len().max(1))
        .unwrap_or(1);
    let base_fee_per_gas: Vec<String> = (0..=block_count).map(|_| "0x1".to_owned()).collect();
    let gas_used_ratio: Vec<f64> = std::iter::repeat_n(0.5, block_count).collect();
    let reward: Vec<Vec<String>> = std::iter::repeat_n(
        std::iter::repeat_n("0x0".to_owned(), reward_tiers).collect(),
        block_count,
    )
    .collect();
    Ok(serde_json::json!({
        "oldestBlock": "0x0",
        "baseFeePerGas": base_fee_per_gas,
        "gasUsedRatio": gas_used_ratio,
        "reward": reward,
    }))
}

fn build_rpc_module(
    context: ServerContext,
) -> std::result::Result<RpcModule<ServerContext>, RegisterMethodError> {
    let mut module = RpcModule::new(context);

    module.register_async_method("web3_clientVersion", |_params, _ctx, _| async {
        Ok::<_, ErrorObjectOwned>("mirage-rs/2.0.0".to_owned())
    })?;

    module.register_async_method("net_version", |_params, ctx, _| async move {
        let state = ctx.state.read();
        Ok::<_, ErrorObjectOwned>(state.fork.chain_id.to_string())
    })?;

    module.register_async_method("eth_chainId", |_params, ctx, _| async move {
        let state = ctx.state.read();
        Ok::<_, ErrorObjectOwned>(hex_u64(state.fork.chain_id))
    })?;

    module.register_async_method("eth_blockNumber", |_params, ctx, _| async move {
        let state = ctx.state.read();
        Ok::<_, ErrorObjectOwned>(hex_u64(state.fork.local_block_number))
    })?;

    module.register_async_method("eth_gasPrice", |_params, _ctx, _| async {
        Ok::<_, ErrorObjectOwned>("0x1".to_owned())
    })?;

    module.register_async_method("eth_maxPriorityFeePerGas", |_params, _ctx, _| async {
        Ok::<_, ErrorObjectOwned>("0x0".to_owned())
    })?;

    module.register_async_method("eth_feeHistory", |params, _ctx, _| async move {
        let args: Vec<serde_json::Value> = params.parse().map_err(invalid_params)?;
        if args.is_empty() {
            return Err(invalid_params_message("eth_feeHistory requires blockCount"));
        }
        let block_count = args[0].clone();
        let newest_block = args.get(1).cloned().unwrap_or(serde_json::json!("latest"));
        let reward_pct = args.get(2).cloned();
        let json =
            build_fee_history_response(block_count, newest_block, reward_pct).map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(json)
    })?;

    module.register_async_method("eth_getBalance", |params, ctx, _| async move {
        let (address, block): (Address, serde_json::Value) =
            params.parse().map_err(invalid_params)?;
        let balance = run_fork_snapshot(&ctx.state, true, move |mut fork| {
            apply_eth_block_param(&mut fork, &block)?;
            Ok(fork.db.basic(address)?.unwrap_or_default().balance)
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(hex_u256(balance))
    })?;

    module.register_async_method("eth_getTransactionCount", |params, ctx, _| async move {
        let (address, block): (Address, serde_json::Value) =
            params.parse().map_err(invalid_params)?;
        let nonce = run_fork_snapshot(&ctx.state, true, move |mut fork| {
            apply_eth_block_param(&mut fork, &block)?;
            Ok(fork.db.basic(address)?.unwrap_or_default().nonce)
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(hex_u64(nonce))
    })?;

    module.register_async_method("eth_getStorageAt", |params, ctx, _| async move {
        let (address, slot, block): (Address, U256, serde_json::Value) =
            params.parse().map_err(invalid_params)?;
        let value = run_fork_snapshot(&ctx.state, true, move |mut fork| {
            apply_eth_block_param(&mut fork, &block)?;
            fork.db.storage(address, slot)
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(format!("0x{:064x}", value))
    })?;

    module.register_async_method("eth_getCode", |params, ctx, _| async move {
        let (address, block): (Address, serde_json::Value) =
            params.parse().map_err(invalid_params)?;
        let code = run_fork_snapshot(&ctx.state, true, move |mut fork| {
            apply_eth_block_param(&mut fork, &block)?;
            bytecode_for_eth_get_code(&mut fork, address)
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(format!("0x{}", hex::encode(code.bytecode())))
    })?;

    module.register_async_method("eth_call", |params, ctx, _| async move {
        // Accept `[tx]` or `[tx, blockTag]`. `from` is optional (defaults to
        // zero-address) to match the JSON-RPC spec's view-call semantics.
        let (request, block): (TransactionRequest, serde_json::Value) = {
            let raw: serde_json::Value = params.parse().map_err(invalid_params)?;
            let arr = raw
                .as_array()
                .ok_or_else(|| invalid_params_message("eth_call: expected array"))?
                .clone();
            let first = arr.first().cloned().unwrap_or(serde_json::json!({}));
            let tx: TransactionRequest = serde_json::from_value(first).map_err(invalid_params)?;
            let blk = arr
                .get(1)
                .cloned()
                .unwrap_or_else(|| serde_json::json!("latest"));
            (tx, blk)
        };
        let from = request.from.unwrap_or(Address::ZERO);
        let to = extract_to(request.to).ok_or_else(|| invalid_params_message("missing to"))?;
        let data = request.data.unwrap_or_default();
        let value = request.value.unwrap_or(U256::ZERO);
        // Default to block gas limit for view calls; 21_000 is too low for
        // anything that touches storage.
        let gas = request.gas.unwrap_or(30_000_000);
        let result = run_fork_snapshot(&ctx.state, false, move |mut fork| {
            apply_eth_block_param(&mut fork, &block)?;
            EvmExecutor::call(&fork, from, to, data, value, gas)
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(format!("0x{}", hex::encode(&result.output)))
    })?;

    module.register_async_method("eth_estimateGas", |params, ctx, _| async move {
        // Accept either `[tx]` or `[tx, blockTag]` — some clients (alloy) always send
        // the block tag, others (cast) omit it. Block tag is ignored: estimation is
        // always against the pending tip in mirage.
        let request: TransactionRequest = {
            let raw: serde_json::Value = params.parse().map_err(invalid_params)?;
            match raw {
                serde_json::Value::Array(mut arr) if !arr.is_empty() => {
                    let first = arr.remove(0);
                    serde_json::from_value(first).map_err(invalid_params)?
                }
                serde_json::Value::Object(_) => {
                    serde_json::from_value(raw).map_err(invalid_params)?
                }
                _ => {
                    return Err(invalid_params(MirageError::InvalidParams(
                        "eth_estimateGas: expected [tx] or [tx, blockTag]".to_owned(),
                    )));
                }
            }
        };
        let state = Arc::clone(&ctx.state);
        let fork = { state.read().fork.clone() };
        let executor = Arc::clone(&state.read().speculative_executor);
        let result = tokio::task::spawn_blocking(move || {
            let mut exec = executor.lock();
            exec.execute(&fork, &request)
        })
        .await
        .map_err(|error| rpc_error(MirageError::BackgroundTask(error.to_string())))?
        .map_err(rpc_error)?;
        let estimate = result.state_diff.gas_used.saturating_mul(12) / 10;
        Ok::<_, ErrorObjectOwned>(hex_u64(estimate.max(21_000)))
    })?;

    module.register_async_method("eth_sendTransaction", |params, ctx, _| async move {
        let (request,): (TransactionRequest,) = params.parse().map_err(invalid_params)?;
        let tx_hash = commit_transaction_request(&ctx.state, request, None)
            .await
            .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(tx_hash)
    })?;

    module.register_async_method("eth_sendRawTransaction", |params, ctx, _| async move {
        let (raw,): (Bytes,) = params.parse().map_err(invalid_params)?;
        let decoded = decode_signed_raw_transaction(&raw).map_err(rpc_error)?;
        let tx_hash =
            commit_transaction_request(&ctx.state, decoded.request, Some(decoded.tx_hash))
                .await
                .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(tx_hash)
    })?;

    module.register_async_method("eth_getTransactionReceipt", |params, ctx, _| async move {
        let (tx_hash,): (B256,) = params.parse().map_err(invalid_params)?;
        let local = {
            let state = ctx.state.read();
            state.fork.receipts.get(&tx_hash).map(receipt_json)
        };
        if let Some(receipt) = local {
            return Ok::<_, ErrorObjectOwned>(Some(receipt));
        }
        // Fall back to upstream when forking.
        let upstream_receipt = run_fork_snapshot(&ctx.state, false, move |fork| {
            fork.db.upstream.get_transaction_receipt(tx_hash)
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(upstream_receipt)
    })?;

    module.register_async_method("eth_getTransactionByHash", |params, ctx, _| async move {
        let (tx_hash,): (B256,) = params.parse().map_err(invalid_params)?;
        let local = {
            let state = ctx.state.read();
            state.fork.transactions.get(&tx_hash).map(transaction_json)
        };
        if let Some(tx) = local {
            return Ok::<_, ErrorObjectOwned>(Some(tx));
        }
        // Fall back to upstream when forking.
        let upstream_tx = run_fork_snapshot(&ctx.state, false, move |fork| {
            fork.db.upstream.get_transaction_by_hash(tx_hash)
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(upstream_tx)
    })?;

    module.register_async_method("eth_getLogs", |params, ctx, _| async move {
        // Accept a single `{ fromBlock?, toBlock?, address?, topics? }` filter
        // object. Range bounds accept block tags (`latest`, `earliest`,
        // `pending`) or hex numbers.
        let (filter,): (serde_json::Value,) = params.parse().map_err(invalid_params)?;
        let state = ctx.state.read();
        let tip = state.fork.local_block_number;
        let (from, to) = {
            let fblock = filter.get("fromBlock");
            let tblock = filter.get("toBlock");
            let from = resolve_block_tag(fblock, tip).unwrap_or(0);
            let to = resolve_block_tag(tblock, tip).unwrap_or(tip);
            (from.min(to), to.max(from))
        };
        // Filter: address(es) + topic0. We only match topic0 (event signature)
        // for now — it covers 99% of client use and keeps the scan O(logs).
        let addresses: Vec<String> = match filter.get("address") {
            Some(serde_json::Value::String(s)) => vec![s.to_ascii_lowercase()],
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(str::to_ascii_lowercase))
                .collect(),
            _ => Vec::new(),
        };
        let topic0_filter: Vec<String> = match filter.get("topics") {
            Some(serde_json::Value::Array(arr)) if !arr.is_empty() => match &arr[0] {
                serde_json::Value::String(s) => vec![s.to_ascii_lowercase()],
                serde_json::Value::Array(sub) => sub
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_ascii_lowercase))
                    .collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        let mut out: Vec<serde_json::Value> = Vec::new();
        for (num, block) in state.fork.blocks_by_number.range(from..=to) {
            for tx_hash in &block.transactions {
                let Some(receipt) = state.fork.receipts.get(tx_hash) else {
                    continue;
                };
                let addr_lower = format!("{:#x}", receipt.from).to_ascii_lowercase();
                let _ = addr_lower;
                for log in &receipt.logs {
                    let log_addr = format!("{:#x}", log.address).to_ascii_lowercase();
                    if !addresses.is_empty() && !addresses.contains(&log_addr) {
                        continue;
                    }
                    if !topic0_filter.is_empty() {
                        let Some(t0) = log.topics.first() else {
                            continue;
                        };
                        let t0_lower = format!("{:#x}", t0).to_ascii_lowercase();
                        if !topic0_filter.contains(&t0_lower) {
                            continue;
                        }
                    }
                    out.push(serde_json::json!({
                        "address": log.address,
                        "topics": log.topics,
                        "data": format!("0x{}", hex::encode(log.data.as_ref())),
                        "blockNumber": hex_u64(*num),
                        "blockHash": block.hash,
                        "transactionHash": tx_hash,
                        "transactionIndex": "0x0",
                        "logIndex": hex_u64(log.log_index as u64),
                        "removed": false,
                    }));
                }
            }
        }
        Ok::<_, ErrorObjectOwned>(out)
    })?;

    module.register_async_method("eth_getBlockByNumber", |params, ctx, _| async move {
        let (number, _full): (String, bool) = params.parse().map_err(invalid_params)?;
        let full_transactions = _full;
        let local_block = {
            let state = ctx.state.read();
            if number == "latest" {
                state
                    .fork
                    .blocks_by_number
                    .get(&state.fork.local_block_number)
                    .map(block_json)
            } else {
                parse_hex_quantity(&number)
                    .ok()
                    .and_then(|number| state.fork.blocks_by_number.get(&number))
                    .map(block_json)
            }
        };
        if let Some(block) = local_block {
            return Ok::<_, ErrorObjectOwned>(Some(block));
        }

        let block_tag = if number == "latest" {
            BlockTag::Latest
        } else {
            BlockTag::Number(parse_hex_quantity(&number).map_err(invalid_params)?)
        };
        let upstream_block = run_fork_snapshot(&ctx.state, false, move |fork| {
            fork.db
                .upstream
                .get_block_by_number(block_tag, full_transactions)
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(upstream_block)
    })?;

    module.register_async_method("eth_getBlockByHash", |params, ctx, _| async move {
        let (hash, full): (B256, bool) = params.parse().map_err(invalid_params)?;
        let local = {
            let state = ctx.state.read();
            state.fork.blocks_by_hash.get(&hash).map(block_json)
        };
        if let Some(block) = local {
            return Ok::<_, ErrorObjectOwned>(Some(block));
        }
        // Fall back to upstream when forking.
        let upstream_block = run_fork_snapshot(&ctx.state, false, move |fork| {
            fork.db.upstream.get_block_by_hash(hash, full)
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(upstream_block)
    })?;

    // ── Standard / Anvil compatibility methods ──────────────────────────

    module.register_async_method("eth_accounts", |_params, ctx, _| async move {
        let state = ctx.state.read();
        let accounts: Vec<Address> = state.fork.impersonated_accounts.iter().copied().collect();
        Ok::<_, ErrorObjectOwned>(accounts)
    })?;

    module.register_async_method("web3_sha3", |params, ctx, _| async move {
        let _ = ctx;
        let (input,): (Bytes,) = params.parse().map_err(invalid_params)?;
        Ok::<_, ErrorObjectOwned>(keccak256(input.as_ref()))
    })?;

    module.register_async_method("net_listening", |_params, _ctx, _| async move {
        Ok::<_, ErrorObjectOwned>(true)
    })?;

    module.register_async_method("net_peerCount", |_params, _ctx, _| async move {
        Ok::<_, ErrorObjectOwned>("0x0")
    })?;

    module.register_async_method("txpool_status", |_params, _ctx, _| async move {
        Ok::<_, ErrorObjectOwned>(serde_json::json!({
            "pending": "0x0",
            "queued": "0x0",
        }))
    })?;

    module.register_async_method("txpool_content", |_params, _ctx, _| async move {
        Ok::<_, ErrorObjectOwned>(serde_json::json!({
            "pending": {},
            "queued": {},
        }))
    })?;

    module.register_async_method("txpool_inspect", |_params, _ctx, _| async move {
        Ok::<_, ErrorObjectOwned>(serde_json::json!({
            "pending": {},
            "queued": {},
        }))
    })?;

    module.register_async_method("anvil_getAutomine", |_params, _ctx, _| async move {
        Ok::<_, ErrorObjectOwned>(true)
    })?;

    module.register_async_method("anvil_nodeInfo", |_params, ctx, _| async move {
        let state = ctx.state.read();
        Ok::<_, ErrorObjectOwned>(serde_json::json!({
            "chainId": state.fork.chain_id,
            "forkUrl": state.fork.fork_url,
            "forkBlock": state.fork.fork_block,
            "currentBlockNumber": hex_u64(state.fork.local_block_number),
            "currentBlockTimestamp": hex_u64(state.fork.timestamp),
            "gasLimit": "0x1c9c380",
            "baseFeePerGas": format!("0x{:x}", state.fork.next_base_fee_per_gas),
            "automine": true,
        }))
    })?;

    module.register_async_method("anvil_dropTransaction", |params, ctx, _| async move {
        let (tx_hash,): (B256,) = params.parse().map_err(invalid_params)?;
        let mut state = ctx.state.write();
        let removed = state.fork.transactions.remove(&tx_hash).is_some();
        if removed {
            state.fork.receipts.remove(&tx_hash);
        }
        Ok::<_, ErrorObjectOwned>(removed)
    })?;

    module.register_async_method("anvil_setRpcUrl", |params, ctx, _| async move {
        let (url,): (String,) = params.parse().map_err(invalid_params)?;
        let mut state = ctx.state.write();
        state.fork.fork_url = Some(url);
        Ok::<_, ErrorObjectOwned>(true)
    })?;

    module.register_async_method("anvil_dumpState", |_params, ctx, _| async move {
        let state = ctx.state.read();
        let dirty_json = serde_json::to_vec(&state.fork.db.dirty)
            .map_err(|e| rpc_error(MirageError::Unsupported(e.to_string())))?;
        Ok::<_, ErrorObjectOwned>(format!("0x{}", hex::encode(&dirty_json)))
    })?;

    module.register_async_method("anvil_loadState", |params, ctx, _| async move {
        let (hex_data,): (String,) = params.parse().map_err(invalid_params)?;
        let bytes = hex::decode(hex_data.trim_start_matches("0x"))
            .map_err(|e| invalid_params_message(format!("invalid hex: {e}")))?;
        let dirty: crate::fork::DirtyStore = serde_json::from_slice(&bytes)
            .map_err(|e| invalid_params_message(format!("invalid state JSON: {e}")))?;
        let mut state = ctx.state.write();
        state.fork.db.dirty = dirty;
        Ok::<_, ErrorObjectOwned>(true)
    })?;

    module.register_async_method("debug_traceTransaction", |params, ctx, _| async move {
        let (tx_hash,): (B256,) = params.parse().map_err(invalid_params)?;
        let state = ctx.state.read();
        let gas = state
            .fork
            .receipts
            .get(&tx_hash)
            .map(|r| r.gas_used)
            .unwrap_or(0);
        // Return a minimal trace stub — full step-level tracing is not yet supported.
        Ok::<_, ErrorObjectOwned>(serde_json::json!({
            "gas": gas,
            "failed": false,
            "returnValue": "",
            "structLogs": [],
        }))
    })?;

    // ── Filter API ───────────────────────────────────────────────────────

    module.register_async_method("eth_newFilter", |params, ctx, _| async move {
        let (filter,): (serde_json::Value,) = params.parse().map_err(invalid_params)?;
        let addresses: Vec<Address> = match filter.get("address") {
            Some(serde_json::Value::String(s)) => {
                vec![
                    s.parse::<Address>()
                        .map_err(|e| invalid_params_message(e.to_string()))?,
                ]
            }
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().and_then(|s| s.parse::<Address>().ok()))
                .collect(),
            _ => Vec::new(),
        };
        let topics: Vec<Option<Vec<B256>>> = match filter.get("topics") {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .map(|t| match t {
                    serde_json::Value::Null => None,
                    serde_json::Value::String(s) => s.parse::<B256>().ok().map(|h| vec![h]),
                    serde_json::Value::Array(sub) => Some(
                        sub.iter()
                            .filter_map(|v| v.as_str().and_then(|s| s.parse::<B256>().ok()))
                            .collect(),
                    ),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let mut state = ctx.state.write();
        let tip = state.fork.local_block_number;
        let id = state.fork.next_filter_id;
        state.fork.next_filter_id += U256::from(1);
        state.fork.filters.insert(
            id,
            EthFilter::Log {
                topics,
                addresses,
                last_poll_block: tip,
            },
        );
        Ok::<_, ErrorObjectOwned>(hex_u256(id))
    })?;

    module.register_async_method("eth_newBlockFilter", |_params, ctx, _| async move {
        let mut state = ctx.state.write();
        let tip = state.fork.local_block_number;
        let id = state.fork.next_filter_id;
        state.fork.next_filter_id += U256::from(1);
        state.fork.filters.insert(
            id,
            EthFilter::Block {
                last_poll_block: tip,
            },
        );
        Ok::<_, ErrorObjectOwned>(hex_u256(id))
    })?;

    module.register_async_method(
        "eth_newPendingTransactionFilter",
        |_params, ctx, _| async move {
            let mut state = ctx.state.write();
            let id = state.fork.next_filter_id;
            state.fork.next_filter_id += U256::from(1);
            state.fork.filters.insert(
                id,
                EthFilter::PendingTransaction {
                    seen: std::collections::HashSet::new(),
                },
            );
            Ok::<_, ErrorObjectOwned>(hex_u256(id))
        },
    )?;

    module.register_async_method("eth_getFilterChanges", |params, ctx, _| async move {
        let (id_hex,): (String,) = params.parse().map_err(invalid_params)?;
        let id = parse_hex_u256(&id_hex).map_err(rpc_error)?;
        let mut state = ctx.state.write();
        let tip = state.fork.local_block_number;
        let Some(filter) = state.fork.filters.get_mut(&id) else {
            return Err(invalid_params_message("filter not found"));
        };
        // Extract filter state and advance cursor under the mutable borrow,
        // then drop the mutable ref so we can read blocks/receipts.
        enum PollKind {
            Block {
                from: u64,
            },
            Pending,
            Log {
                from: u64,
                topics: Vec<Option<Vec<B256>>>,
                addresses: Vec<Address>,
            },
        }
        let kind = match filter {
            EthFilter::Block { last_poll_block } => {
                let from = *last_poll_block + 1;
                *last_poll_block = tip;
                PollKind::Block { from }
            }
            EthFilter::PendingTransaction { .. } => PollKind::Pending,
            EthFilter::Log {
                topics,
                addresses,
                last_poll_block,
            } => {
                let from = *last_poll_block + 1;
                *last_poll_block = tip;
                PollKind::Log {
                    from,
                    topics: topics.clone(),
                    addresses: addresses.clone(),
                }
            }
        };
        // Now only immutable reads on state.fork.
        match kind {
            PollKind::Block { from } => {
                let hashes: Vec<B256> = (from..=tip)
                    .filter_map(|n| state.fork.blocks_by_number.get(&n).map(|b| b.hash))
                    .collect();
                Ok(serde_json::json!(hashes))
            }
            PollKind::Pending => {
                let all_tx_hashes: Vec<B256> = state.fork.transactions.keys().copied().collect();
                // Re-acquire mutable ref to update the seen set.
                if let Some(EthFilter::PendingTransaction { seen }) =
                    state.fork.filters.get_mut(&id)
                {
                    let new_hashes: Vec<B256> = all_tx_hashes
                        .into_iter()
                        .filter(|h| seen.insert(*h))
                        .collect();
                    Ok(serde_json::json!(new_hashes))
                } else {
                    Ok(serde_json::json!([]))
                }
            }
            PollKind::Log {
                from,
                topics,
                addresses,
            } => {
                let out = collect_filter_logs(&state.fork, from, tip, &topics, &addresses);
                Ok(serde_json::json!(out))
            }
        }
    })?;

    module.register_async_method("eth_getFilterLogs", |params, ctx, _| async move {
        let (id_hex,): (String,) = params.parse().map_err(invalid_params)?;
        let id = parse_hex_u256(&id_hex).map_err(rpc_error)?;
        let state = ctx.state.read();
        let tip = state.fork.local_block_number;
        let Some(filter) = state.fork.filters.get(&id) else {
            return Err(invalid_params_message("filter not found"));
        };
        let EthFilter::Log {
            topics, addresses, ..
        } = filter
        else {
            return Err(invalid_params_message(
                "eth_getFilterLogs only works with log filters",
            ));
        };
        let out = collect_filter_logs(&state.fork, 0, tip, topics, addresses);
        Ok::<_, ErrorObjectOwned>(out)
    })?;

    module.register_async_method("eth_uninstallFilter", |params, ctx, _| async move {
        let (id_hex,): (String,) = params.parse().map_err(invalid_params)?;
        let id = parse_hex_u256(&id_hex).map_err(rpc_error)?;
        let mut state = ctx.state.write();
        Ok::<_, ErrorObjectOwned>(state.fork.filters.remove(&id).is_some())
    })?;

    register_impersonation_methods(&mut module)?;
    register_state_mutation_methods(&mut module)?;
    register_snapshot_methods(&mut module)?;
    register_mirage_methods(&mut module)?;
    #[cfg(feature = "chain")]
    register_chain_methods(&mut module)?;

    Ok(module)
}

#[cfg(feature = "chain")]
fn register_chain_methods(
    module: &mut RpcModule<ServerContext>,
) -> std::result::Result<(), RegisterMethodError> {
    use crate::chain_rpc::{
        ChallengeInsightParams, ConfirmInsightParams, DepositPheromoneParams, PostInsightParams,
        SearchInsightsParams, handle_apply_decay, handle_challenge_insight, handle_confirm_insight,
        handle_deposit_pheromone, handle_get_insight, handle_list_kinds, handle_method_schema,
        handle_post_insight, handle_query_pheromones, handle_search_insights, handle_stats,
        handle_version,
    };

    fn require_chain(
        ctx: &ServerContext,
    ) -> std::result::Result<Arc<RwLock<crate::chain_rpc::ChainContext>>, ErrorObjectOwned> {
        ctx.chain.clone().ok_or_else(|| {
            ErrorObjectOwned::owned::<()>(
                crate::chain_rpc::err_code::DISABLED,
                "chain subsystem not attached to this server",
                None,
            )
        })
    }

    module.register_async_method("chain_postInsight", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: PostInsightParams = params.parse().map_err(|e| {
            invalid_params_message(format!(
                "{e}. Expected: {{author: string, kind: string, content: string, enabledBy?: string[], stakeWei?: number}}"
            ))
        })?;
        let result = handle_post_insight(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_searchInsights", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: SearchInsightsParams = params.parse().map_err(|e| {
            invalid_params_message(format!(
                "{e}. Expected: {{query?: string, queryVector?: number[], k?: number, kind?: string}}"
            ))
        })?;
        let result = handle_search_insights(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_confirmInsight", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: ConfirmInsightParams = params.parse().map_err(|e| {
            invalid_params_message(format!("{e}. Expected: {{id: string, confirmer: string}}"))
        })?;
        let result = handle_confirm_insight(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_challengeInsight", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: ChallengeInsightParams = params.parse().map_err(|e| {
            invalid_params_message(format!("{e}. Expected: {{id: string, challenger: string}}"))
        })?;
        let result = handle_challenge_insight(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_getInsight", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: serde_json::Value = params.parse().map_err(invalid_params)?;
        let result = handle_get_insight(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_applyDecay", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: serde_json::Value = params.parse().map_err(invalid_params)?;
        let result = handle_apply_decay(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_depositPheromone", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: DepositPheromoneParams = params.parse().map_err(|e| {
            invalid_params_message(format!(
                "{e}. Expected: {{kind: string, content: string, intensity?: number, halfLifeSeconds?: number}}"
            ))
        })?;
        let result = handle_deposit_pheromone(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_queryPheromones", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: serde_json::Value = params.parse().map_err(invalid_params)?;
        let result = handle_query_pheromones(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_stats", |_params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        Ok::<_, ErrorObjectOwned>(handle_stats(&chain))
    })?;

    module.register_async_method("chain_version", |_params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        Ok::<_, ErrorObjectOwned>(handle_version(&chain))
    })?;

    module.register_async_method("chain_listKinds", |_params, ctx, _| async move {
        let _chain = require_chain(&ctx)?;
        Ok::<_, ErrorObjectOwned>(handle_list_kinds())
    })?;

    module.register_async_method("chain_methodSchema", |params, ctx, _| async move {
        let _chain = require_chain(&ctx)?;
        let payload: serde_json::Value = params.parse().map_err(invalid_params)?;
        let result = handle_method_schema(payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    // Agent registry RPC methods
    module.register_async_method("chain_registerAgent", |params, ctx, _| async move {
        let (id, address_hex, role): (String, String, String) =
            params.parse().map_err(invalid_params)?;
        let chain = require_chain(&ctx)?;
        crate::chain_rpc::handle_register_agent(&chain, id, address_hex, role)
    })?;

    module.register_async_method("chain_agentHeartbeat", |params, ctx, _| async move {
        let (id,): (String,) = params.parse().map_err(invalid_params)?;
        let chain = require_chain(&ctx)?;
        let current_block = ctx.state.read().fork.local_block_number;
        Ok::<_, ErrorObjectOwned>(crate::chain_rpc::handle_agent_heartbeat(
            &chain,
            id,
            current_block,
        ))
    })?;

    module.register_async_method("chain_agentTrace", |params, ctx, _| async move {
        let (id, phase, reads, reasoning, action): (String, String, Vec<String>, String, String) =
            params.parse().map_err(invalid_params)?;
        let chain = require_chain(&ctx)?;
        crate::chain_rpc::handle_agent_trace(&chain, id, phase, reads, reasoning, action)
    })?;

    module.register_async_method("chain_agentStats", |params, ctx, _| async move {
        let (id, delta): (String, crate::chain::AgentStats) =
            params.parse().map_err(invalid_params)?;
        let chain = require_chain(&ctx)?;
        Ok::<_, ErrorObjectOwned>(crate::chain_rpc::handle_agent_stats(&chain, id, delta))
    })?;

    #[cfg(feature = "roko")]
    register_chain_subscription_methods(module)?;

    Ok(())
}

/// §38.d — registers `chain_subscribePheromones`, `chain_subscribeInsights`,
/// and `chain_unsubscribe` on the shared RPC module.
///
/// **Subscription transport decision**: we use jsonrpsee's native
/// `register_subscription` machinery (same pattern as `eth_subscribe` in this
/// file, see line ~1100). Each subscription:
///
/// 1. Accepts the incoming WS upgrade via `pending.accept().await`.
/// 2. Registers an [`MpscSink`](crate::roko_bridge::MpscSink) with the
///    corresponding [`SubscriptionManager`] bus so the write-path handlers
///    (`handle_deposit_pheromone`, `handle_post_insight`, etc.) broadcast
///    through it.
/// 3. Bridges the mpsc receiver into jsonrpsee's `SubscriptionSink` in a
///    `tokio::select!` loop that exits when either side closes.
/// 4. Unregisters the bus subscription on loop exit so stats stay accurate.
///
/// The `chain_unsubscribe` method is registered as a regular async method
/// (not as the "unsubscribe" hook tied to jsonrpsee's subscription lifecycle)
/// because our external id format (`pher:N` / `insi:N`) is namespaced across
/// both buses. Jsonrpsee's own per-connection unsubscribe still fires on WS
/// disconnect via the `is_closed()` check in the select loop.
#[cfg(feature = "roko")]
fn register_chain_subscription_methods(
    module: &mut RpcModule<ServerContext>,
) -> std::result::Result<(), RegisterMethodError> {
    use crate::chain_rpc::{
        INSIGHT_SUB_PREFIX, PHEROMONE_SUB_PREFIX, handle_unsubscribe, insight_event_to_json,
        pheromone_event_to_json,
    };
    use crate::roko_bridge::{BackpressurePolicy, InsightEvent, MpscSink, PheromoneEvent};

    fn require_subs(
        ctx: &ServerContext,
    ) -> std::result::Result<crate::chain_rpc::SubscriptionManager, ErrorObjectOwned> {
        ctx.chain_subs.clone().ok_or_else(|| {
            ErrorObjectOwned::owned::<()>(
                crate::chain_rpc::err_code::DISABLED,
                "chain subscriptions not attached to this server",
                None,
            )
        })
    }

    module.register_subscription::<std::result::Result<(), SubscriptionError>, _, _>(
        "chain_subscribePheromones",
        "chain_pheromoneEvent",
        "chain_unsubscribePheromones",
        |_params, pending, ctx, _| async move {
            let manager = match require_subs(&ctx) {
                Ok(m) => m,
                Err(e) => {
                    pending.reject(e).await;
                    return Ok(());
                }
            };
            let (mpsc_sink, mut rx) = MpscSink::<PheromoneEvent>::new(128);
            let bus_id = manager
                .pheromones()
                .register(Arc::new(mpsc_sink), BackpressurePolicy::DropNewest);
            let external_id = format!("{}{}", PHEROMONE_SUB_PREFIX, bus_id.0);
            let sink = match pending.accept().await {
                Ok(s) => s,
                Err(_) => {
                    manager.pheromones().unregister(bus_id);
                    return Ok(());
                }
            };

            // First message: tell the client its external id so it can call
            // chain_unsubscribe(id) later.
            if let Ok(handshake) = serde_json::value::to_raw_value(
                &serde_json::json!({"subscriptionId": external_id}),
            ) {
                let _ = sink.send(handshake).await;
            }

            loop {
                tokio::select! {
                    _ = sink.closed() => break,
                    msg = rx.recv() => {
                        match msg {
                            Some(event) => {
                                let payload = pheromone_event_to_json(&event);
                                let Ok(raw) = serde_json::value::to_raw_value(&payload) else { break };
                                if sink.send(raw).await.is_err() { break; }
                            }
                            None => break,
                        }
                    }
                }
            }

            manager.pheromones().unregister(bus_id);
            Ok(())
        },
    )?;

    module.register_subscription::<std::result::Result<(), SubscriptionError>, _, _>(
        "chain_subscribeInsights",
        "chain_insightEvent",
        "chain_unsubscribeInsights",
        |_params, pending, ctx, _| async move {
            let manager = match require_subs(&ctx) {
                Ok(m) => m,
                Err(e) => {
                    pending.reject(e).await;
                    return Ok(());
                }
            };
            let (mpsc_sink, mut rx) = MpscSink::<InsightEvent>::new(128);
            let bus_id = manager
                .insights()
                .register(Arc::new(mpsc_sink), BackpressurePolicy::DropNewest);
            let external_id = format!("{}{}", INSIGHT_SUB_PREFIX, bus_id.0);
            let sink = match pending.accept().await {
                Ok(s) => s,
                Err(_) => {
                    manager.insights().unregister(bus_id);
                    return Ok(());
                }
            };

            if let Ok(handshake) = serde_json::value::to_raw_value(
                &serde_json::json!({"subscriptionId": external_id}),
            ) {
                let _ = sink.send(handshake).await;
            }

            loop {
                tokio::select! {
                    _ = sink.closed() => break,
                    msg = rx.recv() => {
                        match msg {
                            Some(event) => {
                                let payload = insight_event_to_json(&event);
                                let Ok(raw) = serde_json::value::to_raw_value(&payload) else { break };
                                if sink.send(raw).await.is_err() { break; }
                            }
                            None => break,
                        }
                    }
                }
            }

            manager.insights().unregister(bus_id);
            Ok(())
        },
    )?;

    module.register_async_method("chain_unsubscribe", |params, ctx, _| async move {
        let manager = require_subs(&ctx)?;
        let payload: serde_json::Value = params.parse().map_err(invalid_params)?;
        let result = handle_unsubscribe(&manager, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    Ok(())
}

/// Full fork reset for `hardhat_reset` / `anvil_reset`: dirty store + read cache + watch lists,
/// local tx indexes, and impersonation set (mirrors Hardhat/Anvil semantics).
fn apply_hardhat_anvil_reset(state: &mut MirageState) -> Result<()> {
    state.fork.db.reset();
    state.fork.db.pinned_block = None;
    state.fork.db.dirty.demote_protocols_to_slot_only = false;
    state.fork.impersonated_accounts.clear();
    state.fork.receipts.clear();
    state.fork.transactions.clear();
    state.fork.blocks_by_hash.clear();
    state.fork.blocks_by_number.clear();
    state.last_committed_state_diff = None;
    Ok(())
}

/// Parses `hardhat_mine` / `anvil_mine` first parameter: hex quantity string or JSON integer.
fn parse_mine_block_count(values: &[serde_json::Value]) -> u64 {
    let Some(first) = values.first() else {
        return 1;
    };
    match first {
        serde_json::Value::String(text) => parse_hex_quantity(text.trim()).unwrap_or(1).max(1),
        serde_json::Value::Number(n) => n.as_u64().unwrap_or(1).max(1),
        _ => 1,
    }
}

fn register_impersonation_methods(
    module: &mut RpcModule<ServerContext>,
) -> std::result::Result<(), RegisterMethodError> {
    for method in ["hardhat_impersonateAccount", "anvil_impersonateAccount"] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (address,): (Address,) = params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                state.fork.impersonated_accounts.insert(address);
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in [
        "hardhat_stopImpersonatingAccount",
        "anvil_stopImpersonatingAccount",
    ] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (address,): (Address,) = params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                state.fork.impersonated_accounts.remove(&address);
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    Ok(())
}

fn register_state_mutation_methods(
    module: &mut RpcModule<ServerContext>,
) -> std::result::Result<(), RegisterMethodError> {
    for method in [
        "hardhat_setBalance",
        "anvil_setBalance",
        "mirage_setBalance",
    ] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (address, balance): (Address, U256) = params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                state.fork.db.set_balance(address, balance);
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in [
        "hardhat_setStorageAt",
        "anvil_setStorageAt",
        "mirage_setStorageAt",
    ] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (address, slot, value): (Address, U256, U256) =
                params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                state.fork.db.set_storage(address, slot, value);
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in ["hardhat_setCode", "anvil_setCode", "mirage_setCode"] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (address, code): (Address, Bytes) = params.parse().map_err(invalid_params)?;
            let bytecode = Bytecode::new_raw(code);
            with_state_write(&ctx.state, |state| {
                state.fork.db.set_code(address, bytecode);
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in ["hardhat_setNonce", "anvil_setNonce"] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (address, raw): (Address, serde_json::Value) =
                params.parse().map_err(invalid_params)?;
            let nonce = match &raw {
                serde_json::Value::Number(n) => n.as_u64().ok_or_else(|| {
                    invalid_params(MirageError::InvalidParams("nonce out of u64 range".into()))
                })?,
                serde_json::Value::String(s) => parse_hex_quantity(s).map_err(invalid_params)?,
                _ => {
                    return Err(invalid_params(MirageError::InvalidParams(
                        "nonce must be a number or hex string".into(),
                    )));
                }
            };
            with_state_write(&ctx.state, |state| {
                state.fork.db.set_nonce(address, nonce);
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in ["hardhat_mine", "anvil_mine", "evm_mine"] {
        module.register_async_method(method, |params, ctx, _| async move {
            let count = params
                .parse::<Vec<serde_json::Value>>()
                .map(|values| parse_mine_block_count(&values))
                .unwrap_or(1);
            with_state_write(&ctx.state, |state| {
                for _ in 0..count {
                    state.fork.local_block_number = state.fork.local_block_number.saturating_add(1);
                    state.fork.timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let block_hash = keccak256(state.fork.local_block_number.to_le_bytes());
                    let block = LocalBlock {
                        hash: block_hash,
                        number: state.fork.local_block_number,
                        timestamp: state.fork.timestamp,
                        gas_used: 0,
                        gas_limit: 30_000_000,
                        base_fee_per_gas: state.fork.next_base_fee_per_gas,
                        coinbase: state.fork.coinbase,
                        prev_randao: state.fork.prev_randao,
                        transactions: Vec::new(),
                    };
                    state.fork.blocks_by_hash.insert(block_hash, block.clone());
                    state
                        .fork
                        .blocks_by_number
                        .insert(state.fork.local_block_number, block);
                }
                state.fork.prune_old_blocks();
                let _ = state.new_heads_tx.send(crate::fork::NewHeadBroadcast {
                    number: state.fork.local_block_number,
                    timestamp: state.fork.timestamp,
                    gas_used: 0,
                    gas_limit: 30_000_000,
                    base_fee_per_gas: state.fork.next_base_fee_per_gas,
                    coinbase: state.fork.coinbase,
                    prev_randao: state.fork.prev_randao,
                });
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in ["hardhat_reset", "anvil_reset"] {
        module.register_async_method(method, |_params, ctx, _| async move {
            with_state_write(&ctx.state, |state| {
                apply_hardhat_anvil_reset(state)?;
                ensure_erc8004_boot_contracts(&mut state.fork)?;
                Ok::<(), MirageError>(())
            })
            .await
            .map_err(rpc_error)?;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in [
        "hardhat_setNextBlockBaseFeePerGas",
        "anvil_setNextBlockBaseFeePerGas",
    ] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (value,): (u128,) = params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                state.fork.next_base_fee_per_gas = value;
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in ["hardhat_setCoinbase", "anvil_setCoinbase"] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (value,): (Address,) = params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                state.fork.coinbase = value;
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in ["hardhat_setPrevRandao", "anvil_setPrevRandao"] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (value,): (B256,) = params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                state.fork.prev_randao = value;
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    // tevm_setAccount — TEVM's combined account mutation used when forking from a custom RPC.
    // Params: { address, nonce?, balance?, deployedBytecode?, state? }
    module.register_async_method("tevm_setAccount", |params, ctx, _| async move {
        let obj: serde_json::Value = params.parse().map_err(invalid_params)?;
        // params arrive as a single-element array containing the object
        let obj = if obj.is_array() {
            obj.as_array()
                .and_then(|a| a.first())
                .cloned()
                .unwrap_or(obj)
        } else {
            obj
        };
        let address: Address = obj
            .get("address")
            .and_then(|v| v.as_str())
            .ok_or_else(|| invalid_params("missing address"))?
            .parse()
            .map_err(|_| invalid_params("invalid address"))?;
        with_state_write(&ctx.state, |state| {
            if let Some(balance_val) = obj.get("balance").and_then(|v| v.as_str()) {
                if let Ok(balance) = U256::from_str_radix(balance_val.trim_start_matches("0x"), 16)
                {
                    state.fork.db.set_balance(address, balance);
                }
            }
            if let Some(nonce_val) = obj.get("nonce").and_then(|v| v.as_str()) {
                if let Ok(nonce) = u64::from_str_radix(nonce_val.trim_start_matches("0x"), 16) {
                    state.fork.db.set_nonce(address, nonce);
                }
            }
            if let Some(code_val) = obj.get("deployedBytecode").and_then(|v| v.as_str()) {
                if let Ok(bytes) = hex::decode(code_val.trim_start_matches("0x")) {
                    state
                        .fork
                        .db
                        .set_code(address, Bytecode::new_raw(Bytes::from(bytes)));
                }
            }
            // state: { "0xslot": "0xvalue", ... }
            if let Some(storage) = obj.get("state").and_then(|v| v.as_object()) {
                for (slot_hex, val) in storage {
                    if let (Ok(slot), Some(value)) = (
                        U256::from_str_radix(slot_hex.trim_start_matches("0x"), 16),
                        val.as_str().and_then(|s| {
                            U256::from_str_radix(s.trim_start_matches("0x"), 16).ok()
                        }),
                    ) {
                        state.fork.db.set_storage(address, slot, value);
                    }
                }
            }
        })
        .await;
        Ok::<_, ErrorObjectOwned>(serde_json::json!({ "errors": [] }))
    })?;
    Ok(())
}

fn register_snapshot_methods(
    module: &mut RpcModule<ServerContext>,
) -> std::result::Result<(), RegisterMethodError> {
    module.register_async_method("evm_snapshot", |_params, ctx, _| async move {
        let snapshot = with_state_write(&ctx.state, |state| hex_u64(state.fork.snapshot())).await;
        Ok::<_, ErrorObjectOwned>(snapshot)
    })?;
    module.register_async_method("evm_revert", |params, ctx, _| async move {
        let (snapshot_id,): (String,) = params.parse().map_err(invalid_params)?;
        let id = parse_hex_quantity(&snapshot_id).map_err(invalid_params)?;
        with_state_write(&ctx.state, |state| state.fork.revert(id).map_err(rpc_error)).await
    })?;
    module.register_async_method("evm_increaseTime", |params, ctx, _| async move {
        let (seconds,): (u64,) = params.parse().map_err(invalid_params)?;
        with_state_write(&ctx.state, |state| {
            state.fork.timestamp = state.fork.timestamp.saturating_add(seconds);
        })
        .await;
        Ok::<_, ErrorObjectOwned>(hex_u64(seconds))
    })?;
    module.register_async_method("evm_setNextBlockTimestamp", |params, ctx, _| async move {
        let (timestamp,): (u64,) = params.parse().map_err(invalid_params)?;
        with_state_write(&ctx.state, |state| {
            state.fork.timestamp = timestamp;
        })
        .await;
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    // Anvil/Hardhat mining-mode stubs. mirage-rs automines every block so
    // these are accepted for compatibility but are effectively no-ops.
    module.register_async_method("evm_setAutomine", |params, _ctx, _| async move {
        let (_enabled,): (bool,) = params.parse().map_err(invalid_params)?;
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    module.register_async_method("evm_setIntervalMining", |params, _ctx, _| async move {
        let (_interval_ms,): (u64,) = params.parse().map_err(invalid_params)?;
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    Ok(())
}

fn register_mirage_methods(
    module: &mut RpcModule<ServerContext>,
) -> std::result::Result<(), RegisterMethodError> {
    module.register_async_method("mirage_mintERC20", |params, ctx, _| async move {
        let (token, owner, amount): (Address, Address, U256) =
            params.parse().map_err(invalid_params)?;
        let staged = stage_erc20_mint(&ctx.state, token, owner, amount)
            .await
            .map_err(rpc_error)?;
        with_state_write(&ctx.state, |state| {
            touch_request(state);
            {
                let account = state.fork.db.dirty.accounts.entry(token).or_default();
                if let Some(slot) = staged.balance_slot {
                    account.erc20_balance_slot = Some(slot);
                }
                account.erc20_balances.insert(staged.owner, staged.balance);
            }
            for (slot, value) in &staged.storage_writes {
                state.fork.db.set_storage(token, *slot, *value);
            }
            let added_at_block = state.fork.local_block_number;
            state
                .fork
                .db
                .dirty
                .watch_list
                .entry(token)
                .or_insert_with(|| crate::fork::WatchEntry {
                    source: crate::fork::WatchSource::Manual,
                    added_at_block,
                    initial_slot_count: 1,
                    replay_count: 0,
                });
            Ok::<_, ErrorObjectOwned>(())
        })
        .await?;
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    module.register_async_method("mirage_prefetchAccount", |params, ctx, _| async move {
        let (address,): (Address,) = params.parse().map_err(invalid_params)?;
        let account = run_fork_snapshot(&ctx.state, true, move |mut fork| fork.db.basic(address))
            .await
            .map_err(rpc_error)?;
        if let Some(account) = account {
            let mut state = ctx.state.write();
            state.fork.db.read_cache.insert_account(address, account);
        }
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    module.register_async_method("mirage_prefetchSlots", |params, ctx, _| async move {
        let (address, slots): (Address, Vec<U256>) = params.parse().map_err(invalid_params)?;
        let prefetched = run_fork_snapshot(&ctx.state, true, move |mut fork| {
            slots
                .into_iter()
                .map(|slot| fork.db.storage(address, slot).map(|value| (slot, value)))
                .collect::<Result<Vec<_>>>()
        })
        .await
        .map_err(rpc_error)?;
        let mut state = ctx.state.write();
        for (slot, value) in prefetched {
            state
                .fork
                .db
                .read_cache
                .insert_storage(address, slot, value);
        }
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    module.register_async_method(MIRAGE_WATCH_CONTRACT_METHOD, |params, ctx, _| async move {
        let (address,): (Address,) = params.parse().map_err(invalid_params)?;
        with_state_write(&ctx.state, |state| {
            touch_request(state);
            let added_at_block = state.fork.local_block_number;
            state.fork.db.dirty.watch_list.insert(
                address,
                crate::fork::WatchEntry {
                    source: crate::fork::WatchSource::Manual,
                    added_at_block,
                    initial_slot_count: 0,
                    replay_count: 0,
                },
            );
        })
        .await;
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    module.register_async_method("mirage_unwatchContract", |params, ctx, _| async move {
        let (address,): (Address,) = params.parse().map_err(invalid_params)?;
        with_state_write(&ctx.state, |state| {
            touch_request(state);
            state.fork.db.dirty.watch_list.remove(&address);
            state.fork.db.dirty.unwatch_list.insert(address);
        })
        .await;
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    module.register_async_method("mirage_getWatchList", |_params, ctx, _| async move {
        let state = ctx.state.read();
        let watch_list = state
            .fork
            .db
            .dirty
            .watch_list
            .iter()
            .map(|(address, entry)| serde_json::json!({"address": address, "entry": entry}))
            .collect::<Vec<_>>();
        Ok::<_, ErrorObjectOwned>(watch_list)
    })?;
    module.register_async_method("mirage_getDirtySlots", |params, ctx, _| async move {
        let (address,): (Address,) = params.parse().map_err(invalid_params)?;
        let state = ctx.state.read();
        let slots = state
            .fork
            .db
            .dirty
            .accounts
            .get(&address)
            .map(|account| account.storage.clone())
            .unwrap_or_default();
        Ok::<_, ErrorObjectOwned>(slots)
    })?;
    module.register_async_method("mirage_getLastStateDiff", |_params, ctx, _| async move {
        let state = ctx.state.read();
        Ok::<_, ErrorObjectOwned>(state.last_committed_state_diff.clone())
    })?;
    module.register_async_method(MIRAGE_STATUS_METHOD, |_params, ctx, _| async move {
        let state = ctx.state.read();
        Ok::<_, ErrorObjectOwned>(state.fork.status(state.mode))
    })?;
    module.register_async_method(
        MIRAGE_GET_RESOURCE_USAGE_METHOD,
        |_params, ctx, _| async move {
            let usage = with_state_write(&ctx.state, |state| {
                touch_request(state);
                apply_resource_pressure(state);
                state.fork.resource_usage(&state.resource_model, state.mode)
            })
            .await;
            Ok::<_, ErrorObjectOwned>(usage)
        },
    )?;
    module.register_async_method("mirage_setResourceLimits", |params, ctx, _| async move {
        let (profile,): (Option<Profile>,) = params.parse().map_err(invalid_params)?;
        with_state_write(&ctx.state, |state| {
            touch_request(state);
            if let Some(profile) = profile {
                state.resource_model =
                    ResourceModel::for_profile(profile, state.resource_model.cache_ttl);
            }
        })
        .await;
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    module.register_async_method(MIRAGE_GET_POSITION_METHOD, |params, ctx, _| async move {
        let (request,): (PositionRequest,) = params.parse().map_err(invalid_params)?;
        let snapshot = run_fork_snapshot(&ctx.state, true, move |mut fork| {
            let balances = request
                .token_addresses
                .iter()
                .map(|address| {
                    let balance = fork
                        .db
                        .erc20_balance_of(*address, request.owner)
                        .or_else(|_| fork.db.basic(*address).map(|info| info.unwrap_or_default().balance))
                        .unwrap_or(U256::ZERO);
                    (*address, hex_u256(balance))
                })
                .collect::<Vec<_>>();
            let data = match request.protocol_type.as_str() {
                "raw-balances" => serde_json::json!({"balances": balances}),
                "uniswap-v3-position" => serde_json::json!({
                    "balances": balances,
                    "positionNftBalance": request
                        .contract
                        .and_then(|contract| fork.db.erc20_balance_of(contract, request.owner).ok())
                        .map_or_else(|| hex_u256(U256::ZERO), hex_u256),
                    "pool": request.contract,
                }),
                "aave-v3-account" => serde_json::json!({
                    "balances": balances,
                    "market": request.contract,
                    "healthFactor": "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                    "debtBalance": hex_u256(U256::ZERO),
                }),
                unknown => return Err(MirageError::UnknownProtocolType(unknown.to_owned())),
            };
            Ok(PositionSnapshot {
                owner: request.owner,
                protocol_type: request.protocol_type,
                data,
            })
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(snapshot)
    })?;
    module.register_async_method(
        MIRAGE_SUBSCRIBE_EVENTS_METHOD,
        |params, ctx, _| async move {
            let (filter,): (EventFilter,) = params.parse().map_err(invalid_params)?;
            let id = register_event_subscription(&ctx.state, filter);
            Ok::<_, ErrorObjectOwned>(id)
        },
    )?;
    module.register_subscription::<std::result::Result<(), SubscriptionError>, _, _>(
        "eth_subscribe",
        "eth_subscription",
        "eth_unsubscribe",
        |params, pending, ctx, _| async move {
            let params_vec: Vec<serde_json::Value> = params.parse().unwrap_or_default();
            let sub_type = params_vec
                .first()
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();

            match sub_type.as_str() {
                "newHeads" => {
                    let mut rx = ctx.state.read().new_heads_tx.subscribe();
                    let sink = match pending.accept().await {
                        Ok(s) => s,
                        Err(_) => return Ok(()),
                    };

                    loop {
                        tokio::select! {
                            _ = sink.closed() => break,
                            result = rx.recv() => {
                                match result {
                                    Ok(head) => {
                                        let header = new_heads_json(
                                            head.number,
                                            head.timestamp,
                                            head.gas_used,
                                            head.gas_limit,
                                            head.base_fee_per_gas,
                                            head.coinbase,
                                            head.prev_randao,
                                        );
                                        let Ok(msg) = serde_json::value::to_raw_value(&header) else {
                                            break;
                                        };
                                        if sink.send(msg).await.is_err() {
                                            break;
                                        }
                                    }
                                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                        // Fell behind — continue receiving from the latest
                                    }
                                    Err(_) => break,
                                }
                            }
                        }
                    }
                }
                "logs" => {
                    let filter = parse_log_subscription_filter(params_vec.get(1));
                    let mut rx = ctx.state.read().event_bus.subscribe();
                    let sink = match pending.accept().await {
                        Ok(s) => s,
                        Err(_) => return Ok(()),
                    };

                    loop {
                        tokio::select! {
                            _ = sink.closed() => break,
                            result = rx.recv() => {
                                match result {
                                    Ok(event) => {
                                        if !log_matches_subscription_filter(&event, &filter) {
                                            continue;
                                        }
                                        let block_hash = {
                                            let state = ctx.state.read();
                                            state
                                                .fork
                                                .blocks_by_number
                                                .get(&event.block_number)
                                                .map(|b| b.hash)
                                                .unwrap_or_default()
                                        };
                                        let log_json = format_log_subscription_result(
                                            &event, block_hash,
                                        );
                                        let Ok(msg) = serde_json::value::to_raw_value(&log_json)
                                        else {
                                            break;
                                        };
                                        if sink.send(msg).await.is_err() {
                                            break;
                                        }
                                    }
                                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                                    Err(_) => break,
                                }
                            }
                        }
                    }
                }
                "newPendingTransactions" => {
                    let mut rx = ctx.state.read().pending_tx_tx.subscribe();
                    let sink = match pending.accept().await {
                        Ok(s) => s,
                        Err(_) => return Ok(()),
                    };

                    loop {
                        tokio::select! {
                            _ = sink.closed() => break,
                            result = rx.recv() => {
                                match result {
                                    Ok(tx_hash) => {
                                        let hash_str = format!("{tx_hash}");
                                        let Ok(msg) = serde_json::value::to_raw_value(&hash_str)
                                        else {
                                            break;
                                        };
                                        if sink.send(msg).await.is_err() {
                                            break;
                                        }
                                    }
                                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                                    Err(_) => break,
                                }
                            }
                        }
                    }
                }
                "syncing" => {
                    let sink = match pending.accept().await {
                        Ok(s) => s,
                        Err(_) => return Ok(()),
                    };
                    // mirage-rs is always synced (local fork) — send `false` once, then
                    // keep the subscription open until the client disconnects.
                    let Ok(msg) = serde_json::value::to_raw_value(&false) else {
                        return Ok(());
                    };
                    let _ = sink.send(msg).await;
                    sink.closed().await;
                }
                _ => {
                    pending
                        .reject(ErrorObjectOwned::owned(
                            -32602,
                            format!("unsupported subscription type: {sub_type}"),
                            None::<()>,
                        ))
                        .await;
                }
            }

            Ok(())
        },
    )?;
    module.register_async_method(
        MIRAGE_BEGIN_SCENARIO_SET_METHOD,
        |params, ctx, _| async move {
            let (baseline,): (String,) = params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                touch_request(state);
                if state.reject_new_forks {
                    return Err::<String, _>(rpc_error(MirageError::Unsupported(
                        "resource pressure is refusing new scenario forks".to_owned(),
                    )));
                }
                let (baseline_snapshot_id, baseline_fork) = if baseline == "latest" {
                    let snapshot_id = state.fork.snapshot();
                    (snapshot_id, Some(state.fork.clone()))
                } else {
                    let snapshot_id = parse_hex_quantity(&baseline).map_err(invalid_params)?;
                    let mut baseline_fork = state.fork.clone();
                    baseline_fork.revert(snapshot_id).map_err(rpc_error)?;
                    let refreshed_snapshot = baseline_fork.snapshot();
                    (refreshed_snapshot, Some(baseline_fork))
                };
                let set_id = format!("set-{}", state.scenarios.len() + 1);
                state.scenarios.insert(
                    set_id.clone(),
                    ScenarioSet {
                        id: set_id.clone(),
                        baseline_snapshot_id,
                        baseline_fork,
                        scenarios: Vec::new(),
                        status: ScenarioSetStatus::Draft,
                    },
                );
                Ok::<_, ErrorObjectOwned>(set_id)
            })
            .await
        },
    )?;
    module.register_async_method(MIRAGE_DEFINE_SCENARIO_METHOD, |params, ctx, _| async move {
        let (set_id, scenario): (String, Scenario) = params.parse().map_err(invalid_params)?;
        with_state_write(&ctx.state, |state| {
            touch_request(state);
            let set = state
                .scenarios
                .get_mut(&set_id)
                .ok_or_else(|| rpc_error(MirageError::SetNotFound(set_id.clone())))?;
            if set.status != ScenarioSetStatus::Draft {
                return Err(rpc_error(MirageError::SetAlreadyRunning(set_id.clone())));
            }
            set.scenarios.push(scenario.clone());
            Ok::<_, ErrorObjectOwned>(scenario.id)
        })
        .await
    })?;
    module.register_async_method(
        MIRAGE_RUN_SCENARIO_SET_METHOD,
        |params, ctx, _| async move {
            let (set_id, mode): (String, RunMode) = params.parse().map_err(invalid_params)?;
            let job_id = {
                let mut state = ctx.state.write();
                let set = state
                    .scenarios
                    .get_mut(&set_id)
                    .ok_or_else(|| rpc_error(MirageError::SetNotFound(set_id.clone())))?;
                if set.status == ScenarioSetStatus::Running {
                    return Err(rpc_error(MirageError::SetAlreadyRunning(set_id.clone())));
                }
                if set.scenarios.is_empty() {
                    return Err(rpc_error(MirageError::SetHasNoScenarios(set_id.clone())));
                }
                set.status = ScenarioSetStatus::Running;
                let job_id = format!("job-{}", state.jobs.len() + 1);
                state.jobs.insert(
                    job_id.clone(),
                    ScenarioJob {
                        job_id: job_id.clone(),
                        set_id: set_id.clone(),
                        status: JobStatus::Running,
                        results: None,
                        total_wall_time_ms: None,
                    },
                );
                job_id
            };
            let state = Arc::clone(&ctx.state);
            let job_id_for_task = job_id.clone();
            tokio::spawn(async move {
                let started = tokio::time::Instant::now();
                let set = { state.read().scenarios.get(&set_id).cloned() };
                if let Some(set) = set {
                    let runner = ScenarioRunner::new(Arc::clone(&state));
                    let results = match mode {
                        RunMode::Sequential => runner.run_sequential(&set).await,
                        RunMode::Parallel => runner.run_parallel(&set).await,
                    };
                    let mut state = state.write();
                    if let Some(job) = state.jobs.get_mut(&job_id_for_task) {
                        job.status = JobStatus::Complete;
                        job.total_wall_time_ms =
                            Some(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX));
                        job.results = Some(results);
                    }
                    if let Some(set) = state.scenarios.get_mut(&set_id) {
                        set.status = ScenarioSetStatus::Complete;
                    }
                }
            });
            Ok::<_, ErrorObjectOwned>(job_id)
        },
    )?;
    module.register_async_method(
        MIRAGE_GET_SCENARIO_RESULTS_METHOD,
        |params, ctx, _| async move {
            let (job_id,): (String,) = params.parse().map_err(invalid_params)?;
            let state = ctx.state.read();
            let job = state
                .jobs
                .get(&job_id)
                .cloned()
                .ok_or_else(|| rpc_error(MirageError::JobNotFound(job_id.clone())))?;
            Ok::<_, ErrorObjectOwned>(job)
        },
    )?;
    module.register_async_method("mirage_compareScenarios", |params, ctx, _| async move {
        let (job_id,): (String,) = params.parse().map_err(invalid_params)?;
        let state = ctx.state.read();
        let job = state
            .jobs
            .get(&job_id)
            .ok_or_else(|| rpc_error(MirageError::JobNotFound(job_id.clone())))?;
        Ok::<_, ErrorObjectOwned>(rank_scenario_results(
            job.results.clone().unwrap_or_default(),
        ))
    })?;
    module.register_async_method(
        "mirage_computeDomainSeparator",
        |params, ctx, _| async move {
            let (contract,): (Address,) = params.parse().map_err(invalid_params)?;
            let result = run_fork_snapshot(&ctx.state, false, move |fork| {
                EvmExecutor::call(
                    &fork,
                    Address::ZERO,
                    contract,
                    Bytes::from_static(&[0x36, 0x44, 0xe5, 0x15]),
                    U256::ZERO,
                    100_000,
                )
            })
            .await
            .map_err(rpc_error)?;
            Ok::<_, ErrorObjectOwned>(format!("0x{}", hex::encode(&result.output)))
        },
    )?;
    module.register_async_method("mirage_cleanup", |_params, _ctx, _| async {
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    module.register_async_method(MIRAGE_SHUTDOWN_METHOD, |_params, ctx, _| async move {
        tracing::warn!("mirage_shutdown RPC called — initiating shutdown");
        let _ = ctx.shutdown.send(());
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    Ok(())
}

async fn run_fork_snapshot<T, F>(
    state: &Arc<RwLock<MirageState>>,
    touch: bool,
    task: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(ForkState) -> Result<T> + Send + 'static,
{
    let fork = if touch {
        let mut state = state.write();
        touch_request(&mut state);
        state.fork.clone()
    } else {
        state.read().fork.clone()
    };
    tokio::task::spawn_blocking(move || task(fork))
        .await
        .map_err(|error| MirageError::BackgroundTask(error.to_string()))?
}

async fn stage_erc20_mint(
    state: &Arc<RwLock<MirageState>>,
    token: Address,
    owner: Address,
    amount: U256,
) -> Result<StagedErc20Mint> {
    let fork = {
        let _writer_guard = lock_state_writes(state).await;
        let mut state = state.write();
        touch_request(&mut state);
        state.fork.clone()
    };
    tokio::task::spawn_blocking(move || stage_erc20_mint_on_snapshot(fork, token, owner, amount))
        .await
        .map_err(|error| MirageError::BackgroundTask(error.to_string()))?
}

fn stage_erc20_mint_on_snapshot(
    mut fork: ForkState,
    token: Address,
    owner: Address,
    amount: U256,
) -> Result<StagedErc20Mint> {
    let current_balance = fork.db.erc20_balance_of(token, owner).unwrap_or(U256::ZERO);
    let next_balance = current_balance.saturating_add(amount);
    let _ = fork.db.set_erc20_balance(token, owner, next_balance)?;
    let token_account = fork.db.dirty.accounts.get(&token);
    let balance_slot = token_account.and_then(|account| account.erc20_balance_slot);
    let storage_writes = token_account
        .map(|account| {
            account
                .storage
                .iter()
                .map(|(slot, value)| (*slot, *value))
                .collect()
        })
        .unwrap_or_default();
    let balance = token_account
        .and_then(|account| account.erc20_balances.get(&owner))
        .copied()
        .unwrap_or(next_balance);
    Ok(StagedErc20Mint {
        owner,
        balance,
        balance_slot,
        storage_writes,
    })
}

fn touch_request(state: &mut MirageState) {
    state.last_request_at = std::time::Instant::now();
    apply_resource_pressure(state);
}

fn apply_resource_pressure(state: &mut MirageState) {
    let usage = state.fork.resource_usage(&state.resource_model, state.mode);
    let action = usage.pressure_action();
    if action != PressureAction::None {
        // Non-blocking: BusSender::emit writes to a bounded broadcast + ring buffer
        // (drops the oldest replay entry when saturated; live send errors are ignored
        // when no subscribers are connected).
        state.telemetry.emit(MirageTelemetryEvent::ResourceWarning {
            resource: "memory".to_owned(),
            utilization: usage.resource_pressure.clamp(0.0, 1.0),
        });
    }
    apply_pressure_action(state, action);
}

fn apply_pressure_action(state: &mut MirageState, action: PressureAction) {
    match action {
        PressureAction::None => {
            state.reject_new_forks = false;
            state.fork.db.dirty.demote_protocols_to_slot_only = false;
        }
        PressureAction::EvictCache => {
            state.reject_new_forks = false;
            state.fork.db.dirty.demote_protocols_to_slot_only = false;
            let target_entries = state.resource_model.cache_capacity / 2;
            state.fork.db.evict_read_cache_to(target_entries);
        }
        PressureAction::Throttle => {
            state.reject_new_forks = false;
            state.fork.db.dirty.demote_protocols_to_slot_only = true;
            let target_entries = state.resource_model.cache_capacity / 4;
            state.fork.db.evict_read_cache_to(target_entries);
        }
        PressureAction::DemoteToProxy => {
            state.reject_new_forks = true;
            state.fork.db.dirty.demote_protocols_to_slot_only = true;
            state.fork.db.evict_read_cache_to(0);
            state
                .fork
                .db
                .dirty
                .watch_list
                .retain(|_, entry| matches!(entry.source, WatchSource::Manual));
            state.jobs.clear();
            state.scenarios.clear();
            state.mode = MirageMode::Proxy;
            let _ = state.mode_change.send(state.mode);
        }
    }
}

fn register_event_subscription(state: &Arc<RwLock<MirageState>>, filter: EventFilter) -> String {
    let mut state = state.write();
    state.last_request_at = std::time::Instant::now();
    state.next_event_subscription_id = state.next_event_subscription_id.saturating_add(1);
    let stream_id = format!("stream-{}", state.next_event_subscription_id);
    state.event_subscriptions.insert(stream_id.clone(), filter);
    stream_id
}

fn publish_receipt_events(state: &MirageState, receipt: &LocalReceipt) {
    for log in &receipt.logs {
        let event = MirageEvent {
            block_number: receipt.block_number,
            tx_hash: receipt.transaction_hash,
            log_index: log.log_index,
            contract: log.address,
            topics: log.topics.clone(),
            data: log.data.clone(),
            source: EventSource::LocalTx,
            decoded: None,
        };
        // Keep event publication non-blocking like golem-core's bounded event fan-out:
        // producers never wait on consumers, and lagging subscribers may miss events.
        let _ = state.event_bus.send(event);
    }
}

async fn health_handler(State(state): State<Arc<RwLock<MirageState>>>) -> impl IntoResponse {
    let state = state.read();
    axum::Json(state.fork.status(state.mode))
}

async fn event_ws_handler(
    Path(stream_id): Path<String>,
    State(state): State<Arc<RwLock<MirageState>>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let filter = {
        let state = state.read();
        state.event_subscriptions.get(&stream_id).cloned()
    };
    filter.map_or_else(
        || StatusCode::NOT_FOUND.into_response(),
        |filter| ws.on_upgrade(move |socket| handle_event_socket(socket, state, filter)),
    )
}

async fn unsubscribe_event_handler(
    Path(stream_id): Path<String>,
    State(state): State<Arc<RwLock<MirageState>>>,
) -> impl IntoResponse {
    let removed = state
        .write()
        .event_subscriptions
        .remove(&stream_id)
        .is_some();
    axum::Json(removed)
}

async fn handle_event_socket(
    mut socket: WebSocket,
    state: Arc<RwLock<MirageState>>,
    filter: EventFilter,
) {
    let mut receiver = state.read().event_bus.subscribe();
    while let Ok(event) = receiver.recv().await {
        if !event_matches_filter(&event, &filter) {
            continue;
        }
        let payload = match serde_json::to_string(&event) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!("failed to serialize mirage event: {error}");
                continue;
            }
        };
        if socket.send(Message::Text(payload.into())).await.is_err() {
            break;
        }
    }
}

fn event_matches_filter(event: &MirageEvent, filter: &EventFilter) -> bool {
    let address_match = filter
        .addresses
        .as_ref()
        .is_none_or(|addresses| addresses.contains(&event.contract));
    let topic_match = filter
        .topics
        .as_ref()
        .is_none_or(|topics| event.topics.iter().any(|topic| topics.contains(topic)));
    address_match && topic_match
}

// ---------------------------------------------------------------------------
// eth_subscribe("logs") filter types and helpers
// ---------------------------------------------------------------------------

/// Parsed filter for `eth_subscribe("logs", {filter})`.
///
/// Each topic position can be `None` (any value), a single hash, or multiple
/// hashes (OR semantics within the position).
struct LogSubscriptionFilter {
    addresses: Vec<Address>,
    topics: Vec<Option<Vec<B256>>>,
}

/// Parses the optional filter object from the `eth_subscribe("logs", filter)` params.
///
/// Supports `address` as a single hex string or an array of hex strings, and
/// `topics` as an array where each position is `null`, a single hash, or an
/// array of hashes (standard Ethereum topic filter semantics).
fn parse_log_subscription_filter(filter_val: Option<&serde_json::Value>) -> LogSubscriptionFilter {
    let Some(obj) = filter_val.and_then(|v| v.as_object()) else {
        return LogSubscriptionFilter {
            addresses: Vec::new(),
            topics: Vec::new(),
        };
    };

    // Parse `address`: single string or array of strings.
    let addresses = match obj.get("address") {
        Some(serde_json::Value::String(s)) => s.parse::<Address>().into_iter().collect(),
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str()?.parse::<Address>().ok())
            .collect(),
        _ => Vec::new(),
    };

    // Parse `topics`: array where each element is null, a single hash string,
    // or an array of hash strings.
    let topics = match obj.get("topics") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .map(|entry| match entry {
                serde_json::Value::Null => None,
                serde_json::Value::String(s) => s.parse::<B256>().ok().map(|h| vec![h]),
                serde_json::Value::Array(inner) => {
                    let hashes: Vec<B256> = inner
                        .iter()
                        .filter_map(|v| v.as_str()?.parse::<B256>().ok())
                        .collect();
                    if hashes.is_empty() {
                        None
                    } else {
                        Some(hashes)
                    }
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };

    LogSubscriptionFilter { addresses, topics }
}

/// Checks whether a `MirageEvent` matches a log subscription filter.
fn log_matches_subscription_filter(event: &MirageEvent, filter: &LogSubscriptionFilter) -> bool {
    // Address check: if addresses are specified, the event contract must be in the list.
    if !filter.addresses.is_empty() && !filter.addresses.contains(&event.contract) {
        return false;
    }

    // Topic check: position-based.  If the filter specifies more topic positions
    // than the event has, positions beyond the event's topics cannot match.
    for (i, constraint) in filter.topics.iter().enumerate() {
        if let Some(allowed) = constraint {
            match event.topics.get(i) {
                Some(actual) => {
                    if !allowed.contains(actual) {
                        return false;
                    }
                }
                // Filter requires a value at this position but the log doesn't have it.
                None => return false,
            }
        }
        // `None` constraint = wildcard, always matches.
    }

    true
}

/// Formats a `MirageEvent` as the standard Ethereum log subscription result.
fn format_log_subscription_result(event: &MirageEvent, block_hash: B256) -> serde_json::Value {
    let topics: Vec<String> = event.topics.iter().map(|t| format!("{t}")).collect();
    serde_json::json!({
        "address": format!("{}", event.contract),
        "topics": topics,
        "data": format!("0x{}", hex::encode(event.data.as_ref())),
        "blockNumber": hex_u64(event.block_number),
        "blockHash": format!("{block_hash}"),
        "transactionHash": format!("{}", event.tx_hash),
        "transactionIndex": "0x0",
        "logIndex": hex_u64(u64::from(event.log_index)),
        "removed": false,
    })
}

fn extract_to(kind: Option<Address>) -> Option<Address> {
    kind
}

/// Collect log entries from the fork state matching filter criteria.
fn collect_filter_logs(
    fork: &ForkState,
    from: u64,
    to: u64,
    topics: &[Option<Vec<B256>>],
    addresses: &[Address],
) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for (num, block) in fork.blocks_by_number.range(from..=to) {
        for tx_hash in &block.transactions {
            let Some(receipt) = fork.receipts.get(tx_hash) else {
                continue;
            };
            for log in &receipt.logs {
                if !addresses.is_empty() && !addresses.contains(&log.address) {
                    continue;
                }
                if !topics.is_empty() {
                    let mut matched = true;
                    for (i, topic_filter) in topics.iter().enumerate() {
                        if let Some(allowed) = topic_filter {
                            let actual = log.topics.get(i);
                            if !actual.is_some_and(|t| allowed.contains(t)) {
                                matched = false;
                                break;
                            }
                        }
                    }
                    if !matched {
                        continue;
                    }
                }
                out.push(serde_json::json!({
                    "address": log.address,
                    "topics": log.topics,
                    "data": format!("0x{}", hex::encode(log.data.as_ref())),
                    "blockNumber": hex_u64(*num),
                    "blockHash": block.hash,
                    "transactionHash": tx_hash,
                    "transactionIndex": "0x0",
                    "logIndex": hex_u64(log.log_index as u64),
                    "removed": false,
                }));
            }
        }
    }
    out
}

fn resolve_block_tag(v: Option<&serde_json::Value>, tip: u64) -> Option<u64> {
    match v? {
        serde_json::Value::String(s) => match s.as_str() {
            "latest" | "pending" | "safe" | "finalized" => Some(tip),
            "earliest" => Some(0),
            hex => parse_hex_quantity(hex).ok(),
        },
        serde_json::Value::Number(n) => n.as_u64(),
        _ => None,
    }
}

fn receipt_json(receipt: &LocalReceipt) -> serde_json::Value {
    // For contract-creation txs, the diff.output carries the deployed address
    // (20 bytes); surface it as `contractAddress` so standard clients see it.
    let contract_address = if receipt.to.is_none() && receipt.state_diff.output.len() == 20 {
        Some(format!(
            "0x{}",
            hex::encode(receipt.state_diff.output.as_ref())
        ))
    } else {
        None
    };
    // Zero-bloom (no indexed-log prefilter) — sufficient for tests and roko agents.
    let zero_bloom = format!("0x{}", "0".repeat(512));
    // Enrich log entries with block+tx context so alloy-style receipt
    // deserialization succeeds.
    let logs_full: Vec<serde_json::Value> = receipt
        .logs
        .iter()
        .map(|l| {
            serde_json::json!({
                "address": l.address,
                "topics": l.topics,
                "data": format!("0x{}", hex::encode(l.data.as_ref())),
                "logIndex": hex_u64(l.log_index as u64),
                "blockHash": receipt.block_hash,
                "blockNumber": hex_u64(receipt.block_number),
                "transactionHash": receipt.transaction_hash,
                "transactionIndex": "0x0",
                "removed": false,
            })
        })
        .collect();
    serde_json::json!({
        "type": "0x2",
        "transactionHash": receipt.transaction_hash,
        "transactionIndex": "0x0",
        "blockHash": receipt.block_hash,
        "blockNumber": hex_u64(receipt.block_number),
        "from": receipt.from,
        "to": receipt.to,
        "cumulativeGasUsed": hex_u64(receipt.gas_used),
        "gasUsed": hex_u64(receipt.gas_used),
        "effectiveGasPrice": "0x1",
        "contractAddress": contract_address,
        "logs": logs_full,
        "logsBloom": zero_bloom,
        "status": if receipt.success { "0x1" } else { "0x0" },
    })
}

fn transaction_json(tx: &LocalTransaction) -> serde_json::Value {
    serde_json::json!({
        "hash": tx.hash,
        "from": tx.from,
        "to": tx.to,
        "value": hex_u256(tx.value),
        "input": format!("0x{}", hex::encode(&tx.input)),

        "gas": hex_u64(tx.gas),
        "nonce": hex_u64(tx.nonce),
        "blockNumber": hex_u64(tx.block_number),
    })
}

fn block_json(block: &LocalBlock) -> serde_json::Value {
    let parent_hash = if block.number > 0 {
        keccak256((block.number - 1).to_le_bytes())
    } else {
        B256::ZERO
    };
    serde_json::json!({
        "hash": block.hash,
        "parentHash": format!("{parent_hash}"),
        "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
        "miner": block.coinbase,
        "stateRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "transactionsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
        "receiptsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
        "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "difficulty": "0x0",
        "number": hex_u64(block.number),
        "gasLimit": hex_u64(block.gas_limit),
        "gasUsed": hex_u64(block.gas_used),
        "timestamp": hex_u64(block.timestamp),
        "extraData": "0x",
        "mixHash": format!("{}", block.prev_randao),
        "nonce": "0x0000000000000000",
        "baseFeePerGas": format!("0x{:x}", block.base_fee_per_gas),
        "transactions": block.transactions,
    })
}

fn new_heads_json(
    block_num: u64,
    timestamp: u64,
    gas_used: u64,
    gas_limit: u64,
    base_fee: u128,
    coinbase: Address,
    prev_randao: B256,
) -> serde_json::Value {
    let hash = keccak256(block_num.to_le_bytes());
    serde_json::json!({
        "hash": format!("{hash}"),
        "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
        "miner": coinbase,
        "stateRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "transactionsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
        "receiptsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
        "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "difficulty": "0x0",
        "number": hex_u64(block_num),
        "gasLimit": hex_u64(gas_limit),
        "gasUsed": hex_u64(gas_used),
        "timestamp": hex_u64(timestamp),
        "extraData": "0x",
        "mixHash": format!("{prev_randao}"),
        "nonce": "0x0000000000000000",
        "baseFeePerGas": format!("0x{base_fee:x}"),
    })
}

fn hex_u64(value: u64) -> String {
    format!("0x{value:x}")
}

fn hex_u256(value: U256) -> String {
    format!("0x{value:x}")
}

const Q96: f64 = (1_u128 << 96) as f64;

/// Converts a human-readable price into a Uniswap V3 `sqrtPriceX96` value.
///
/// The helper is pure and returns zero for non-finite or non-positive inputs.
#[must_use]
pub fn to_sqrt_price_x96(price: f64) -> U256 {
    if !price.is_finite() || price <= 0.0 {
        return U256::ZERO;
    }

    let sqrt_price_x96 = (price.sqrt() * Q96).round();
    if !sqrt_price_x96.is_finite() || sqrt_price_x96 <= 0.0 || sqrt_price_x96 > u128::MAX as f64 {
        return U256::ZERO;
    }

    U256::from(sqrt_price_x96 as u128)
}

/// Converts a Uniswap V3 `sqrtPriceX96` value back into a human-readable price.
///
/// The helper is pure and returns zero for values it cannot parse.
#[must_use]
pub fn from_sqrt_price_x96(sqrt_price_x96: U256) -> f64 {
    if sqrt_price_x96.is_zero() {
        return 0.0;
    }

    let sqrt_price = sqrt_price_x96.to_string().parse::<f64>().unwrap_or(0.0);
    let ratio = sqrt_price / Q96;
    ratio * ratio
}

/// Spawns a Mirage test child process for integration tests.
///
/// # Errors
///
/// Returns any error from [`crate::spawn_mirage_test_instance`].
///
/// Reads the listening port from the `MIRAGE_TEST_PORT` environment variable when set to a valid
/// `u16`. If unset or unparsable, binds **18552**.
pub async fn mirage_instance_or_env() -> crate::Result<crate::MirageTestInstance> {
    let port = std::env::var("MIRAGE_TEST_PORT")
        .ok()
        .and_then(|raw| raw.parse::<u16>().ok())
        .unwrap_or(18_552);
    crate::spawn_mirage_test_instance(None, Some(port)).await
}

fn parse_hex_quantity(value: &str) -> Result<u64> {
    u64::from_str_radix(value.trim_start_matches("0x"), 16)
        .map_err(|error| MirageError::InvalidParams(format!("invalid hex quantity: {error}")))
}

fn parse_hex_u256(value: &str) -> Result<U256> {
    U256::from_str_radix(value.trim_start_matches("0x"), 16)
        .map_err(|error| MirageError::InvalidParams(format!("invalid hex U256: {error}")))
}

#[derive(Debug, Clone)]
struct CommittedTransaction {
    tx_hash: B256,
    block_number: u64,
    diff: crate::replay::StateDiff,
    transaction: LocalTransaction,
    receipt: LocalReceipt,
    block: LocalBlock,
}

async fn commit_transaction_request(
    state: &Arc<RwLock<MirageState>>,
    request: TransactionRequest,
    override_hash: Option<B256>,
) -> Result<B256> {
    let fork = {
        let _writer_guard = lock_state_writes(state).await;
        let mut state = state.write();
        touch_request(&mut state);
        state.fork.clone()
    };
    let committed = tokio::task::spawn_blocking(move || {
        commit_transaction_on_snapshot(fork, request, override_hash)
    })
    .await
    .map_err(|error| MirageError::BackgroundTask(error.to_string()))??;
    let receipt = committed.receipt.clone();
    let classifier = DiffClassifier::new(ClassificationConfig::default());
    let CommittedTransaction {
        diff,
        transaction,
        receipt: committed_receipt,
        block,
        block_number,
        tx_hash,
    } = committed;
    let invalidate_request = TransactionRequest {
        from: Some(transaction.from),
        to: transaction.to,
        gas: Some(transaction.gas),
        value: Some(transaction.value),
        data: Some(transaction.input.clone()),
        ..Default::default()
    };
    let _writer_guard = lock_state_writes(state).await;
    let mut state = state.write();
    state
        .fork
        .commit_local_transaction(&diff, transaction, committed_receipt, block);
    classifier.apply(&mut state.fork.db.dirty, &diff, block_number)?;
    state
        .speculative_executor
        .lock()
        .invalidate_for_request(&invalidate_request);
    state.last_committed_state_diff = Some(diff);
    publish_receipt_events(&state, &receipt);
    let _ = state.pending_tx_tx.send(tx_hash);
    Ok(tx_hash)
}

fn commit_transaction_on_snapshot(
    mut fork: ForkState,
    request: TransactionRequest,
    override_hash: Option<B256>,
) -> Result<CommittedTransaction> {
    let from = request
        .from
        .ok_or_else(|| MirageError::InvalidParams("missing from".to_owned()))?;
    let to = extract_to(request.to);
    let data = request.data.unwrap_or_default();
    let value = request.value.unwrap_or(U256::ZERO);
    let gas = request.gas.unwrap_or(21_000);
    let (_result, diff) = EvmExecutor::transact(&mut fork, from, to, data, value, gas)?;
    let current_hash = latest_local_tx_hash(&fork)?;
    let tx_hash = if let Some(expected_hash) = override_hash {
        adopt_latest_transaction_hash(&mut fork, current_hash, expected_hash)?;
        expected_hash
    } else {
        current_hash
    };
    let transaction = fork
        .transactions
        .get(&tx_hash)
        .cloned()
        .ok_or_else(|| MirageError::Unsupported("missing tx after commit".to_owned()))?;
    let receipt = fork
        .receipts
        .get(&tx_hash)
        .cloned()
        .ok_or_else(|| MirageError::Unsupported("missing receipt after commit".to_owned()))?;
    let block = fork
        .blocks_by_number
        .get(&transaction.block_number)
        .cloned()
        .ok_or_else(|| MirageError::Unsupported("missing block after commit".to_owned()))?;

    Ok(CommittedTransaction {
        tx_hash,
        block_number: transaction.block_number,
        diff,
        transaction,
        receipt,
        block,
    })
}

fn latest_local_tx_hash(state: &ForkState) -> Result<B256> {
    state
        .transactions
        .iter()
        .max_by_key(|(_, tx)| tx.block_number)
        .map(|(hash, _)| *hash)
        .ok_or_else(|| MirageError::Unsupported("missing tx after commit".to_owned()))
}

fn adopt_latest_transaction_hash(
    state: &mut ForkState,
    current_hash: B256,
    new_hash: B256,
) -> Result<()> {
    if current_hash == new_hash {
        return Ok(());
    }

    let mut tx = state.transactions.remove(&current_hash).ok_or_else(|| {
        MirageError::Unsupported("latest transaction missing from store".to_owned())
    })?;
    tx.hash = new_hash;
    let block_number = tx.block_number;
    state.transactions.insert(new_hash, tx);

    let mut receipt = state
        .receipts
        .remove(&current_hash)
        .ok_or_else(|| MirageError::Unsupported("latest receipt missing from store".to_owned()))?;
    receipt.transaction_hash = new_hash;
    state.receipts.insert(new_hash, receipt);

    if let Some(block) = state.blocks_by_number.get_mut(&block_number) {
        for hash in &mut block.transactions {
            if *hash == current_hash {
                *hash = new_hash;
            }
        }
    }
    if let Some(block) = state
        .blocks_by_hash
        .values_mut()
        .find(|block| block.number == block_number)
    {
        for hash in &mut block.transactions {
            if *hash == current_hash {
                *hash = new_hash;
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct DecodedRawTransaction {
    tx_hash: B256,
    request: TransactionRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RlpValue {
    Bytes(Vec<u8>),
    List(Vec<Self>),
}

fn decode_signed_raw_transaction(raw: &Bytes) -> Result<DecodedRawTransaction> {
    if raw.is_empty() {
        return Err(MirageError::InvalidParams(
            "raw transaction is empty".to_owned(),
        ));
    }

    let tx_hash = keccak256(raw);
    match raw[0] {
        0x01 => decode_typed_raw_transaction(0x01, &raw[1..], tx_hash),
        0x02 => decode_typed_raw_transaction(0x02, &raw[1..], tx_hash),
        0x03 => decode_typed_raw_transaction(0x03, &raw[1..], tx_hash),
        _ => decode_legacy_raw_transaction(raw, tx_hash),
    }
}

fn decode_legacy_raw_transaction(raw: &[u8], tx_hash: B256) -> Result<DecodedRawTransaction> {
    let fields = rlp_decode_top_list(raw)?;
    if fields.len() != 9 {
        return Err(MirageError::InvalidParams(format!(
            "legacy transaction must have 9 fields, found {}",
            fields.len()
        )));
    }

    let nonce = rlp_u64(&fields[0])?;
    let gas_price = rlp_u128(&fields[1])?;
    let gas = rlp_u64(&fields[2])?;
    let to = rlp_address_opt(&fields[3])?;
    let value = rlp_u256(&fields[4])?;
    let data = Bytes::from(rlp_bytes(&fields[5])?);
    let v = rlp_u64(&fields[6])?;
    let r = rlp_u256(&fields[7])?;
    let s = rlp_u256(&fields[8])?;
    let (chain_id, recovery_id) = decode_legacy_v(v)?;

    let mut signing_fields = fields[..6].to_vec();
    if let Some(chain_id) = chain_id {
        signing_fields.push(rlp_from_u64(chain_id));
        signing_fields.push(RlpValue::Bytes(Vec::new()));
        signing_fields.push(RlpValue::Bytes(Vec::new()));
    }
    let signing_hash = keccak256(rlp_encode(&RlpValue::List(signing_fields)));
    let from = recover_address(signing_hash, r, s, recovery_id)?;

    Ok(DecodedRawTransaction {
        tx_hash,
        request: TransactionRequest {
            from: Some(from),
            to,
            gas: Some(gas),
            value: Some(value),
            data: Some(data),
            gas_price: Some(gas_price),
            nonce: Some(nonce),
            chain_id,
        },
    })
}

fn decode_typed_raw_transaction(
    tx_type: u8,
    raw: &[u8],
    tx_hash: B256,
) -> Result<DecodedRawTransaction> {
    let fields = rlp_decode_top_list(raw)?;
    let (chain_id, nonce, gas_price, gas, to, value, data, signing_fields, recovery_id, r, s) =
        match tx_type {
            0x01 => {
                if fields.len() != 11 {
                    return Err(MirageError::InvalidParams(format!(
                        "type 1 transaction must have 11 fields, found {}",
                        fields.len()
                    )));
                }
                (
                    Some(rlp_u64(&fields[0])?),
                    Some(rlp_u64(&fields[1])?),
                    Some(rlp_u128(&fields[2])?),
                    Some(rlp_u64(&fields[3])?),
                    rlp_address_opt(&fields[4])?,
                    Some(rlp_u256(&fields[5])?),
                    Bytes::from(rlp_bytes(&fields[6])?),
                    fields[..8].to_vec(),
                    rlp_u8(&fields[8])?,
                    rlp_u256(&fields[9])?,
                    rlp_u256(&fields[10])?,
                )
            }
            0x02 => {
                if fields.len() != 12 {
                    return Err(MirageError::InvalidParams(format!(
                        "type 2 transaction must have 12 fields, found {}",
                        fields.len()
                    )));
                }
                (
                    Some(rlp_u64(&fields[0])?),
                    Some(rlp_u64(&fields[1])?),
                    Some(rlp_u128(&fields[3])?),
                    Some(rlp_u64(&fields[4])?),
                    rlp_address_opt(&fields[5])?,
                    Some(rlp_u256(&fields[6])?),
                    Bytes::from(rlp_bytes(&fields[7])?),
                    fields[..9].to_vec(),
                    rlp_u8(&fields[9])?,
                    rlp_u256(&fields[10])?,
                    rlp_u256(&fields[11])?,
                )
            }
            0x03 => {
                if fields.len() != 14 {
                    return Err(MirageError::InvalidParams(format!(
                        "type 3 transaction must have 14 fields, found {}",
                        fields.len()
                    )));
                }
                (
                    Some(rlp_u64(&fields[0])?),
                    Some(rlp_u64(&fields[1])?),
                    Some(rlp_u128(&fields[3])?),
                    Some(rlp_u64(&fields[4])?),
                    rlp_address_opt(&fields[5])?,
                    Some(rlp_u256(&fields[6])?),
                    Bytes::from(rlp_bytes(&fields[7])?),
                    fields[..11].to_vec(),
                    rlp_u8(&fields[11])?,
                    rlp_u256(&fields[12])?,
                    rlp_u256(&fields[13])?,
                )
            }
            _ => {
                return Err(MirageError::InvalidParams(format!(
                    "unsupported typed transaction {tx_type:#x}"
                )));
            }
        };

    let mut payload = vec![tx_type];
    payload.extend_from_slice(&rlp_encode(&RlpValue::List(signing_fields)));
    let signing_hash = keccak256(payload);
    let from = recover_address(signing_hash, r, s, recovery_id)?;

    Ok(DecodedRawTransaction {
        tx_hash,
        request: TransactionRequest {
            from: Some(from),
            to,
            gas,
            value,
            data: Some(data),
            gas_price,
            nonce,
            chain_id,
        },
    })
}

fn decode_legacy_v(v: u64) -> Result<(Option<u64>, u8)> {
    match v {
        27 => Ok((None, 0)),
        28 => Ok((None, 1)),
        value if value >= 35 => {
            let adjusted = value - 35;
            let parity = u8::from(adjusted % 2 != 0);
            Ok((Some(adjusted / 2), parity))
        }
        _ => Err(MirageError::InvalidParams(format!(
            "invalid legacy v value {v}"
        ))),
    }
}

fn recover_address(signing_hash: B256, r: U256, s: U256, recovery_id: u8) -> Result<Address> {
    let recovery_id = RecoveryId::from_byte(recovery_id)
        .ok_or_else(|| MirageError::InvalidParams(format!("invalid recovery id {recovery_id}")))?;
    let signature =
        Signature::from_scalars(r.to_be_bytes::<32>(), s.to_be_bytes::<32>()).map_err(|error| {
            MirageError::InvalidParams(format!("invalid signature scalars: {error}"))
        })?;
    let verifying_key =
        VerifyingKey::recover_from_prehash(signing_hash.as_slice(), &signature, recovery_id)
            .map_err(|error| {
                MirageError::InvalidParams(format!("failed to recover sender: {error}"))
            })?;
    let encoded = verifying_key.to_encoded_point(false);
    let hash = keccak256(&encoded.as_bytes()[1..]);
    Ok(Address::from_slice(&hash.as_slice()[12..]))
}

fn rlp_decode_top_list(input: &[u8]) -> Result<Vec<RlpValue>> {
    let (value, consumed) = rlp_decode(input)?;
    if consumed != input.len() {
        return Err(MirageError::InvalidParams(
            "unexpected trailing RLP bytes".to_owned(),
        ));
    }
    match value {
        RlpValue::List(fields) => Ok(fields),
        RlpValue::Bytes(_) => Err(MirageError::InvalidParams(
            "expected top-level RLP list".to_owned(),
        )),
    }
}

fn rlp_decode(input: &[u8]) -> Result<(RlpValue, usize)> {
    let first = *input
        .first()
        .ok_or_else(|| MirageError::InvalidParams("unexpected end of RLP input".to_owned()))?;
    match first {
        0x00..=0x7f => Ok((RlpValue::Bytes(vec![first]), 1)),
        0x80..=0xb7 => {
            let len = usize::from(first - 0x80);
            let end = 1 + len;
            let bytes = input
                .get(1..end)
                .ok_or_else(|| MirageError::InvalidParams("short RLP string".to_owned()))?;
            Ok((RlpValue::Bytes(bytes.to_vec()), end))
        }
        0xb8..=0xbf => {
            let len_of_len = usize::from(first - 0xb7);
            let len = rlp_len(input, 1, len_of_len)?;
            let start = 1 + len_of_len;
            let end = start + len;
            let bytes = input
                .get(start..end)
                .ok_or_else(|| MirageError::InvalidParams("short long RLP string".to_owned()))?;
            Ok((RlpValue::Bytes(bytes.to_vec()), end))
        }
        0xc0..=0xf7 => {
            let len = usize::from(first - 0xc0);
            let start = 1;
            let end = start + len;
            let payload = input
                .get(start..end)
                .ok_or_else(|| MirageError::InvalidParams("short RLP list".to_owned()))?;
            Ok((RlpValue::List(rlp_decode_list_payload(payload)?), end))
        }
        0xf8..=0xff => {
            let len_of_len = usize::from(first - 0xf7);
            let len = rlp_len(input, 1, len_of_len)?;
            let start = 1 + len_of_len;
            let end = start + len;
            let payload = input
                .get(start..end)
                .ok_or_else(|| MirageError::InvalidParams("short long RLP list".to_owned()))?;
            Ok((RlpValue::List(rlp_decode_list_payload(payload)?), end))
        }
    }
}

fn rlp_decode_list_payload(mut payload: &[u8]) -> Result<Vec<RlpValue>> {
    let mut values = Vec::new();
    while !payload.is_empty() {
        let (value, consumed) = rlp_decode(payload)?;
        values.push(value);
        payload = &payload[consumed..];
    }
    Ok(values)
}

fn rlp_len(input: &[u8], start: usize, len_of_len: usize) -> Result<usize> {
    let end = start + len_of_len;
    let bytes = input
        .get(start..end)
        .ok_or_else(|| MirageError::InvalidParams("short RLP length".to_owned()))?;
    bytes.iter().try_fold(0_usize, |acc, byte| {
        acc.checked_mul(256)
            .and_then(|value| value.checked_add(usize::from(*byte)))
            .ok_or_else(|| MirageError::InvalidParams("RLP length overflow".to_owned()))
    })
}

fn rlp_bytes(value: &RlpValue) -> Result<Vec<u8>> {
    match value {
        RlpValue::Bytes(bytes) => Ok(bytes.clone()),
        RlpValue::List(_) => Err(MirageError::InvalidParams("expected RLP bytes".to_owned())),
    }
}

fn rlp_u64(value: &RlpValue) -> Result<u64> {
    let bytes = rlp_bytes(value)?;
    if bytes.is_empty() {
        return Ok(0);
    }
    if bytes.len() > 8 {
        return Err(MirageError::InvalidParams(
            "integer does not fit in u64".to_owned(),
        ));
    }
    Ok(bytes
        .into_iter()
        .fold(0_u64, |acc, byte| (acc << 8) | u64::from(byte)))
}

fn rlp_u128(value: &RlpValue) -> Result<u128> {
    let bytes = rlp_bytes(value)?;
    if bytes.is_empty() {
        return Ok(0);
    }
    if bytes.len() > 16 {
        return Err(MirageError::InvalidParams(
            "integer does not fit in u128".to_owned(),
        ));
    }
    Ok(bytes
        .into_iter()
        .fold(0_u128, |acc, byte| (acc << 8) | u128::from(byte)))
}

fn rlp_u8(value: &RlpValue) -> Result<u8> {
    let value = rlp_u64(value)?;
    u8::try_from(value)
        .map_err(|_| MirageError::InvalidParams(format!("integer does not fit in u8: {value}")))
}

fn rlp_u256(value: &RlpValue) -> Result<U256> {
    Ok(U256::from_be_slice(&rlp_bytes(value)?))
}

fn rlp_address_opt(value: &RlpValue) -> Result<Option<Address>> {
    let bytes = rlp_bytes(value)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.len() != 20 {
        return Err(MirageError::InvalidParams(format!(
            "address must be 20 bytes, found {}",
            bytes.len()
        )));
    }
    Ok(Some(Address::from_slice(&bytes)))
}

fn rlp_from_u64(value: u64) -> RlpValue {
    if value == 0 {
        return RlpValue::Bytes(Vec::new());
    }
    RlpValue::Bytes(trim_leading_zeros(value.to_be_bytes().to_vec()))
}

fn rlp_encode(value: &RlpValue) -> Vec<u8> {
    match value {
        RlpValue::Bytes(bytes) => rlp_encode_bytes(bytes),
        RlpValue::List(items) => {
            let payload = items.iter().flat_map(rlp_encode).collect::<Vec<_>>();
            rlp_encode_with_offset(&payload, 0xc0, 0xf7)
        }
    }
}

fn rlp_encode_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        vec![bytes[0]]
    } else {
        rlp_encode_with_offset(bytes, 0x80, 0xb7)
    }
}

fn rlp_encode_with_offset(payload: &[u8], short_offset: u8, long_offset: u8) -> Vec<u8> {
    if payload.len() <= 55 {
        let mut encoded = Vec::with_capacity(1 + payload.len());
        let short_len = u8::try_from(payload.len())
            .unwrap_or_else(|_| unreachable!("short RLP payload length always fits in u8"));
        encoded.push(short_offset + short_len);
        encoded.extend_from_slice(payload);
        encoded
    } else {
        let length_bytes = trim_leading_zeros((payload.len() as u64).to_be_bytes().to_vec());
        let mut encoded = Vec::with_capacity(1 + length_bytes.len() + payload.len());
        let length_of_length = u8::try_from(length_bytes.len())
            .unwrap_or_else(|_| unreachable!("RLP length-of-length always fits in u8"));
        encoded.push(long_offset + length_of_length);
        encoded.extend_from_slice(&length_bytes);
        encoded.extend_from_slice(payload);
        encoded
    }
}

fn trim_leading_zeros(mut bytes: Vec<u8>) -> Vec<u8> {
    let first_non_zero = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len());
    bytes.drain(..first_non_zero);
    bytes
}

fn invalid_params(error: impl std::fmt::Display) -> ErrorObjectOwned {
    invalid_params_message(error.to_string())
}

fn invalid_params_message(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(-32602, message.into(), None::<()>)
}

fn rpc_error(error: MirageError) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(error.rpc_code(), error.to_string(), None::<()>)
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, sync::Arc, time::Duration};

    use alloy_primitives::{Address, B256, Bytes, U256, address, keccak256};
    use k256::{
        FieldBytes,
        ecdsa::{SigningKey, hazmat::SignPrimitive},
        sha2,
    };
    use tokio::{sync::broadcast, time::sleep};

    use super::{
        ERC8004_IDENTITY_REGISTRY, MIRAGE_IDENTITY_REGISTRY_ALIAS, RlpValue, ServerContext,
        apply_pressure_action, build_rpc_module, commit_transaction_request,
        decode_signed_raw_transaction, ensure_erc8004_boot_contracts, from_sqrt_price_x96,
        parse_hex_quantity, rlp_encode, rlp_from_u64, rpc_error, run_fork_snapshot,
        stage_erc20_mint, to_sqrt_price_x96,
    };
    use crate::{
        MirageError, TransactionRequest,
        fork::{
            ClassificationConfig, DiffClassifier, EvmExecutor, ForkState, HybridDB, MirageFork,
            WatchEntry, WatchSource, with_state_write,
        },
        integration::MIRAGE_WATCH_CONTRACT_METHOD,
        provider::UpstreamRpc,
        resources::{MirageMode, PressureAction, Profile, ResourceModel},
        scenario::{JobStatus, Scenario, ScenarioJob},
    };

    fn test_rpc_module() -> (jsonrpsee::RpcModule<ServerContext>, ServerContext) {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let (shutdown, _) = broadcast::channel(1);
        let context = ServerContext {
            state: mirage.state(),
            shutdown,
            #[cfg(feature = "chain")]
            chain: None,
            #[cfg(feature = "roko")]
            chain_subs: None,
        };
        let module = build_rpc_module(context.clone())
            .unwrap_or_else(|error| panic!("build rpc module: {error}"));
        (module, context)
    }

    fn abi_word_from_u64(value: u64) -> [u8; 32] {
        let mut word = [0_u8; 32];
        word[24..].copy_from_slice(&value.to_be_bytes());
        word
    }

    fn abi_word_from_address(address: Address) -> [u8; 32] {
        let mut word = [0_u8; 32];
        word[12..].copy_from_slice(address.as_slice());
        word
    }

    fn selector(signature: &str) -> [u8; 4] {
        let hash = keccak256(signature.as_bytes());
        let mut selector = [0_u8; 4];
        selector.copy_from_slice(&hash[..4]);
        selector
    }

    fn encode_register_passport(owner: Address, capability_bitmask: u64) -> Bytes {
        let mut data = Vec::with_capacity(4 + (32 * 5));
        data.extend_from_slice(&selector(
            "registerPassport(address,uint64,bytes32,bytes32,uint64)",
        ));
        data.extend_from_slice(&abi_word_from_address(owner));
        data.extend_from_slice(&abi_word_from_u64(capability_bitmask));
        data.extend_from_slice(B256::ZERO.as_slice());
        data.extend_from_slice(B256::ZERO.as_slice());
        data.extend_from_slice(&abi_word_from_u64(0));
        Bytes::from(data)
    }

    fn encode_registered_count() -> Bytes {
        Bytes::from(selector("registeredCount()").to_vec())
    }

    fn decode_u256(output: &Bytes) -> U256 {
        let mut word = [0_u8; 32];
        word.copy_from_slice(&output[..32]);
        U256::from_be_bytes(word)
    }

    #[test]
    fn test_rpc_error_codes_match_plan_table() {
        let cases = [
            (MirageError::SnapshotNotFound(1), -32001),
            (MirageError::InvalidFrom(Address::ZERO), -32010),
            (MirageError::SlotDetectionFailed(Address::ZERO), -32020),
            (MirageError::WatchListFull, -32030),
            (MirageError::UnknownProtocolType("x".to_owned()), -32040),
            (MirageError::SetNotFound("set".to_owned()), -32050),
            (MirageError::SetAlreadyRunning("set".to_owned()), -32051),
            (MirageError::SetHasNoScenarios("set".to_owned()), -32052),
            (MirageError::JobNotFound("job".to_owned()), -32054),
            (MirageError::JobNotComplete("job".to_owned()), -32055),
            (MirageError::Upstream("err".to_owned()), -32099),
        ];

        for (error, expected_code) in cases {
            assert_eq!(rpc_error(error).code(), expected_code);
        }
    }

    #[tokio::test]
    async fn scenario_run_empty_set_returns_minus_32052() {
        use jsonrpsee::core::server::MethodsError;

        let (module, _context) = test_rpc_module();
        let set_id: String = module
            .call("mirage_beginScenarioSet", ("latest",))
            .await
            .expect("begin scenario set");
        let err = module
            .call::<_, String>("mirage_runScenarioSet", (set_id, "sequential"))
            .await
            .expect_err("run without scenarios");
        match err {
            MethodsError::JsonRpc(obj) => assert_eq!(obj.code(), -32052),
            other => panic!("expected JSON-RPC error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn scenario_define_after_set_completes_returns_minus_32051() {
        use jsonrpsee::core::server::MethodsError;
        use std::time::Duration;

        let (module, _context) = test_rpc_module();
        let set_id: String = module
            .call("mirage_beginScenarioSet", ("latest",))
            .await
            .expect("begin scenario set");
        let scenario = Scenario {
            id: "s1".to_owned(),
            name: "noop".to_owned(),
            transactions: Vec::new(),
            track_addresses: Vec::new(),
            max_gas: None,
            timeout: Duration::from_secs(10),
            assertions: Default::default(),
        };
        module
            .call::<_, String>("mirage_defineScenario", (set_id.clone(), scenario))
            .await
            .expect("define scenario");
        let job_id: String = module
            .call("mirage_runScenarioSet", (set_id.clone(), "sequential"))
            .await
            .expect("run scenario set");
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let job: ScenarioJob = module
                .call("mirage_getScenarioResults", (job_id.clone(),))
                .await
                .expect("job status");
            if job.status == JobStatus::Complete {
                break;
            }
        }
        let second = Scenario {
            id: "s2".to_owned(),
            name: "late".to_owned(),
            transactions: Vec::new(),
            track_addresses: Vec::new(),
            max_gas: None,
            timeout: Duration::from_secs(10),
            assertions: Default::default(),
        };
        let err = module
            .call::<_, String>("mirage_defineScenario", (set_id, second))
            .await
            .expect_err("define after set left draft");
        match err {
            MethodsError::JsonRpc(obj) => assert_eq!(obj.code(), -32051),
            other => panic!("expected JSON-RPC error, got {other:?}"),
        }
    }

    #[test]
    fn build_rpc_module_registers_required_eth_methods() {
        let (module, _context) = test_rpc_module();

        for method in [
            "eth_blockNumber",
            "eth_chainId",
            "eth_getBalance",
            "eth_getStorageAt",
            "eth_getCode",
            "eth_getTransactionCount",
            "eth_call",
            "eth_sendTransaction",
            "eth_sendRawTransaction",
            "eth_getTransactionReceipt",
            "eth_getTransactionByHash",
            "eth_getLogs",
            "eth_getBlockByNumber",
            "eth_getBlockByHash",
            "eth_estimateGas",
            "eth_gasPrice",
            "eth_feeHistory",
            "eth_maxPriorityFeePerGas",
        ] {
            assert!(
                module.method(method).is_some(),
                "{method} should be registered"
            );
        }
    }

    #[test]
    fn hardhat_mine_count_parses_hex_string_or_json_number() {
        assert_eq!(super::parse_mine_block_count(&[]), 1);
        assert_eq!(
            super::parse_mine_block_count(&[serde_json::json!("0x4")]),
            4
        );
        assert_eq!(super::parse_mine_block_count(&[serde_json::json!(7)]), 7);
    }

    #[test]
    fn eth_fee_history_response_matches_block_count() {
        let value = super::build_fee_history_response(
            serde_json::json!(3),
            serde_json::json!("latest"),
            Some(serde_json::json!([25, 75])),
        )
        .expect("fee history");
        assert_eq!(value["baseFeePerGas"].as_array().expect("baseFee").len(), 4);
        assert_eq!(value["gasUsedRatio"].as_array().expect("ratio").len(), 3);
        let reward = value["reward"].as_array().expect("reward");
        assert_eq!(reward.len(), 3);
        assert_eq!(reward[0].as_array().expect("tier").len(), 2);
    }

    #[test]
    fn build_rpc_module_registers_required_hardhat_anvil_methods() {
        let (module, _context) = test_rpc_module();

        for method in [
            "hardhat_impersonateAccount",
            "anvil_impersonateAccount",
            "hardhat_stopImpersonatingAccount",
            "anvil_stopImpersonatingAccount",
            "hardhat_setBalance",
            "anvil_setBalance",
            "hardhat_setStorageAt",
            "anvil_setStorageAt",
            "hardhat_setCode",
            "anvil_setCode",
            "hardhat_setNonce",
            "anvil_setNonce",
            "hardhat_mine",
            "anvil_mine",
            "hardhat_reset",
            "anvil_reset",
            "hardhat_setNextBlockBaseFeePerGas",
            "anvil_setNextBlockBaseFeePerGas",
            "hardhat_setCoinbase",
            "anvil_setCoinbase",
            "hardhat_setPrevRandao",
            "anvil_setPrevRandao",
        ] {
            assert!(
                module.method(method).is_some(),
                "{method} should be registered"
            );
        }
    }

    #[test]
    fn ensure_erc8004_boot_contracts_seeds_all_boot_addresses() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let mut fork = ForkState::new(db, 0, 1);

        let installed = ensure_erc8004_boot_contracts(&mut fork).expect("seed erc8004");
        assert_eq!(installed.len(), 6);

        for address in [
            super::ERC8004_IDENTITY_REGISTRY,
            super::ERC8004_REPUTATION_REGISTRY,
            super::ERC8004_VALIDATION_REGISTRY,
            super::MIRAGE_IDENTITY_REGISTRY_ALIAS,
            super::MIRAGE_REPUTATION_REGISTRY_ALIAS,
            super::MIRAGE_VALIDATION_REGISTRY_ALIAS,
        ] {
            assert!(
                super::account_has_code(&mut fork, address).expect("contract code check"),
                "{address} should carry boot code"
            );
            let nonce = fork
                .db
                .basic(address)
                .expect("account read")
                .expect("boot account")
                .nonce;
            assert_eq!(nonce, 1, "{address} should look deployed");
        }
    }

    #[test]
    #[ignore = "boot bytecode is runtime-only (no constructor); storage not initialized for registerPassport"]
    fn local_identity_alias_delegates_to_canonical_registry() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let mut fork = ForkState::new(db, 0, 1);
        ensure_erc8004_boot_contracts(&mut fork).expect("seed erc8004");

        let caller = address!("0x6100000000000000000000000000000000000007");
        fork.db.set_balance(caller, U256::from(10_u128.pow(18)));

        // Register directly on the canonical contract (no proxy indirection).
        let (register_result, _diff) = EvmExecutor::transact(
            &mut fork,
            caller,
            Some(ERC8004_IDENTITY_REGISTRY),
            encode_register_passport(caller, 0b101),
            U256::ZERO,
            500_000,
        )
        .expect("register via canonical");
        assert!(
            register_result.success,
            "canonical registration should succeed"
        );

        let canonical_count = EvmExecutor::call(
            &fork,
            caller,
            ERC8004_IDENTITY_REGISTRY,
            encode_registered_count(),
            U256::ZERO,
            100_000,
        )
        .expect("canonical registeredCount");
        assert_eq!(decode_u256(&canonical_count.output), U256::from(1_u64));

        // The alias proxy delegates reads to the canonical contract, so it should
        // report the same count.
        let alias_count = EvmExecutor::call(
            &fork,
            caller,
            MIRAGE_IDENTITY_REGISTRY_ALIAS,
            encode_registered_count(),
            U256::ZERO,
            100_000,
        )
        .expect("alias registeredCount");
        assert_eq!(decode_u256(&alias_count.output), U256::from(1_u64));
    }

    #[test]
    fn build_rpc_module_registers_required_evm_and_mirage_methods() {
        let (module, _context) = test_rpc_module();

        for method in [
            "evm_snapshot",
            "evm_revert",
            "evm_mine",
            "evm_increaseTime",
            "evm_setNextBlockTimestamp",
            "mirage_setBalance",
            "mirage_setCode",
            "mirage_setStorageAt",
            "mirage_mintERC20",
            "mirage_prefetchSlots",
            "mirage_prefetchAccount",
            "mirage_watchContract",
            "mirage_unwatchContract",
            "mirage_getWatchList",
            "mirage_getDirtySlots",
            "mirage_getLastStateDiff",
            "mirage_status",
            "mirage_getResourceUsage",
            "mirage_setResourceLimits",
            "mirage_getPosition",
            "mirage_subscribeEvents",
            "mirage_beginScenarioSet",
            "mirage_defineScenario",
            "mirage_runScenarioSet",
            "mirage_getScenarioResults",
            "mirage_compareScenarios",
            "mirage_computeDomainSeparator",
            "mirage_cleanup",
            "mirage_shutdown",
        ] {
            assert!(
                module.method(method).is_some(),
                "{method} should be registered"
            );
        }
    }

    #[tokio::test]
    async fn test_account_impersonation_validity() {
        let (module, context) = test_rpc_module();
        let sender = address!("0x6100000000000000000000000000000000000001");
        let receiver = address!("0x6100000000000000000000000000000000000002");
        let target = address!("0x6100000000000000000000000000000000000003");
        let storage_slot = U256::from(7_u64);
        let hardhat_balance = U256::from(42_u64);
        let anvil_balance = U256::from(99_u64);
        let hardhat_nonce = 7_u64;
        let anvil_nonce = 11_u64;
        let hardhat_code = Bytes::from_static(&[0x60, 0x01, 0x60, 0x00, 0x55]);
        let anvil_code = Bytes::from_static(&[0x60, 0x02, 0x60, 0x00, 0x55]);
        let hardhat_base_fee = 123_u128;
        let anvil_base_fee = 456_u128;
        let hardhat_coinbase = address!("0x6100000000000000000000000000000000000004");
        let anvil_coinbase = address!("0x6100000000000000000000000000000000000005");
        let hardhat_prev_randao = B256::from([0x11; 32]);
        let anvil_prev_randao = B256::from([0x22; 32]);

        assert!(
            module
                .call::<_, bool>("hardhat_impersonateAccount", (sender,))
                .await
                .unwrap_or_else(|error| panic!("hardhat impersonation succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_impersonateAccount", (receiver,))
                .await
                .unwrap_or_else(|error| panic!("anvil impersonation succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert!(state.fork.impersonated_accounts.contains(&sender));
            assert!(state.fork.impersonated_accounts.contains(&receiver));
        }

        let sender_baseline_balance: String = module
            .call("eth_getBalance", (sender, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read sender baseline balance: {error}"));
        let receiver_baseline_balance: String = module
            .call("eth_getBalance", (receiver, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read receiver baseline balance: {error}"));

        assert!(
            module
                .call::<_, bool>("hardhat_setBalance", (sender, hardhat_balance))
                .await
                .unwrap_or_else(|error| panic!("hardhat setBalance succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_setBalance", (receiver, anvil_balance))
                .await
                .unwrap_or_else(|error| panic!("anvil setBalance succeeds: {error}"))
        );
        let sender_balance: String = module
            .call("eth_getBalance", (sender, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read sender balance: {error}"));
        let receiver_balance: String = module
            .call("eth_getBalance", (receiver, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read receiver balance: {error}"));
        assert_eq!(sender_balance, format!("0x{:x}", hardhat_balance));
        assert_eq!(receiver_balance, format!("0x{:x}", anvil_balance));

        assert!(
            module
                .call::<_, bool>(
                    "hardhat_setStorageAt",
                    (sender, storage_slot, hardhat_balance)
                )
                .await
                .unwrap_or_else(|error| panic!("hardhat setStorageAt succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>(
                    "anvil_setStorageAt",
                    (receiver, storage_slot, anvil_balance)
                )
                .await
                .unwrap_or_else(|error| panic!("anvil setStorageAt succeeds: {error}"))
        );
        let sender_storage: String = module
            .call("eth_getStorageAt", (sender, storage_slot, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read sender storage: {error}"));
        let receiver_storage: String = module
            .call("eth_getStorageAt", (receiver, storage_slot, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read receiver storage: {error}"));
        assert_eq!(sender_storage, format!("0x{:064x}", hardhat_balance));
        assert_eq!(receiver_storage, format!("0x{:064x}", anvil_balance));

        assert!(
            module
                .call::<_, bool>("hardhat_setCode", (sender, hardhat_code.clone()))
                .await
                .unwrap_or_else(|error| panic!("hardhat setCode succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_setCode", (receiver, anvil_code.clone()))
                .await
                .unwrap_or_else(|error| panic!("anvil setCode succeeds: {error}"))
        );
        let sender_code: String = module
            .call("eth_getCode", (sender, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read sender code: {error}"));
        let receiver_code: String = module
            .call("eth_getCode", (receiver, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read receiver code: {error}"));
        assert_eq!(sender_code, "0x6001600055");
        assert_eq!(receiver_code, "0x6002600055");

        assert!(
            module
                .call::<_, bool>("hardhat_setNonce", (sender, hardhat_nonce))
                .await
                .unwrap_or_else(|error| panic!("hardhat setNonce succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_setNonce", (receiver, anvil_nonce))
                .await
                .unwrap_or_else(|error| panic!("anvil setNonce succeeds: {error}"))
        );
        let sender_nonce: String = module
            .call("eth_getTransactionCount", (sender, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read sender nonce: {error}"));
        let receiver_nonce: String = module
            .call("eth_getTransactionCount", (receiver, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read receiver nonce: {error}"));
        assert_eq!(sender_nonce, format!("0x{hardhat_nonce:x}"));
        assert_eq!(receiver_nonce, format!("0x{anvil_nonce:x}"));

        assert!(
            module
                .call::<_, bool>("hardhat_setNextBlockBaseFeePerGas", (hardhat_base_fee,))
                .await
                .unwrap_or_else(|error| panic!("hardhat base fee succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_setNextBlockBaseFeePerGas", (anvil_base_fee,))
                .await
                .unwrap_or_else(|error| panic!("anvil base fee succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert_eq!(state.fork.next_base_fee_per_gas, anvil_base_fee);
        }

        assert!(
            module
                .call::<_, bool>("hardhat_setCoinbase", (hardhat_coinbase,))
                .await
                .unwrap_or_else(|error| panic!("hardhat coinbase succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert_eq!(state.fork.coinbase, hardhat_coinbase);
        }
        assert!(
            module
                .call::<_, bool>("anvil_setCoinbase", (anvil_coinbase,))
                .await
                .unwrap_or_else(|error| panic!("anvil coinbase succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert_eq!(state.fork.coinbase, anvil_coinbase);
        }

        assert!(
            module
                .call::<_, bool>("hardhat_setPrevRandao", (hardhat_prev_randao,))
                .await
                .unwrap_or_else(|error| panic!("hardhat prevRandao succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert_eq!(state.fork.prev_randao, hardhat_prev_randao);
        }
        assert!(
            module
                .call::<_, bool>("anvil_setPrevRandao", (anvil_prev_randao,))
                .await
                .unwrap_or_else(|error| panic!("anvil prevRandao succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert_eq!(state.fork.prev_randao, anvil_prev_randao);
        }

        let block_before: u64 = parse_hex_quantity(
            &module
                .call::<_, String>("eth_blockNumber", Vec::<u8>::new())
                .await
                .unwrap_or_else(|error| panic!("read block number before mine: {error}")),
        )
        .unwrap_or_else(|error| panic!("parse block number before mine: {error}"));
        assert!(
            module
                .call::<_, bool>("hardhat_mine", ("0x2",))
                .await
                .unwrap_or_else(|error| panic!("hardhat mine succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_mine", ("0x3",))
                .await
                .unwrap_or_else(|error| panic!("anvil mine succeeds: {error}"))
        );
        let block_after: u64 = parse_hex_quantity(
            &module
                .call::<_, String>("eth_blockNumber", Vec::<u8>::new())
                .await
                .unwrap_or_else(|error| panic!("read block number after mine: {error}")),
        )
        .unwrap_or_else(|error| panic!("parse block number after mine: {error}"));
        assert_eq!(block_after, block_before + 5);

        let tx_hash: B256 = module
            .call(
                "eth_sendTransaction",
                (TransactionRequest {
                    from: Some(sender),
                    to: Some(target),
                    gas: Some(21_000),
                    value: Some(U256::ZERO),
                    data: None,
                    gas_price: None,
                    nonce: None,
                    chain_id: None,
                },),
            )
            .await
            .unwrap_or_else(|error| panic!("impersonated transaction succeeds: {error}"));
        assert_ne!(tx_hash, B256::ZERO);

        assert!(
            module
                .call::<_, bool>("hardhat_stopImpersonatingAccount", (sender,))
                .await
                .unwrap_or_else(|error| panic!("hardhat stop impersonation succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_stopImpersonatingAccount", (receiver,))
                .await
                .unwrap_or_else(|error| panic!("anvil stop impersonation succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert!(state.fork.impersonated_accounts.is_empty());
        }

        assert!(
            module
                .call::<_, bool>(MIRAGE_WATCH_CONTRACT_METHOD, (target,))
                .await
                .unwrap_or_else(|error| panic!("mirage watch contract succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert!(state.fork.db.dirty.watch_list.contains_key(&target));
        }

        assert!(
            module
                .call::<_, bool>("hardhat_reset", Vec::<u8>::new())
                .await
                .unwrap_or_else(|error| panic!("hardhat reset succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert!(
                state.fork.db.dirty.watch_list.is_empty(),
                "reset clears watch list"
            );
            assert!(
                state.fork.db.dirty.unwatch_list.is_empty(),
                "reset clears unwatch list"
            );
            assert!(
                state.fork.impersonated_accounts.is_empty(),
                "reset clears impersonation set"
            );
            assert!(
                super::account_has_code(&mut state.fork.clone(), super::ERC8004_IDENTITY_REGISTRY)
                    .expect("identity code after hardhat reset"),
                "hardhat reset should preserve ERC-8004 boot contracts"
            );
        }
        assert_eq!(
            module
                .call::<_, String>("eth_getBalance", (sender, "latest"))
                .await
                .unwrap_or_else(|error| panic!("balance after hardhat reset: {error}")),
            sender_baseline_balance
        );
        assert_eq!(
            module
                .call::<_, String>("eth_getCode", (sender, "latest"))
                .await
                .unwrap_or_else(|error| panic!("code after hardhat reset: {error}")),
            "0x"
        );
        assert_eq!(
            module
                .call::<_, String>("eth_getStorageAt", (sender, storage_slot, "latest"))
                .await
                .unwrap_or_else(|error| panic!("storage after hardhat reset: {error}")),
            format!("0x{:064x}", U256::ZERO)
        );
        assert_eq!(
            module
                .call::<_, String>("eth_getTransactionCount", (sender, "latest"))
                .await
                .unwrap_or_else(|error| panic!("nonce after hardhat reset: {error}")),
            "0x0"
        );

        assert!(
            module
                .call::<_, bool>("anvil_setBalance", (receiver, anvil_balance))
                .await
                .unwrap_or_else(|error| panic!("reapply receiver balance: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_reset", Vec::<u8>::new())
                .await
                .unwrap_or_else(|error| panic!("anvil reset succeeds: {error}"))
        );
        assert_eq!(
            module
                .call::<_, String>("eth_getBalance", (receiver, "latest"))
                .await
                .unwrap_or_else(|error| panic!("balance after anvil reset: {error}")),
            receiver_baseline_balance
        );
        let identity_code_after_anvil_reset: String = module
            .call("eth_getCode", (super::ERC8004_IDENTITY_REGISTRY, "latest"))
            .await
            .unwrap_or_else(|error| panic!("identity code after anvil reset: {error}"));
        assert_ne!(identity_code_after_anvil_reset, "0x");
    }

    #[test]
    fn decode_raw_legacy_transaction() {
        let signing_key = SigningKey::from_bytes((&[7_u8; 32]).into())
            .unwrap_or_else(|error| panic!("signing key: {error}"));
        let from = signing_key_address(&signing_key);
        let to = address!("0x1000000000000000000000000000000000000001");
        let raw = sign_legacy(&signing_key, 1, 5, 21_000, Some(to), 9, &[0xde, 0xad]);
        let decoded = decode_signed_raw_transaction(&raw)
            .unwrap_or_else(|error| panic!("decode legacy: {error}"));
        assert_transaction(
            decoded.request,
            from,
            Some(to),
            21_000,
            9,
            &[0xde, 0xad],
            Some(1),
            Some(5),
        );
    }

    #[test]
    fn decode_raw_typed_transactions() {
        let signing_key = SigningKey::from_bytes((&[9_u8; 32]).into())
            .unwrap_or_else(|error| panic!("signing key: {error}"));
        let from = signing_key_address(&signing_key);
        let to = address!("0x2000000000000000000000000000000000000002");

        let type1 = sign_typed(&signing_key, 0x01, Some(to));
        let decoded1 = decode_signed_raw_transaction(&type1)
            .unwrap_or_else(|error| panic!("decode type1: {error}"));
        assert_transaction(
            decoded1.request,
            from,
            Some(to),
            80_000,
            11,
            &[0x01, 0x02],
            Some(1),
            Some(3),
        );

        let type2 = sign_typed(&signing_key, 0x02, Some(to));
        let decoded2 = decode_signed_raw_transaction(&type2)
            .unwrap_or_else(|error| panic!("decode type2: {error}"));
        assert_transaction(
            decoded2.request,
            from,
            Some(to),
            80_000,
            11,
            &[0x01, 0x02],
            Some(1),
            Some(4),
        );

        let type3 = sign_typed(&signing_key, 0x03, None);
        let decoded3 = decode_signed_raw_transaction(&type3)
            .unwrap_or_else(|error| panic!("decode type3: {error}"));
        assert_transaction(
            decoded3.request,
            from,
            None,
            80_000,
            11,
            &[0x01, 0x02],
            Some(1),
            Some(4),
        );
    }

    #[test]
    fn sqrt_price_x96_round_trip_matches_expected_price() {
        let price = 1_800.0;
        let encoded = to_sqrt_price_x96(price);
        let decoded = from_sqrt_price_x96(encoded);
        let relative_error = (decoded - price).abs() / price;
        assert!(
            relative_error < 1e-9,
            "decoded={decoded} price={price} relative_error={relative_error}"
        );
    }

    #[tokio::test]
    async fn run_fork_snapshot_reads_dirty_state_without_holding_server_lock() {
        let address = address!("0x3000000000000000000000000000000000000003");
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let (shutdown, _) = broadcast::channel(1);
        let context = ServerContext {
            state: mirage.state(),
            shutdown,
            #[cfg(feature = "chain")]
            chain: None,
            #[cfg(feature = "roko")]
            chain_subs: None,
        };
        context
            .state
            .write()
            .fork
            .db
            .set_balance(address, U256::from(42_u64));

        let observed = run_fork_snapshot(&context.state, true, move |mut fork| {
            Ok(fork.db.basic(address)?.unwrap_or_default().balance)
        })
        .await
        .unwrap_or_else(|error| panic!("snapshot read succeeds: {error}"));

        assert_eq!(observed, U256::from(42_u64));
    }

    #[tokio::test]
    async fn commit_transaction_request_runs_without_holding_state_write_lock() {
        let from = address!("0x3100000000000000000000000000000000000001");
        let to = address!("0x3100000000000000000000000000000000000002");
        let upstream = Arc::new(UpstreamRpc::mock(1));
        upstream.set_mock_delay(Duration::from_millis(150));
        let db = HybridDB::new(
            Arc::clone(&upstream),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );

        let state = mirage.state();
        let request = TransactionRequest {
            from: Some(from),
            to: Some(to),
            gas: Some(21_000),
            value: Some(U256::from(1_u64)),
            data: None,
            gas_price: None,
            nonce: None,
            chain_id: None,
        };

        let state_for_task = Arc::clone(&state);
        let task = tokio::spawn(async move {
            commit_transaction_request(&state_for_task, request, None).await
        });
        sleep(Duration::from_millis(20)).await;

        let started = std::time::Instant::now();
        let read_guard = state.read();
        assert!(started.elapsed() < Duration::from_millis(75));
        drop(read_guard);

        let tx_hash = task
            .await
            .unwrap_or_else(|error| panic!("join tx task: {error}"))
            .unwrap_or_else(|error| panic!("commit transaction: {error}"));
        assert_ne!(tx_hash, B256::ZERO);
    }

    #[tokio::test]
    async fn commit_transaction_request_releases_writer_gate_during_blocking_execution() {
        let from = address!("0x3200000000000000000000000000000000000001");
        let to = address!("0x3200000000000000000000000000000000000002");
        let upstream = Arc::new(UpstreamRpc::mock(1));
        upstream.set_mock_delay(Duration::from_millis(150));
        let db = HybridDB::new(
            Arc::clone(&upstream),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );

        let state = mirage.state();
        let request = TransactionRequest {
            from: Some(from),
            to: Some(to),
            gas: Some(21_000),
            value: Some(U256::from(1_u64)),
            data: None,
            gas_price: None,
            nonce: None,
            chain_id: None,
        };

        let state_for_task = Arc::clone(&state);
        let task = tokio::spawn(async move {
            commit_transaction_request(&state_for_task, request, None).await
        });
        sleep(Duration::from_millis(20)).await;

        let started = std::time::Instant::now();
        with_state_write(&state, |state| {
            state.reject_new_forks = !state.reject_new_forks;
        })
        .await;
        assert!(started.elapsed() < Duration::from_millis(75));

        let tx_hash = task
            .await
            .unwrap_or_else(|error| panic!("join tx task: {error}"))
            .unwrap_or_else(|error| panic!("commit transaction: {error}"));
        assert_ne!(tx_hash, B256::ZERO);
    }

    #[tokio::test]
    async fn stage_erc20_mint_releases_writer_gate_during_blocking_reads() {
        let token = address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let owner = address!("0x3300000000000000000000000000000000000002");
        let upstream = Arc::new(UpstreamRpc::mock(1));
        upstream.set_mock_delay(Duration::from_millis(150));
        let db = HybridDB::new(
            Arc::clone(&upstream),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );

        let state = mirage.state();
        let state_for_task = Arc::clone(&state);
        let task = tokio::spawn(async move {
            stage_erc20_mint(&state_for_task, token, owner, U256::from(5_u64)).await
        });
        sleep(Duration::from_millis(20)).await;

        let started = std::time::Instant::now();
        with_state_write(&state, |state| {
            state.reject_new_forks = !state.reject_new_forks;
        })
        .await;
        assert!(started.elapsed() < Duration::from_millis(75));

        let staged = task
            .await
            .unwrap_or_else(|error| panic!("join mint task: {error}"))
            .unwrap_or_else(|error| panic!("stage mint: {error}"));
        assert!(!staged.storage_writes.is_empty());
    }

    #[test]
    fn throttle_pressure_demotes_new_contracts_to_slot_only() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let state_handle = mirage.state();
        let mut state = state_handle.write();
        let manual = address!("0x4000000000000000000000000000000000000004");
        let auto = address!("0x5000000000000000000000000000000000000005");
        state.fork.db.dirty.watch_list.insert(
            manual,
            WatchEntry {
                source: WatchSource::Manual,
                added_at_block: 0,
                initial_slot_count: 0,
                replay_count: 0,
            },
        );
        state.fork.db.dirty.watch_list.insert(
            auto,
            WatchEntry {
                source: WatchSource::AutoClassified,
                added_at_block: 0,
                initial_slot_count: 0,
                replay_count: 0,
            },
        );
        state.reject_new_forks = false;
        state.fork.db.dirty.demote_protocols_to_slot_only = false;

        let classifier = DiffClassifier::new(ClassificationConfig::default());
        let mut diff = crate::StateDiff::success(21_000, Bytes::default());
        diff.accounts.insert(
            address!("0x4100000000000000000000000000000000000004"),
            crate::AccountDiff {
                info_changed: true,
                new_balance: None,
                new_nonce: None,
                new_code: None,
                storage_written: [
                    (U256::from(1), U256::from(1)),
                    (U256::from(2), U256::from(2)),
                    (U256::from(3), U256::from(3)),
                ]
                .into_iter()
                .collect(),
                storage_read: Default::default(),
            },
        );

        apply_pressure_action(&mut state, PressureAction::Throttle);
        classifier
            .apply(&mut state.fork.db.dirty, &diff, 1)
            .unwrap_or_else(|error| panic!("classifier apply succeeds: {error}"));

        assert!(!state.reject_new_forks);
        assert!(state.fork.db.dirty.demote_protocols_to_slot_only);
        assert!(state.fork.db.dirty.watch_list.contains_key(&manual));
        assert!(state.fork.db.dirty.watch_list.contains_key(&auto));
        assert!(
            !state
                .fork
                .db
                .dirty
                .watch_list
                .contains_key(&address!("0x4100000000000000000000000000000000000004"))
        );
        assert_eq!(state.mode, MirageMode::Live);
    }

    fn assert_transaction(
        request: TransactionRequest,
        from: Address,
        to: Option<Address>,
        gas: u64,
        value: u64,
        data: &[u8],
        chain_id: Option<u64>,
        gas_price: Option<u128>,
    ) {
        assert_eq!(request.from, Some(from));
        assert_eq!(request.to, to);
        assert_eq!(request.gas, Some(gas));
        assert_eq!(request.value, Some(U256::from(value)));
        assert_eq!(request.data.as_ref().map(Bytes::as_ref), Some(data));
        assert_eq!(request.chain_id, chain_id);
        assert_eq!(request.gas_price, gas_price);
    }

    fn signing_key_address(signing_key: &SigningKey) -> Address {
        let encoded = signing_key.verifying_key().to_encoded_point(false);
        let hash = keccak256(&encoded.as_bytes()[1..]);
        Address::from_slice(&hash.as_slice()[12..])
    }

    fn sign_legacy(
        signing_key: &SigningKey,
        chain_id: u64,
        gas_price: u64,
        gas_limit: u64,
        to: Option<Address>,
        value: u64,
        data: &[u8],
    ) -> Bytes {
        let unsigned = RlpValue::List(vec![
            rlp_from_u64(0),
            rlp_from_u64(gas_price),
            rlp_from_u64(gas_limit),
            to.map_or_else(
                || RlpValue::Bytes(Vec::new()),
                |value| RlpValue::Bytes(value.as_slice().to_vec()),
            ),
            rlp_from_u64(value),
            RlpValue::Bytes(data.to_vec()),
            rlp_from_u64(chain_id),
            RlpValue::Bytes(Vec::new()),
            RlpValue::Bytes(Vec::new()),
        ]);
        let unsigned_rlp = rlp_encode(&unsigned);
        let hash = keccak256(&unsigned_rlp);
        let mut field_bytes = FieldBytes::default();
        field_bytes.copy_from_slice(hash.as_slice());
        let (signature, recovery_id) = signing_key
            .as_nonzero_scalar()
            .try_sign_prehashed_rfc6979::<sha2::Sha256>(&field_bytes, &[])
            .unwrap_or_else(|error| panic!("sign legacy prehash: {error}"));
        let recovery_id = recovery_id.unwrap_or_else(|| panic!("legacy recovery id present"));
        let v = chain_id * 2 + 35 + u64::from(recovery_id.to_byte());
        let signed = RlpValue::List(vec![
            rlp_from_u64(0),
            rlp_from_u64(gas_price),
            rlp_from_u64(gas_limit),
            to.map_or_else(
                || RlpValue::Bytes(Vec::new()),
                |value| RlpValue::Bytes(value.as_slice().to_vec()),
            ),
            rlp_from_u64(value),
            RlpValue::Bytes(data.to_vec()),
            rlp_from_u64(v),
            RlpValue::Bytes(signature.r().to_bytes().to_vec()),
            RlpValue::Bytes(signature.s().to_bytes().to_vec()),
        ]);
        Bytes::from(rlp_encode(&signed))
    }

    #[tokio::test]
    async fn evm_snapshot_captures_dirty_store_and_revert_is_single_use() {
        let addr = address!("0x6000000000000000000000000000000000000006");
        let (module, _context) = test_rpc_module();

        assert!(
            module
                .call::<_, bool>("mirage_setBalance", (addr, U256::from(100_u64)))
                .await
                .unwrap_or_else(|error| panic!("set initial balance: {error}"))
        );

        let snapshot_raw: String = module
            .call("evm_snapshot", Vec::<u8>::new())
            .await
            .unwrap_or_else(|error| panic!("take snapshot via rpc: {error}"));
        let snapshot_id = parse_hex_quantity(&snapshot_raw)
            .unwrap_or_else(|error| panic!("parse snapshot id {snapshot_raw}: {error}"));

        assert!(
            module
                .call::<_, bool>("mirage_setBalance", (addr, U256::from(999_u64)))
                .await
                .unwrap_or_else(|error| panic!("set modified balance: {error}"))
        );

        let balance_after_modify: String = module
            .call("eth_getBalance", (addr, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read modified balance via rpc: {error}"));
        assert_eq!(balance_after_modify, format!("0x{:x}", U256::from(999_u64)));

        let reverted: bool = module
            .call("evm_revert", (format!("0x{snapshot_id:x}"),))
            .await
            .unwrap_or_else(|error| panic!("revert snapshot via rpc: {error}"));
        assert!(reverted);

        let balance_after_revert: String = module
            .call("eth_getBalance", (addr, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read reverted balance via rpc: {error}"));
        assert_eq!(balance_after_revert, format!("0x{:x}", U256::from(100_u64)));

        let second_revert = module
            .call::<_, bool>("evm_revert", (format!("0x{snapshot_id:x}"),))
            .await
            .unwrap_err();
        let second_revert_message = second_revert.to_string();
        assert!(
            second_revert_message.contains("-32001")
                || second_revert_message.contains("snapshot not found"),
            "expected snapshot-not-found error, got: {second_revert_message}"
        );
    }

    fn sign_typed(signing_key: &SigningKey, tx_type: u8, to: Option<Address>) -> Bytes {
        let unsigned = match tx_type {
            0x01 => RlpValue::List(vec![
                rlp_from_u64(1),
                rlp_from_u64(0),
                rlp_from_u64(3),
                rlp_from_u64(80_000),
                to.map_or_else(
                    || RlpValue::Bytes(Vec::new()),
                    |value| RlpValue::Bytes(value.as_slice().to_vec()),
                ),
                rlp_from_u64(11),
                RlpValue::Bytes(vec![0x01, 0x02]),
                RlpValue::List(Vec::new()),
            ]),
            0x02 => RlpValue::List(vec![
                rlp_from_u64(1),
                rlp_from_u64(0),
                rlp_from_u64(1),
                rlp_from_u64(4),
                rlp_from_u64(80_000),
                to.map_or_else(
                    || RlpValue::Bytes(Vec::new()),
                    |value| RlpValue::Bytes(value.as_slice().to_vec()),
                ),
                rlp_from_u64(11),
                RlpValue::Bytes(vec![0x01, 0x02]),
                RlpValue::List(Vec::new()),
            ]),
            0x03 => RlpValue::List(vec![
                rlp_from_u64(1),
                rlp_from_u64(0),
                rlp_from_u64(1),
                rlp_from_u64(4),
                rlp_from_u64(80_000),
                to.map_or_else(
                    || RlpValue::Bytes(Vec::new()),
                    |value| RlpValue::Bytes(value.as_slice().to_vec()),
                ),
                rlp_from_u64(11),
                RlpValue::Bytes(vec![0x01, 0x02]),
                RlpValue::List(Vec::new()),
                rlp_from_u64(5),
                RlpValue::List(Vec::new()),
            ]),
            _ => panic!("unsupported tx type"),
        };
        let unsigned_rlp = rlp_encode(&unsigned);
        let mut payload = vec![tx_type];
        payload.extend_from_slice(&unsigned_rlp);
        let hash = keccak256(payload);
        let mut field_bytes = FieldBytes::default();
        field_bytes.copy_from_slice(hash.as_slice());
        let (signature, recovery_id) = signing_key
            .as_nonzero_scalar()
            .try_sign_prehashed_rfc6979::<sha2::Sha256>(&field_bytes, &[])
            .unwrap_or_else(|error| panic!("sign typed prehash: {error}"));
        let recovery_id = recovery_id.unwrap_or_else(|| panic!("typed recovery id present"));

        let mut fields = match unsigned {
            RlpValue::List(fields) => fields,
            _ => unreachable!(),
        };
        fields.push(rlp_from_u64(u64::from(recovery_id.to_byte())));
        fields.push(RlpValue::Bytes(signature.r().to_bytes().to_vec()));
        fields.push(RlpValue::Bytes(signature.s().to_bytes().to_vec()));
        let mut encoded = vec![tx_type];
        encoded.extend_from_slice(&rlp_encode(&RlpValue::List(fields)));
        Bytes::from(encoded)
    }

    #[test]
    fn sqrt_price_x96_round_trip() {
        // Verify that to_sqrt_price_x96 and from_sqrt_price_x96 round-trip within
        // the tolerance used by the integration test (INV-004, price = 1800.0).
        let price = 1800.0_f64;
        let encoded = to_sqrt_price_x96(price);
        assert!(
            !encoded.is_zero(),
            "encoded sqrtPriceX96 must be non-zero for price={price}"
        );

        let decoded = from_sqrt_price_x96(encoded);
        let rel_error = (decoded - price).abs() / price;
        assert!(
            rel_error < 1e-9,
            "round-trip relative error too large: decoded={decoded} price={price} rel_error={rel_error}"
        );
    }

    #[test]
    fn sqrt_price_x96_edge_cases() {
        // Zero input produces zero output.
        assert_eq!(to_sqrt_price_x96(0.0), U256::ZERO);
        assert_eq!(to_sqrt_price_x96(-1.0), U256::ZERO);
        assert_eq!(to_sqrt_price_x96(f64::NAN), U256::ZERO);
        assert_eq!(to_sqrt_price_x96(f64::INFINITY), U256::ZERO);
        assert_eq!(to_sqrt_price_x96(f64::NEG_INFINITY), U256::ZERO);

        // Zero sqrtPriceX96 decodes as 0.0.
        assert_eq!(from_sqrt_price_x96(U256::ZERO), 0.0);
    }

    #[tokio::test]
    async fn test_jsonrpc_unknown_method_returns_32601() {
        use jsonrpsee::core::server::MethodsError;

        let (module, _context) = test_rpc_module();
        assert!(
            module.method("eth_fooNonExistent").is_none(),
            "eth_fooNonExistent should not be registered"
        );
        let error = module
            .call::<_, serde_json::Value>("eth_fooNonExistent", Vec::<()>::new())
            .await
            .expect_err("unknown method should return an error");
        match error {
            MethodsError::JsonRpc(obj) => {
                assert_eq!(
                    obj.code(),
                    -32601,
                    "wire code must be JSON-RPC method not found"
                );
            }
            other => panic!("expected JSON-RPC error object, got {other:?}"),
        }
    }

    #[cfg(feature = "chain")]
    fn test_rpc_module_with_chain() -> (jsonrpsee::RpcModule<ServerContext>, ServerContext) {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let (shutdown, _) = broadcast::channel(1);
        let chain = Arc::new(parking_lot::RwLock::new(
            crate::chain_rpc::ChainContext::new(crate::chain_rpc::ChainToggles::default()),
        ));
        let context = ServerContext {
            state: mirage.state(),
            shutdown,
            chain: Some(chain),
            #[cfg(feature = "roko")]
            chain_subs: None,
        };
        let module = build_rpc_module(context.clone())
            .unwrap_or_else(|error| panic!("build rpc module with chain: {error}"));
        (module, context)
    }

    #[cfg(feature = "chain")]
    #[tokio::test]
    async fn chain_register_agent_rpc() {
        let (module, _context) = test_rpc_module_with_chain();
        let result: bool = module
            .call("chain_registerAgent", ("agent-1", "0xdead", "researcher"))
            .await
            .expect("register agent");
        assert!(result, "first registration should succeed");

        let duplicate: bool = module
            .call("chain_registerAgent", ("agent-1", "0xdead", "researcher"))
            .await
            .expect("duplicate register");
        assert!(!duplicate, "duplicate registration should return false");
    }

    #[cfg(feature = "chain")]
    #[tokio::test]
    async fn chain_agent_heartbeat_rpc() {
        let (module, _context) = test_rpc_module_with_chain();
        let _: bool = module
            .call("chain_registerAgent", ("agent-1", "0xdead", "worker"))
            .await
            .unwrap();

        let result: bool = module
            .call("chain_agentHeartbeat", ("agent-1",))
            .await
            .expect("heartbeat");
        assert!(result);

        let missing: bool = module
            .call("chain_agentHeartbeat", ("nonexistent",))
            .await
            .expect("heartbeat nonexistent");
        assert!(!missing);
    }

    #[cfg(feature = "chain")]
    #[tokio::test]
    async fn chain_agent_trace_rpc() {
        let (module, _context) = test_rpc_module_with_chain();
        let _: bool = module
            .call("chain_registerAgent", ("agent-1", "0xdead", "coder"))
            .await
            .unwrap();

        let result: bool = module
            .call(
                "chain_agentTrace",
                (
                    "agent-1",
                    "reason",
                    vec!["file.rs"],
                    "thinking about it",
                    "edit file",
                ),
            )
            .await
            .expect("trace");
        assert!(result);

        let missing: bool = module
            .call(
                "chain_agentTrace",
                (
                    "nonexistent",
                    "act",
                    Vec::<String>::new(),
                    "doing something",
                    "run cmd",
                ),
            )
            .await
            .expect("trace for missing agent");
        assert!(!missing);
    }

    #[cfg(feature = "chain")]
    #[tokio::test]
    async fn chain_agent_stats_rpc() {
        let (module, _context) = test_rpc_module_with_chain();
        let _: bool = module
            .call("chain_registerAgent", ("agent-1", "0xdead", "analyst"))
            .await
            .unwrap();

        let delta = crate::chain::AgentStats {
            confirmations_given: 5,
            challenges_given: 2,
            warnings_posted: 1,
            insights_posted: 3,
            tasks_completed: 0,
            tasks_failed: 0,
            delta_cycles: 10,
            total_cost_usd: 0.5,
            total_tokens: 1000,
        };
        let result: bool = module
            .call("chain_agentStats", ("agent-1", delta))
            .await
            .expect("stats update");
        assert!(result);

        // Verify stats were accumulated in the chain context.
        let chain = _context.chain.as_ref().expect("chain present");
        let guard = chain.read();
        let stats = guard
            .agent_registry
            .get_stats("agent-1")
            .expect("agent should exist");
        assert_eq!(stats.confirmations_given, 5);
        assert_eq!(stats.total_tokens, 1000);
    }

    #[cfg(feature = "chain")]
    #[tokio::test]
    async fn agent_http_endpoints_via_full_server() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let (shutdown, _) = broadcast::channel(4);
        let chain = Arc::new(parking_lot::RwLock::new(
            crate::chain_rpc::ChainContext::new(crate::chain_rpc::ChainToggles::default()),
        ));
        let (addr, _handle) =
            super::start_rpc_server_with_chain("127.0.0.1:0", mirage, shutdown, chain)
                .await
                .expect("start server");
        let url = format!("http://{addr}");
        let http = reqwest::Client::new();

        // Register an agent via JSON-RPC.
        let resp = http
            .post(&url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0", "id": 1,
                "method": "chain_registerAgent",
                "params": ["agent-1", "0xdead", "researcher"]
            }))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["result"], true);

        // Heartbeat.
        let resp = http
            .post(&url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0", "id": 2,
                "method": "chain_agentHeartbeat",
                "params": ["agent-1"]
            }))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["result"], true);

        // Add a trace.
        let resp = http
            .post(&url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0", "id": 3,
                "method": "chain_agentTrace",
                "params": ["agent-1", "retrieve", ["doc.md"], "reading docs", "read file"]
            }))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["result"], true);

        // GET /api/agents — list agents.
        let resp: serde_json::Value = http
            .get(format!("{url}/api/agents"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let agents = resp["items"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["id"], "agent-1");
        assert_eq!(agents[0]["role"], "researcher");

        // GET /api/agents/agent-1/trace
        let resp: serde_json::Value = http
            .get(format!("{url}/api/agents/agent-1/trace?limit=10&offset=0"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["total"], 1);
        assert_eq!(resp["items"].as_array().unwrap().len(), 1);

        // GET /api/agents/agent-1/heartbeat
        let resp: serde_json::Value = http
            .get(format!("{url}/api/agents/agent-1/heartbeat"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["agent_id"], "agent-1");
        assert!(resp.get("alive").is_some());

        // GET /api/agents/agent-1/stats
        let resp: serde_json::Value = http
            .get(format!("{url}/api/agents/agent-1/stats"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["agent_id"], "agent-1");
        assert_eq!(resp["registered_at"].as_u64().is_some(), true);

        // Non-existent agent returns error.
        let resp: serde_json::Value = http
            .get(format!("{url}/api/agents/nobody/stats"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["error"], "agent not found");
    }

    /// Verifies that WebSocket `eth_subscribe("newHeads")` receives events when
    /// blocks are mined via `evm_mine`.
    #[tokio::test]
    async fn ws_eth_subscribe_new_heads() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let (shutdown, _) = broadcast::channel(4);
        let (addr, _handle) = super::start_rpc_server("127.0.0.1:0", mirage, shutdown)
            .await
            .expect("start server");

        // Connect via WebSocket.
        let ws_url = format!("ws://{addr}");
        let (mut ws, _) = connect_async(&ws_url).await.expect("ws connect");

        // Send eth_subscribe("newHeads").
        let subscribe_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_subscribe",
            "params": ["newHeads"]
        });
        ws.send(WsMessage::Text(subscribe_req.to_string().into()))
            .await
            .expect("send subscribe");

        // Read the subscription confirmation (returns subscription id).
        let confirm = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("timeout waiting for subscribe confirmation")
            .expect("stream ended")
            .expect("ws error");
        let confirm: serde_json::Value =
            serde_json::from_str(confirm.to_text().expect("text frame")).expect("parse json");
        let sub_id = confirm["result"].clone();
        assert!(!sub_id.is_null(), "subscription id missing");

        // Mine a block via HTTP JSON-RPC to trigger newHeads broadcast.
        let http = reqwest::Client::new();
        let url = format!("http://{addr}");
        http.post(&url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "evm_mine",
                "params": []
            }))
            .send()
            .await
            .expect("evm_mine");

        // Read the newHeads event from the WS stream.
        let event = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("timeout waiting for newHeads event")
            .expect("stream ended")
            .expect("ws error");
        let event: serde_json::Value =
            serde_json::from_str(event.to_text().expect("text frame")).expect("parse json");
        assert_eq!(event["method"], "eth_subscription");
        assert_eq!(event["params"]["subscription"], sub_id);
        assert!(
            event["params"]["result"]["number"].is_string(),
            "expected hex block number in newHeads event"
        );

        // Unsubscribe.
        let unsub_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "eth_unsubscribe",
            "params": [sub_id]
        });
        ws.send(WsMessage::Text(unsub_req.to_string().into()))
            .await
            .expect("send unsubscribe");
        let unsub = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("timeout waiting for unsubscribe response")
            .expect("stream ended")
            .expect("ws error");
        let unsub: serde_json::Value =
            serde_json::from_str(unsub.to_text().expect("text frame")).expect("parse json");
        assert_eq!(unsub["result"], true);
    }
}
