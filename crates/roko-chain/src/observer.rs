//! Block observer for the ChainWitness event-watching pipeline (CHAIN-07).
//!
//! `BlockObserver` subscribes to new blocks via [`ChainClient`], pre-screens
//! transactions against a set of watched addresses using a Bloom filter, and
//! feeds matched events into the [`TriagePipeline`](crate::triage::TriagePipeline).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::types::{BlockNumber, ChainHeader, LogEntry};

/// Configuration for the block observer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockObserverConfig {
    /// Addresses to watch for events.
    pub watched_addresses: Vec<String>,
    /// Topic hashes to watch for.
    pub watched_topics: Vec<String>,
    /// Maximum number of blocks to fetch in a single batch (gap filling).
    pub max_batch_size: u64,
    /// Whether to track processed blocks for gap detection.
    pub gap_detection: bool,
}

impl Default for BlockObserverConfig {
    fn default() -> Self {
        Self {
            watched_addresses: Vec::new(),
            watched_topics: Vec::new(),
            max_batch_size: 100,
            gap_detection: true,
        }
    }
}

/// A Bloom-style address filter for fast pre-screening of transactions.
///
/// Uses a simple `HashSet` rather than a full Binary Fuse filter; the
/// interface is ready for an `xorf` upgrade when the dependency is justified.
#[derive(Debug, Clone)]
pub struct AddressFilter {
    addresses: HashSet<String>,
    topics: HashSet<String>,
}

impl AddressFilter {
    /// Build an address filter from the observer configuration.
    #[must_use]
    pub fn from_config(config: &BlockObserverConfig) -> Self {
        Self {
            addresses: config
                .watched_addresses
                .iter()
                .map(|a| a.to_lowercase())
                .collect(),
            topics: config
                .watched_topics
                .iter()
                .map(|t| t.to_lowercase())
                .collect(),
        }
    }

    /// Check whether a log entry matches any watched address or topic.
    #[must_use]
    pub fn matches(&self, log: &LogEntry) -> bool {
        if !self.addresses.is_empty() && !self.addresses.contains(&log.address.to_lowercase()) {
            return false;
        }
        if self.topics.is_empty() {
            return true;
        }
        log.topics
            .iter()
            .any(|t| self.topics.contains(&t.to_lowercase()))
    }

    /// Number of watched addresses.
    #[must_use]
    pub fn address_count(&self) -> usize {
        self.addresses.len()
    }

    /// Number of watched topics.
    #[must_use]
    pub fn topic_count(&self) -> usize {
        self.topics.len()
    }
}

/// Tracks which blocks have been processed, detecting gaps.
#[derive(Debug, Clone, Default)]
pub struct BlockTracker {
    /// Set of processed block numbers.
    processed: HashSet<BlockNumber>,
    /// Highest block number seen.
    high_water: BlockNumber,
    /// Lowest block number seen.
    low_water: Option<BlockNumber>,
}

impl BlockTracker {
    /// Record a block as processed.
    pub fn mark_processed(&mut self, block: BlockNumber) {
        self.processed.insert(block);
        self.high_water = self.high_water.max(block);
        self.low_water = Some(self.low_water.map_or(block, |lw| lw.min(block)));
    }

    /// Detect gaps in the processed block range.
    #[must_use]
    pub fn detect_gaps(&self) -> Vec<BlockNumber> {
        let Some(low) = self.low_water else {
            return Vec::new();
        };
        (low..=self.high_water)
            .filter(|n| !self.processed.contains(n))
            .collect()
    }

    /// Number of processed blocks.
    #[must_use]
    pub fn processed_count(&self) -> usize {
        self.processed.len()
    }

    /// Highest block number seen.
    #[must_use]
    pub fn high_water_mark(&self) -> BlockNumber {
        self.high_water
    }
}

/// An event extracted from a block that passed the address filter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservedEvent {
    /// Block number where the event was emitted.
    pub block_number: BlockNumber,
    /// Block hash.
    pub block_hash: String,
    /// Block timestamp (unix seconds).
    pub block_timestamp: u64,
    /// The matching log entry.
    pub log: LogEntry,
}

/// Block observer that watches for events matching configured addresses/topics.
#[derive(Debug, Clone)]
pub struct BlockObserver {
    /// Observer configuration.
    pub config: BlockObserverConfig,
    /// Pre-screening filter.
    pub filter: AddressFilter,
    /// Block gap tracker.
    pub tracker: BlockTracker,
}

impl BlockObserver {
    /// Create a new block observer from configuration.
    #[must_use]
    pub fn new(config: BlockObserverConfig) -> Self {
        let filter = AddressFilter::from_config(&config);
        Self {
            config,
            filter,
            tracker: BlockTracker::default(),
        }
    }

    /// Process a block header and its logs, returning matched events.
    ///
    /// The block is marked as processed in the tracker.
    pub fn process_block(&mut self, header: &ChainHeader, logs: &[LogEntry]) -> Vec<ObservedEvent> {
        self.tracker.mark_processed(header.number);

        logs.iter()
            .filter(|log| self.filter.matches(log))
            .map(|log| ObservedEvent {
                block_number: header.number,
                block_hash: header.hash.clone(),
                block_timestamp: header.timestamp,
                log: log.clone(),
            })
            .collect()
    }

    /// Return any gaps in the block sequence that need backfilling.
    #[must_use]
    pub fn pending_gaps(&self) -> Vec<BlockNumber> {
        if self.config.gap_detection {
            self.tracker.detect_gaps()
        } else {
            Vec::new()
        }
    }

    /// Scan a range of blocks using the provided client accessor.
    ///
    /// This is a synchronous batch processor: the caller supplies block
    /// headers and logs for each block in the range. The returned events
    /// are the union of all matches.
    pub fn scan_range(&mut self, blocks: &[(ChainHeader, Vec<LogEntry>)]) -> Vec<ObservedEvent> {
        let mut events = Vec::new();
        for (header, logs) in blocks {
            events.extend(self.process_block(header, logs));
        }
        events
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn header(number: u64) -> ChainHeader {
        ChainHeader {
            number,
            hash: format!("0xblock{number}"),
            parent: format!("0xblock{}", number.saturating_sub(1)),
            timestamp: 1_700_000_000 + number,
        }
    }

    fn log_entry(address: &str, topic: &str) -> LogEntry {
        LogEntry {
            address: address.to_string(),
            topics: vec![topic.to_string()],
            data: vec![1, 2, 3],
        }
    }

    #[test]
    fn filter_matches_watched_address() {
        let config = BlockObserverConfig {
            watched_addresses: vec!["0xCafe".to_string()],
            watched_topics: vec![],
            ..Default::default()
        };
        let filter = AddressFilter::from_config(&config);

        assert!(filter.matches(&log_entry("0xcafe", "0xtopic1")));
        assert!(!filter.matches(&log_entry("0xdead", "0xtopic1")));
    }

    #[test]
    fn filter_matches_watched_topic() {
        let config = BlockObserverConfig {
            watched_addresses: vec![],
            watched_topics: vec!["0xABCD".to_string()],
            ..Default::default()
        };
        let filter = AddressFilter::from_config(&config);

        assert!(filter.matches(&log_entry("0xany", "0xabcd")));
        assert!(!filter.matches(&log_entry("0xany", "0x1234")));
    }

    #[test]
    fn filter_matches_both_address_and_topic() {
        let config = BlockObserverConfig {
            watched_addresses: vec!["0xcafe".to_string()],
            watched_topics: vec!["0xabcd".to_string()],
            ..Default::default()
        };
        let filter = AddressFilter::from_config(&config);

        assert!(filter.matches(&log_entry("0xcafe", "0xabcd")));
        assert!(!filter.matches(&log_entry("0xdead", "0xabcd")));
        assert!(!filter.matches(&log_entry("0xcafe", "0x1234")));
    }

    #[test]
    fn observer_processes_block_and_filters_events() {
        let config = BlockObserverConfig {
            watched_addresses: vec!["0xcafe".to_string()],
            ..Default::default()
        };
        let mut observer = BlockObserver::new(config);

        let logs = vec![
            log_entry("0xcafe", "0xtopic1"),
            log_entry("0xdead", "0xtopic2"),
            log_entry("0xcafe", "0xtopic3"),
        ];

        let events = observer.process_block(&header(100), &logs);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].block_number, 100);
        assert_eq!(events[1].log.topics[0], "0xtopic3");
    }

    #[test]
    fn tracker_detects_gaps() {
        let mut tracker = BlockTracker::default();
        tracker.mark_processed(10);
        tracker.mark_processed(12);
        tracker.mark_processed(14);

        let gaps = tracker.detect_gaps();
        assert_eq!(gaps, vec![11, 13]);
    }

    #[test]
    fn tracker_no_gaps_for_contiguous_range() {
        let mut tracker = BlockTracker::default();
        for i in 5..=10 {
            tracker.mark_processed(i);
        }
        assert!(tracker.detect_gaps().is_empty());
    }

    #[test]
    fn observer_scan_range_accumulates_events() {
        let config = BlockObserverConfig {
            watched_addresses: vec!["0xcafe".to_string()],
            ..Default::default()
        };
        let mut observer = BlockObserver::new(config);

        let blocks = vec![
            (header(1), vec![log_entry("0xcafe", "0xa")]),
            (header(2), vec![log_entry("0xdead", "0xb")]),
            (header(3), vec![log_entry("0xcafe", "0xc")]),
        ];

        let events = observer.scan_range(&blocks);
        assert_eq!(events.len(), 2);
        assert_eq!(observer.tracker.processed_count(), 3);
    }

    #[test]
    fn gap_detection_disabled_returns_empty() {
        let config = BlockObserverConfig {
            gap_detection: false,
            ..Default::default()
        };
        let mut observer = BlockObserver::new(config);
        observer.tracker.mark_processed(1);
        observer.tracker.mark_processed(5);

        assert!(observer.pending_gaps().is_empty());
    }
}
