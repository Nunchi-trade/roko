//! CLI configuration for `roko-chain-watcher`.
//!
//! All runtime knobs are surfaced as `clap`-derived flags with environment-variable
//! fallbacks so the binary can be driven from either a shell or a systemd unit.

use clap::Parser;

/// Command-line flags accepted by the watcher.
#[derive(Clone, Debug, Parser)]
#[command(
    name = "roko-chain-watcher",
    about = "Long-running roko agent: observes a mirage chain and posts insights"
)]
pub struct WatcherCli {
    /// `mirage-rs` JSON-RPC endpoint (HTTP).
    #[arg(long, default_value = "http://127.0.0.1:8545", env = "MIRAGE_RPC_URL")]
    pub rpc_url: String,

    /// Name/identity of this watcher (used as `author` on posted insights).
    #[arg(long, default_value = "roko-watcher-001", env = "ROKO_WATCHER_ID")]
    pub watcher_id: String,

    /// Poll interval for chain state / pheromones (ms).
    #[arg(long, default_value_t = 2000)]
    pub poll_interval_ms: u64,

    /// How many recent pheromones to fetch per poll.
    #[arg(long, default_value_t = 25)]
    pub poll_k: usize,

    /// Pheromone topic query (HDC search).
    #[arg(
        long,
        default_value = "threat opportunity wisdom",
        env = "ROKO_WATCHER_QUERY"
    )]
    pub query: String,

    /// Dry-run mode — observe and log, don't post anything.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    /// Max reactions per minute (rate limit).
    #[arg(long, default_value_t = 30)]
    pub max_reactions_per_min: u32,

    /// Exit after N observed events (0 = run forever).
    #[arg(long, default_value_t = 0)]
    pub max_events: u64,

    /// Ethereum RPC URL for real block-data fetches. Defaults to `--rpc-url`
    /// (mirage serves historical blocks from its upstream fork via lazy proxy).
    #[arg(long, env = "ETH_RPC_URL")]
    pub eth_rpc_url: Option<String>,

    /// Interval for block-observer polling (ms). Independent from chain poll.
    #[arg(long, default_value_t = 3000)]
    pub block_poll_interval_ms: u64,

    /// On startup, analyze this many recent historical blocks.
    #[arg(long, default_value_t = 10)]
    pub block_backfill: u64,

    /// Fetch full transactions per block for whale/MEV detection (heavier).
    #[arg(long, default_value_t = false)]
    pub fetch_full_txs: bool,

    /// Disable the pattern-matcher reaction loop (keep only block observer).
    #[arg(long, default_value_t = false)]
    pub disable_reactions: bool,

    /// Disable the block observer (keep only pattern-matcher reactions).
    #[arg(long, default_value_t = false)]
    pub disable_block_observer: bool,
}

impl WatcherCli {
    /// Returns a `WatcherCli` populated with all defaults. Used by tests and
    /// by embedders that want to invoke the watcher without a CLI invocation.
    #[must_use]
    #[allow(dead_code)]
    pub fn with_defaults() -> Self {
        Self {
            rpc_url: "http://127.0.0.1:8545".to_string(),
            watcher_id: "roko-watcher-001".to_string(),
            poll_interval_ms: 2000,
            poll_k: 25,
            query: "threat opportunity wisdom".to_string(),
            dry_run: false,
            max_reactions_per_min: 30,
            max_events: 0,
            eth_rpc_url: None,
            block_poll_interval_ms: 3000,
            block_backfill: 10,
            fetch_full_txs: false,
            disable_reactions: false,
            disable_block_observer: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let cli = WatcherCli::with_defaults();
        assert_eq!(cli.rpc_url, "http://127.0.0.1:8545");
        assert_eq!(cli.poll_interval_ms, 2000);
        assert!(!cli.dry_run);
        assert_eq!(cli.max_reactions_per_min, 30);
    }
}
