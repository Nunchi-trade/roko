//! Chain domain tool definitions for DeFi operations.
//!
//! These [`ToolDef`] registrations define the 10 core DeFi tools specified in
//! `docs/v1/18-tools/03-chain-domain-tools.md`. The tools use the [`ChainClient`]
//! and [`ChainWallet`] traits for EVM interaction, allowing the same tool
//! definitions to work against both mocked and live backends.
//!
//! # Two-layer model
//!
//! - **Layer 1 (chain primitives)**: `balance`, `transfer`, `gas_estimate`,
//!   `simulate_tx` -- backend-agnostic EVM operations.
//! - **Layer 2 (protocol adapters)**: `approve`, `swap`, `add_liquidity`,
//!   `remove_liquidity`, `get_pool_info`, `get_position` -- protocol-specific
//!   ABI calls (Uniswap, ERC-20).

use roko_core::tool::{
    ToolCategory, ToolConcurrency, ToolDef, ToolPermission, ToolSchema, ToolSource,
};
use std::sync::LazyLock;

/// Number of chain domain tools (10 core + 4 wallet management + 3 knowledge).
pub const CHAIN_TOOL_COUNT: usize = 17;

/// All 14 chain domain tool definitions.
pub static CHAIN_DOMAIN_TOOLS: LazyLock<[ToolDef; CHAIN_TOOL_COUNT]> = LazyLock::new(|| {
    [
        balance_tool_def(),
        transfer_tool_def(),
        approve_tool_def(),
        swap_tool_def(),
        add_liquidity_tool_def(),
        remove_liquidity_tool_def(),
        get_pool_info_tool_def(),
        get_position_tool_def(),
        simulate_tx_tool_def(),
        gas_estimate_tool_def(),
        // TOOL-09: Wallet management tools
        wallet_create_tool_def(),
        wallet_list_tool_def(),
        wallet_info_tool_def(),
        wallet_export_address_tool_def(),
        // Knowledge graph tools (chain insight RPC bridge)
        post_insight_tool_def(),
        search_insights_tool_def(),
        confirm_insight_tool_def(),
    ]
});

/// Canonical names of the 14 chain domain tools.
pub const CHAIN_TOOL_NAMES: [&str; CHAIN_TOOL_COUNT] = [
    "chain.balance",
    "chain.transfer",
    "chain.approve",
    "chain.swap",
    "chain.add_liquidity",
    "chain.remove_liquidity",
    "chain.get_pool_info",
    "chain.get_position",
    "chain.simulate_tx",
    "chain.gas_estimate",
    "chain.wallet_create",
    "chain.wallet_list",
    "chain.wallet_info",
    "chain.wallet_export_address",
    "chain.post_insight",
    "chain.search_insights",
    "chain.confirm_insight",
];

// ──────────────────────────── Layer 1: Chain Primitives ──────────────────────

/// `chain.balance` -- query native or ERC-20 token balance.
fn balance_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.balance".into(),
        description: "Get the balance of a native token or ERC-20 for an address. \
            Returns balance in wei (native) or smallest unit (ERC-20)."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "address": {
                    "type": "string",
                    "description": "The wallet address (0x-prefixed hex)."
                },
                "token": {
                    "type": "string",
                    "description": "ERC-20 token contract address. Omit for native ETH balance."
                },
                "block": {
                    "type": "integer",
                    "description": "Block number to query at. Omit for latest."
                }
            },
            "required": ["address"],
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 30_000,
        concurrency: ToolConcurrency::Parallel,
        idempotent: true,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

/// `chain.transfer` -- send native token or ERC-20 transfer.
fn transfer_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.transfer".into(),
        description: "Transfer native tokens or ERC-20 tokens to an address. \
            Uses eth_sendTransaction for native, ERC-20 transfer() for tokens."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Recipient address (0x-prefixed hex)."
                },
                "amount": {
                    "type": "string",
                    "description": "Amount in wei (native) or smallest unit (ERC-20)."
                },
                "token": {
                    "type": "string",
                    "description": "ERC-20 token contract address. Omit for native ETH."
                }
            },
            "required": ["to", "amount"],
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 120_000,
        concurrency: ToolConcurrency::Serial,
        idempotent: false,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

/// `chain.gas_estimate` -- estimate gas for a transaction.
fn gas_estimate_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.gas_estimate".into(),
        description: "Estimate gas cost for a transaction via eth_estimateGas \
            with a safety buffer (1.2x). Returns estimated gas units and cost in wei."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Destination address (0x-prefixed hex)."
                },
                "data": {
                    "type": "string",
                    "description": "Calldata (0x-prefixed hex)."
                },
                "value": {
                    "type": "string",
                    "description": "Value in wei."
                },
                "from": {
                    "type": "string",
                    "description": "Sender address for context."
                }
            },
            "required": ["to"],
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 30_000,
        concurrency: ToolConcurrency::Parallel,
        idempotent: true,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

/// `chain.simulate_tx` -- dry-run a transaction without broadcasting.
fn simulate_tx_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.simulate_tx".into(),
        description: "Simulate a transaction without broadcasting using eth_call. \
            Returns the call output, gas used, and whether it would revert."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Contract address to call (0x-prefixed hex)."
                },
                "data": {
                    "type": "string",
                    "description": "Calldata (0x-prefixed hex)."
                },
                "value": {
                    "type": "string",
                    "description": "Value in wei (default 0)."
                },
                "from": {
                    "type": "string",
                    "description": "Sender address for simulation context."
                },
                "block": {
                    "type": "integer",
                    "description": "Block number to simulate against. Omit for latest."
                }
            },
            "required": ["to", "data"],
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 60_000,
        concurrency: ToolConcurrency::Parallel,
        idempotent: true,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

// ──────────────────────────── Layer 2: Protocol Adapters ─────────────────────

/// `chain.approve` -- ERC-20 approve spending allowance.
fn approve_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.approve".into(),
        description: "Approve a spender to use ERC-20 tokens via the approve() function. \
            Required before swap/liquidity operations on most DEXes."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "token": {
                    "type": "string",
                    "description": "ERC-20 token contract address (0x-prefixed hex)."
                },
                "spender": {
                    "type": "string",
                    "description": "Address to approve as spender (e.g. router contract)."
                },
                "amount": {
                    "type": "string",
                    "description": "Amount to approve in smallest token unit. Use 'max' for uint256 max."
                }
            },
            "required": ["token", "spender", "amount"],
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 120_000,
        concurrency: ToolConcurrency::Serial,
        idempotent: false,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

/// `chain.swap` -- execute a token swap on a DEX.
fn swap_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.swap".into(),
        description: "Execute a token swap via Uniswap V3 exactInputSingle() or \
            V2 swapExactTokensForTokens(). Requires prior approval of the input token."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "token_in": {
                    "type": "string",
                    "description": "Input token address (0x-prefixed hex)."
                },
                "token_out": {
                    "type": "string",
                    "description": "Output token address (0x-prefixed hex)."
                },
                "amount_in": {
                    "type": "string",
                    "description": "Amount of input token in smallest unit."
                },
                "amount_out_min": {
                    "type": "string",
                    "description": "Minimum output amount (slippage protection)."
                },
                "fee": {
                    "type": "integer",
                    "description": "Pool fee tier in basis points (e.g. 3000 for 0.3%). V3 only."
                },
                "deadline": {
                    "type": "integer",
                    "description": "Unix timestamp deadline. Defaults to now + 20 minutes."
                },
                "recipient": {
                    "type": "string",
                    "description": "Recipient address. Defaults to the wallet address."
                }
            },
            "required": ["token_in", "token_out", "amount_in", "amount_out_min"],
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 120_000,
        concurrency: ToolConcurrency::Serial,
        idempotent: false,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

/// `chain.add_liquidity` -- add liquidity to a DEX pool.
fn add_liquidity_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.add_liquidity".into(),
        description: "Add liquidity to a Uniswap V3 pool via mint() or \
            V2 via addLiquidity(). Requires prior approval of both tokens."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "token_a": {
                    "type": "string",
                    "description": "First token address (0x-prefixed hex)."
                },
                "token_b": {
                    "type": "string",
                    "description": "Second token address (0x-prefixed hex)."
                },
                "amount_a": {
                    "type": "string",
                    "description": "Amount of token A in smallest unit."
                },
                "amount_b": {
                    "type": "string",
                    "description": "Amount of token B in smallest unit."
                },
                "fee": {
                    "type": "integer",
                    "description": "Pool fee tier in basis points (V3 only)."
                },
                "tick_lower": {
                    "type": "integer",
                    "description": "Lower tick bound (V3 concentrated liquidity)."
                },
                "tick_upper": {
                    "type": "integer",
                    "description": "Upper tick bound (V3 concentrated liquidity)."
                }
            },
            "required": ["token_a", "token_b", "amount_a", "amount_b"],
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 120_000,
        concurrency: ToolConcurrency::Serial,
        idempotent: false,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

/// `chain.remove_liquidity` -- remove liquidity from a DEX pool.
fn remove_liquidity_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.remove_liquidity".into(),
        description: "Remove liquidity from a Uniswap V3 position via \
            decreaseLiquidity() + collect(), or V2 via removeLiquidity()."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "token_id": {
                    "type": "string",
                    "description": "V3 NFT position token ID."
                },
                "liquidity": {
                    "type": "string",
                    "description": "Amount of liquidity to remove."
                },
                "token_a": {
                    "type": "string",
                    "description": "First token address (V2)."
                },
                "token_b": {
                    "type": "string",
                    "description": "Second token address (V2)."
                },
                "amount_a_min": {
                    "type": "string",
                    "description": "Minimum token A received (slippage protection)."
                },
                "amount_b_min": {
                    "type": "string",
                    "description": "Minimum token B received (slippage protection)."
                }
            },
            "required": ["liquidity"],
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 120_000,
        concurrency: ToolConcurrency::Serial,
        idempotent: false,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

/// `chain.get_pool_info` -- query pool state (reserves, fee, tick, liquidity).
fn get_pool_info_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.get_pool_info".into(),
        description: "Query a Uniswap pool's state including reserves, fee tier, \
            current tick, total liquidity, and token addresses."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "pool": {
                    "type": "string",
                    "description": "Pool contract address (0x-prefixed hex)."
                },
                "token_a": {
                    "type": "string",
                    "description": "First token address (used to compute pool address if pool is omitted)."
                },
                "token_b": {
                    "type": "string",
                    "description": "Second token address."
                },
                "fee": {
                    "type": "integer",
                    "description": "Fee tier (used with token_a/token_b to find pool)."
                }
            },
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 30_000,
        concurrency: ToolConcurrency::Parallel,
        idempotent: true,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

/// `chain.get_position` -- query an LP position's details.
fn get_position_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.get_position".into(),
        description: "Query a Uniswap V3 LP position by token ID. Returns tick range, \
            liquidity amount, uncollected fees, and token pair information."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "token_id": {
                    "type": "string",
                    "description": "V3 NFT position token ID."
                },
                "position_manager": {
                    "type": "string",
                    "description": "NonfungiblePositionManager contract address. \
                        Uses canonical Uniswap address if omitted."
                }
            },
            "required": ["token_id"],
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 30_000,
        concurrency: ToolConcurrency::Parallel,
        idempotent: true,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

// ──────────────────────────── TOOL-09: Wallet Management ─────���───────────────

/// `chain.wallet_create` -- create a new wallet (key pair).
fn wallet_create_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.wallet_create".into(),
        description: "Create a new wallet with a fresh key pair. Returns the wallet \
            address and a wallet ID for subsequent operations. The private key is \
            stored in the agent's local keystore and never exposed."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "Human-readable label for the wallet (e.g. 'trading', 'treasury')."
                },
                "network": {
                    "type": "string",
                    "description": "Target network (e.g. 'ethereum', 'base', 'arbitrum'). Default: 'ethereum'."
                }
            },
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 30_000,
        concurrency: ToolConcurrency::Serial,
        idempotent: false,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

/// `chain.wallet_list` -- list all wallets managed by this agent.
fn wallet_list_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.wallet_list".into(),
        description: "List all wallets in the agent's local keystore. Returns \
            wallet IDs, labels, addresses, and networks. Does not expose \
            private keys."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "network": {
                    "type": "string",
                    "description": "Filter by network. Omit to list all wallets."
                }
            },
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 10_000,
        concurrency: ToolConcurrency::Parallel,
        idempotent: true,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

/// `chain.wallet_info` -- get details about a specific wallet.
fn wallet_info_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.wallet_info".into(),
        description: "Get detailed information about a wallet: address, label, \
            network, native balance, and recent transaction count. Does not \
            expose private keys."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "wallet_id": {
                    "type": "string",
                    "description": "Wallet identifier returned by wallet_create or wallet_list."
                },
                "address": {
                    "type": "string",
                    "description": "Wallet address (0x-prefixed hex). Alternative to wallet_id."
                }
            },
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 30_000,
        concurrency: ToolConcurrency::Parallel,
        idempotent: true,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

/// `chain.wallet_export_address` -- export a wallet's public address.
fn wallet_export_address_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.wallet_export_address".into(),
        description: "Export a wallet's public address for sharing with other \
            agents or services. Only the address is exported; private keys \
            remain in the local keystore."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "wallet_id": {
                    "type": "string",
                    "description": "Wallet identifier to export the address for."
                }
            },
            "required": ["wallet_id"],
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 10_000,
        concurrency: ToolConcurrency::Parallel,
        idempotent: true,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

// ──────────────────────────── Layer 3: Knowledge Graph ────────────────────────

/// `chain.post_insight` -- post a knowledge insight to the chain knowledge graph.
fn post_insight_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.post_insight".into(),
        description: "Post a knowledge insight to the on-chain knowledge graph via mirage. \
            Insights are HDC-indexed and available for search by other agents. \
            Returns the insight ID and block number."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["heuristic", "causalLink", "warning", "strategyFragment"],
                    "description": "The type of insight being posted."
                },
                "content": {
                    "type": "string",
                    "description": "The insight content (natural language or structured text)."
                },
                "confidence": {
                    "type": "number",
                    "description": "Confidence score between 0.0 and 1.0.",
                    "minimum": 0.0,
                    "maximum": 1.0
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tags for categorizing the insight (e.g. ['defi', 'yield', 'aave'])."
                }
            },
            "required": ["kind", "content", "confidence"],
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 30_000,
        concurrency: ToolConcurrency::Serial,
        idempotent: false,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

/// `chain.search_insights` -- search the chain knowledge graph for relevant insights.
fn search_insights_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.search_insights".into(),
        description: "Search the on-chain knowledge graph for insights matching a query. \
            Uses HDC similarity search. Returns ranked insights with content and metadata."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tags to filter insights by."
                },
                "query": {
                    "type": "string",
                    "description": "Natural language search query for semantic matching."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return. Default: 5.",
                    "default": 5
                }
            },
            "required": ["tags"],
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 30_000,
        concurrency: ToolConcurrency::Parallel,
        idempotent: true,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

/// `chain.confirm_insight` -- confirm (upvote) an existing insight on the chain.
fn confirm_insight_tool_def() -> ToolDef {
    ToolDef {
        name: "chain.confirm_insight".into(),
        description: "Confirm an insight in the on-chain knowledge graph, increasing its \
            weight and credibility. Returns confirmation status and block number."
            .into(),
        parameters: ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The insight ID to confirm (returned by chain.post_insight or chain.search_insights)."
                }
            },
            "required": ["id"],
            "additionalProperties": false
        })),
        category: ToolCategory::Network,
        permission: ToolPermission::networked(),
        timeout_ms: 30_000,
        concurrency: ToolConcurrency::Serial,
        idempotent: false,
        source: ToolSource::Builtin,
        metadata: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_tools_have_correct_count() {
        assert_eq!(CHAIN_DOMAIN_TOOLS.len(), CHAIN_TOOL_COUNT);
        assert_eq!(CHAIN_TOOL_NAMES.len(), CHAIN_TOOL_COUNT);
    }

    #[test]
    fn chain_tool_names_match_definitions() {
        for (i, tool) in CHAIN_DOMAIN_TOOLS.iter().enumerate() {
            assert_eq!(
                tool.name, CHAIN_TOOL_NAMES[i],
                "tool at index {i} name mismatch"
            );
        }
    }

    #[test]
    fn all_chain_tools_have_network_category() {
        for tool in CHAIN_DOMAIN_TOOLS.iter() {
            assert_eq!(
                tool.category,
                ToolCategory::Network,
                "tool {} should be Network",
                tool.name
            );
        }
    }

    #[test]
    fn read_only_tools_are_idempotent() {
        let read_tools = [
            "chain.balance",
            "chain.gas_estimate",
            "chain.simulate_tx",
            "chain.get_pool_info",
            "chain.get_position",
            "chain.search_insights",
        ];
        for tool in CHAIN_DOMAIN_TOOLS.iter() {
            if read_tools.contains(&tool.name.as_str()) {
                assert!(tool.idempotent, "tool {} should be idempotent", tool.name);
            }
        }
    }

    #[test]
    fn mutating_tools_are_serial() {
        let write_tools = [
            "chain.transfer",
            "chain.approve",
            "chain.swap",
            "chain.add_liquidity",
            "chain.remove_liquidity",
        ];
        for tool in CHAIN_DOMAIN_TOOLS.iter() {
            if write_tools.contains(&tool.name.as_str()) {
                assert_eq!(
                    tool.concurrency,
                    ToolConcurrency::Serial,
                    "tool {} should be Serial",
                    tool.name
                );
                assert!(
                    !tool.idempotent,
                    "tool {} should not be idempotent",
                    tool.name
                );
            }
        }
    }

    #[test]
    fn balance_tool_requires_address() {
        let tool = &CHAIN_DOMAIN_TOOLS[0];
        assert_eq!(tool.name, "chain.balance");
        let schema = tool.parameters.as_value();
        let required = schema["required"].as_array().expect("required array");
        assert!(required.contains(&serde_json::json!("address")));
    }

    #[test]
    fn swap_tool_has_correct_required_params() {
        let tool = CHAIN_DOMAIN_TOOLS
            .iter()
            .find(|t| t.name == "chain.swap")
            .expect("swap tool");
        let schema = tool.parameters.as_value();
        let required = schema["required"].as_array().expect("required array");
        assert_eq!(required.len(), 4);
        assert!(required.contains(&serde_json::json!("token_in")));
        assert!(required.contains(&serde_json::json!("token_out")));
        assert!(required.contains(&serde_json::json!("amount_in")));
        assert!(required.contains(&serde_json::json!("amount_out_min")));
    }

    #[test]
    fn tool_defs_serde_roundtrip() {
        for tool in CHAIN_DOMAIN_TOOLS.iter() {
            let json = serde_json::to_string(tool)
                .unwrap_or_else(|err| panic!("serialize {}: {err}", tool.name));
            let back: ToolDef = serde_json::from_str(&json)
                .unwrap_or_else(|err| panic!("deserialize {}: {err}", tool.name));
            assert_eq!(back.name, tool.name);
            assert_eq!(back.category, tool.category);
        }
    }

    // TOOL-09: Wallet management tools

    #[test]
    fn wallet_tools_present() {
        let wallet_names = [
            "chain.wallet_create",
            "chain.wallet_list",
            "chain.wallet_info",
            "chain.wallet_export_address",
        ];
        for name in wallet_names {
            assert!(
                CHAIN_DOMAIN_TOOLS.iter().any(|t| t.name == name),
                "missing wallet tool: {name}"
            );
        }
    }

    #[test]
    fn wallet_create_is_serial() {
        let tool = CHAIN_DOMAIN_TOOLS
            .iter()
            .find(|t| t.name == "chain.wallet_create")
            .expect("wallet_create tool");
        assert_eq!(tool.concurrency, ToolConcurrency::Serial);
        assert!(!tool.idempotent);
    }

    #[test]
    fn wallet_list_is_idempotent() {
        let tool = CHAIN_DOMAIN_TOOLS
            .iter()
            .find(|t| t.name == "chain.wallet_list")
            .expect("wallet_list tool");
        assert!(tool.idempotent);
        assert_eq!(tool.concurrency, ToolConcurrency::Parallel);
    }

    #[test]
    fn wallet_export_requires_wallet_id() {
        let tool = CHAIN_DOMAIN_TOOLS
            .iter()
            .find(|t| t.name == "chain.wallet_export_address")
            .expect("wallet_export tool");
        let schema = tool.parameters.as_value();
        let required = schema["required"].as_array().expect("required array");
        assert!(required.contains(&serde_json::json!("wallet_id")));
    }
}
