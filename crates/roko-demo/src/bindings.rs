//! Typed alloy `sol!` bindings for the demo suite.
//!
//! Each scenario + its scripted spine imports these typed handles instead of
//! hand-encoding calldata. Kept in one module since `roko-demo` is the only
//! consumer today; promote to its own crate if another binary needs them.

use alloy::sol;

sol! {
    #[sol(rpc)]
    contract MockERC20 {
        function name() external view returns (string memory);
        function symbol() external view returns (string memory);
        function decimals() external view returns (uint8);
        function totalSupply() external view returns (uint256);
        function balanceOf(address account) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
        function allowance(address owner, address spender) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
        function transferFrom(address from, address to, uint256 amount) external returns (bool);
        function mint(address to, uint256 amount) external;
        event Transfer(address indexed from, address indexed to, uint256 value);
        event Approval(address indexed owner, address indexed spender, uint256 value);
    }

    #[sol(rpc)]
    contract AgentRegistry {
        function register(string calldata capabilities, bytes32 passportHash) external;
        function heartbeat() external;
        function isActive(address agent) external view returns (bool);
        function registeredCount() external view returns (uint256);
        function LIVENESS_WINDOW() external view returns (uint64);
        event AgentRegistered(address indexed agent, bytes32 passportHash, string capabilities);
        event AgentHeartbeat(address indexed agent, uint64 blockNumber);
    }

    #[sol(rpc)]
    contract WorkerRegistry {
        function register(uint256 amount) external;
        function bond(uint256 amount) external;
        function unbond(uint256 amount) external;
        function updateReputation(address worker, bool outcome) external;
        function slash(address worker, uint8 reasonCode, uint256 bpsAmount) external;
        function tier(address worker) external view returns (uint8);
        function canAccept(address worker, uint8 minTier) external view returns (bool);
        function reputationOf(address worker) external view returns (uint256);
        function getWorker(address worker) external view returns (
            uint256 bond,
            uint256 reputation,
            uint64 jobsCompleted,
            uint64 jobsSlashed,
            uint64 lastUpdated,
            bool exists
        );
        function registeredCount() external view returns (uint256);
        function registeredAt(uint256 index) external view returns (address);
        function authorized(address caller) external view returns (bool);
        function setAuthorized(address caller, bool allowed) external;
        function MIN_BOND() external view returns (uint256);
        function SLASH_QUALITY_REJECT() external view returns (uint8);
        event WorkerRegistered(address indexed worker, uint256 bond);
        event ReputationUpdated(address indexed worker, uint256 oldRep, uint256 newRep, bool outcome);
        event WorkerSlashed(address indexed worker, uint8 reasonCode, uint256 amount, uint256 newBond);
    }

    #[sol(rpc)]
    contract BountyMarket {
        function postJob(
            bytes32 specHash,
            uint256 bounty,
            uint64 deadline,
            uint8 minTier
        ) external returns (uint256);
        function assign(uint256 id, address worker) external;
        function submit(uint256 id, bytes32 resultHash) external;
        function resolve(uint256 id, bool accepted) external;
        function stateOf(uint256 id) external view returns (uint8);
        function nextJobId() external view returns (uint256);
        function setResolver(address newResolver) external;
        event JobOpen(uint256 indexed jobId, address indexed poster, uint256 bounty, uint64 deadline, bytes32 specHash);
        event JobFunded(uint256 indexed jobId, uint256 bounty);
        event JobAssigned(uint256 indexed jobId, address indexed worker);
        event JobSubmitted(uint256 indexed jobId, bytes32 resultHash);
        event JobResolved(uint256 indexed jobId, bool accepted, address indexed worker);
    }

    #[sol(rpc)]
    contract ConsortiumValidator {
        function assembleCommittee(uint256 jobId) external;
        function vote(uint256 jobId, bool approve) external;
        function getMembers(uint256 jobId) external view returns (address[3] memory);
        function voteCounts(uint256 jobId) external view returns (uint8 approves, uint8 rejects, bool tallied);
        event CommitteeAssembled(uint256 indexed jobId, address a, address b, address c);
        event VoteCast(uint256 indexed jobId, address indexed validator, bool approve);
        event Tallied(uint256 indexed jobId, bool accepted);
    }

    #[sol(rpc)]
    contract InsightBoard {
        function post(bytes32 contentHash, string calldata uri) external returns (uint256);
        function confirm(uint256 id) external;
        function claim() external returns (uint256);
        function rewardToken() external view returns (address);
        function REWARD_PER_CONFIRM() external view returns (uint256);
        function confirmed(uint256 id, address confirmer) external view returns (bool);
        function earningsOf(address poster) external view returns (uint256);
        function nextInsightId() external view returns (uint256);
        function getInsight(uint256 id) external view returns (
            address poster,
            bytes32 contentHash,
            string memory uri,
            uint64 postedAt,
            uint64 pheromone
        );
        event InsightPosted(uint256 indexed id, address indexed poster, bytes32 contentHash, string uri);
        event InsightConfirmed(uint256 indexed id, address indexed confirmer, uint64 pheromone);
        event EarningsClaimed(address indexed poster, uint256 amount);
    }

    #[sol(rpc)]
    contract FeeDistributor {
        function distribute(
            uint256 jobId,
            uint256 amount,
            address winner,
            address[] calldata validators,
            address[] calldata dataProviders
        ) external;
        function cumulativeEarnings(address participant) external view returns (uint256);
        event FeesDistributed(
            uint256 indexed jobId,
            uint256 amount,
            address indexed winner,
            uint256 validatorShare,
            uint256 dataShare,
            uint256 agentShare,
            uint256 treasuryShare
        );
        event EarningsCredited(address indexed participant, uint256 amount);
    }
}
