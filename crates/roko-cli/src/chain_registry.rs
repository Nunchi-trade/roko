//! Builds a [`HandlerResolver`](roko_agent::dispatcher::HandlerResolver) that
//! chains chain tool handlers before std builtins.

use std::collections::HashMap;
use std::sync::Arc;

use roko_chain::tools::CHAIN_TOOL_NAMES;
use roko_chain::{ChainClient, ChainWallet};
use roko_core::tool::ToolHandler;

use crate::chain_handler::ChainToolHandler;

/// Build a map of chain tool name -> handler, given a live client/wallet.
pub fn chain_handler_map(
    client: Arc<dyn ChainClient>,
    wallet: Option<Arc<dyn ChainWallet>>,
) -> HashMap<String, Arc<dyn ToolHandler>> {
    chain_handler_map_with_rpc(client, wallet, None)
}

/// Build a map of chain tool name -> handler, with an explicit mirage RPC URL
/// for knowledge graph operations (chain.post_insight, etc.).
pub fn chain_handler_map_with_rpc(
    client: Arc<dyn ChainClient>,
    wallet: Option<Arc<dyn ChainWallet>>,
    rpc_url: Option<String>,
) -> HashMap<String, Arc<dyn ToolHandler>> {
    CHAIN_TOOL_NAMES
        .iter()
        .map(|&name| {
            let h: Arc<dyn ToolHandler> = Arc::new(ChainToolHandler {
                client: Arc::clone(&client),
                wallet: wallet.clone(),
                tool_name: name.to_string(),
                rpc_url: rpc_url.clone(),
            });
            (name.to_string(), h)
        })
        .collect()
}

/// Create a handler resolver closure that checks chain tools first,
/// then falls through to the standard builtin handlers.
pub fn chain_aware_resolver(
    chain_handlers: HashMap<String, Arc<dyn ToolHandler>>,
) -> impl Fn(&str) -> Option<Arc<dyn ToolHandler>> + Send + Sync {
    move |name: &str| -> Option<Arc<dyn ToolHandler>> {
        chain_handlers
            .get(name)
            .cloned()
            .or_else(|| roko_std::tool::handlers::handler_for(name))
    }
}
