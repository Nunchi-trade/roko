# ChainWitness: Event Watching Pipeline

> ChainWitness is the agent's eyes on the chain. One Tokio task per configured chain maintains a WebSocket connection, ingests block headers, pre-screens with a Binary Fuse filter (8.7 bits/entry, <1% FPR), and feeds relevant blocks to the triage pipeline. Over 90% of blocks are skipped in O(1) time with zero false negatives.


> **Implementation**: Built

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [01-korai-chain-spec.md](./01-korai-chain-spec.md)
**Key sources**: `bardo-backup/prd/14-chain/01-witness.md`, `bardo-backup/prd/14-chain/00-architecture.md`, `roko/tmp/implementation-plans/12b-chain-layer.md` §H

---

## Abstract

ChainWitness is the lowest level of the chain intelligence pipeline. It connects to Ethereum-compatible chains via WebSocket, ingests every block header, and determines which blocks are relevant to the agent. The key innovation is the Binary Fuse pre-screening filter: a probabilistic data structure that tests each block's `logsBloom` (a 2048-bit Bloom filter included in every Ethereum block header) against the agent's set of watched addresses and event topics. The filter achieves <1% false positive rate at 8.7 bits per entry, with **zero false negatives** — no relevant block is ever missed.

On Ethereum mainnet (~7,500 blocks/day), an agent watching 50 addresses and 100 event topics skips >90% of blocks. The remaining ~10% trigger full block fetches. This pre-screening is what makes it feasible for a single agent to monitor multiple chains simultaneously without overwhelming bandwidth or processing capacity.

ChainWitness is the renamed equivalent of `bardo-witness` from the legacy architecture (see [naming conventions](../00-architecture/INDEX.md)).

---

## Architecture

### WitnessEngine

```rust
pub struct WitnessEngine {
    /// Binary Fuse filter of watched addresses + event topics.
    /// Rebuilt at each Gamma tick, swapped atomically via ArcSwap.
    watch_filter: Arc<ArcSwap<BinaryFuse8>>,

    /// Dedicated WebSocket subscription connection.
    /// Only used for eth_subscribe("newHeads"). Never shares with queries.
    subscription: Arc<WsProvider>,

    /// Pool for block + receipt fetches.
    /// Multiple concurrent fetches during burst activity.
    query_pool: deadpool::Pool<WsProvider>,

    /// Tracks processed blocks for gap detection.
    /// Roaring Bitmap: ~7,500 blocks/day → few KB for 30-day window.
    seen_blocks: Arc<Mutex<RoaringBitmap>>,

    /// Latest chain head block number.
    latest_block: AtomicU64,
}
```

### Block Ingestion Pipeline

```
eth_subscribe("newHeads")
    │ block header arrives
    ├── update gas_gwei (always, from header)
    ├── update latest_block (AtomicU64)
    └── Binary Fuse filter check against logsBloom
        ├── miss (>90%): continue to next header
        └── hit:
            ├── eth_getBlockByHash(hash, true)     // full block with txs
            ├── eth_getBlockReceipts(hash)          // receipts with logs
            └── send (block, receipts) to triage channel
```

The subscription connection is dedicated to `eth_subscribe("newHeads")` only. Block fetches fan out across a separate query connection pool, so burst block activity cannot starve the subscription.

---

## Binary Fuse Pre-Screening

### How It Works

Every Ethereum block header contains a `logsBloom` — a 2048-bit Bloom filter (256 bytes) that encodes the union of all log-emitting addresses and all indexed event topics across every transaction in the block (Wood, 2014, §4.3). This built-in filter answers "might this block contain an event from address X?" with zero false negatives.

The agent maintains its own filter — a `BinaryFuse8` from the `xorf` crate — built from the agent's set of watched addresses and topics. Binary fuse filters (Lemire et al., 2022) are immutable, rebuild-oriented structures that use only 8.7 bits per entry with sub-1% false positive rates.

For each arriving block header, the witness tests the header's `logsBloom` against the agent's Binary Fuse filter:

```rust
fn check_bloom_against_filter(
    logs_bloom: &Bloom,
    filter: &BinaryFuse8,
) -> bool {
    // Extract candidate addresses and topics from the logsBloom
    // and test each against the BinaryFuse8.
    // False positive rate: <1% at typical watch list sizes.
    check_logs_bloom_intersection(logs_bloom, filter)
}
```

### Why Binary Fuse Over Bloom?

| Property | Standard Bloom | Binary Fuse | Cuckoo |
|---|---|---|---|
| **Bits per entry** | ~9.6 | **8.7** | ~8.5 |
| **False positive rate** | <1% | <1% | <1% |
| **Deletion support** | No | No | Yes |
| **Construction speed** | Fast | **2× faster than xor** | Medium |
| **Mutability** | Insert-only | **Immutable (rebuild)** | Delete + insert |

The agent's watch list is rebuilt from scratch at each Gamma tick — the set of watched addresses changes as the agent's attention shifts. Binary fuse filters' immutable-rebuild pattern is a natural fit. Cuckoo filters' deletion support is irrelevant. Bloom filters waste ~10% more space for the same FPR.

### Filter Construction

Built from the agent's interest entries at each Gamma tick:

```rust
fn build_watch_filter(entries: &[InterestEntry]) -> BinaryFuse8 {
    let keys: Vec<u64> = entries
        .iter()
        .flat_map(|e| {
            let addr_hash = xxh3_64(e.address.as_slice());
            let topic_hashes = e.event_topics.iter()
                .map(|t| xxh3_64(t.as_slice()));
            std::iter::once(addr_hash).chain(topic_hashes)
        })
        .collect();

    BinaryFuse8::try_from(&keys).expect("filter construction failed")
}
```

The new filter atomically replaces the old one via `arc_swap::ArcSwap`. In-flight checks against the old filter are safe — zero false negatives means at worst the agent fetches one extra block during the swap.

---

## Reconnection and Gap Handling

WebSocket connections drop. The witness handles this without losing coverage:

1. **On disconnect**: Record `last_processed_block` in `AtomicU64`.
2. **On reconnect**:
   - Call `eth_blockNumber` for current chain head.
   - Load seen-blocks Roaring Bitmap from redb.
   - Scan for gaps between `last_processed` and current head.
3. **Gap ≤ 1,000 blocks**: Backfill via `eth_getLogs` with watched address filter.
4. **Gap > 1,000 blocks**:
   - Emit `ChainGapDetected` event with `{ chain_id, from_block, to_block, gap_size }`.
   - Resume from current head; the gap is a permanent hole in awareness.
   - The agent's uncertainty model adjusts — the agent knows it missed something.

### Seen-Blocks Tracking

Uses Roaring Bitmaps (Lemire et al., 2016): partitions the integer space into 2^16-element chunks, using dense bitsets for continuous ranges and sorted arrays for sparse ones. At ~7,500 blocks/day, a 30-day bitmap (~225,000 block numbers) compresses to a few KB.

---

## Connection Pool

```rust
pub struct WitnessPool {
    /// Dedicated subscription — never used for anything else.
    subscription_conn: Arc<WsProvider>,

    /// Pool for block + receipt fetches.
    query_conns: deadpool::Pool<WsProvider>,

    /// HTTP fallback when WS pool saturates.
    rpc_fallbacks: Vec<Arc<HttpProvider>>,
}
```

Reconnection uses exponential backoff: 3s, 6s, 12s, 30s max.

---

## Block Normalization

Before forwarding to triage, the witness normalizes blocks into a chain-agnostic format:

```rust
pub struct NormalizedBlock {
    pub chain_id: u64,
    pub number: u64,
    pub hash: B256,
    pub timestamp: u64,
    pub base_fee_per_gas: u64,
    pub transactions: Vec<NormalizedTx>,
    pub receipts: Vec<TransactionReceipt>,
}
```

This abstraction handles chain-specific differences: L1 vs. L2 transaction types, EIP-4844 blob transactions, Optimism-specific deposit transactions, etc.

---

## Per-Chain Configuration

```rust
pub struct ChainWitnessConfig {
    pub chain_id: u64,
    pub ws_url: String,
    pub rpc_urls: Vec<String>,       // HTTP fallbacks
    pub query_pool_size: usize,      // default: 4
    pub gap_backfill_limit: u64,     // default: 1_000 blocks
    pub max_watch_size: usize,       // cap on filter items, default: 10_000
}
```

One `WitnessEngine` per configured chain, each running independently. If one chain's WebSocket drops, others are unaffected.

---

## Metrics

| Metric | Type | Description |
|---|---|---|
| `witness.blocks_received` | counter | Total block headers received |
| `witness.filter_hits` | counter | Blocks passing filter check |
| `witness.filter_misses` | counter | Blocks rejected by filter |
| `witness.full_fetches` | counter | Full block+receipts fetches |
| `witness.gaps_detected` | counter | WebSocket reconnect gaps |
| `witness.ws_reconnects` | counter | Reconnection events |
| `witness.fetch_latency_ms` | histogram | Full block fetch latency |

Filter hit rate (`hits / (hits + misses)`) is the primary tuning metric. If it exceeds 20%, the filter may be too permissive — review the agent's interest scoring.

---

## Academic Foundations

- Bloom, B.H. (1970). "Space/time trade-offs in hash coding with allowable errors." *Communications of the ACM*, 13(7). — The Ethereum logsBloom is a descendant.
- Graf, T.M. and Lemire, D. (2020). "Xor Filters: Faster and Smaller Than Bloom and Cuckoo Filters." *Journal of Experimental Algorithmics*. — Predecessor to Binary Fuse filters.
- Lemire, D. et al. (2022). "Binary Fuse Filters: Fast and Smaller Than Xor Filters." *Journal of Experimental Algorithmics*. — The specific filter implementation: 8.7 bits/entry, <1% FPR.
- Lemire, D. et al. (2016). "Consistently faster and smaller compressed bitmaps with Roaring." *Software: Practice & Experience*. — Roaring Bitmap for seen-block tracking.
- Wood, G. (2014). Ethereum Yellow Paper, §4.3. — logsBloom definition.

---

## Current Status and Gaps

**Scaffold:**
- `ChainClient` trait in `roko-chain/src/client.rs` provides `get_block_header()`, `get_logs()`
- Binary Fuse filter available via `xorf` crate
- Roaring Bitmap available via `roaring` crate

**Not yet built (Tier 6):**
- `WitnessEngine` with WebSocket subscription loop (§H1)
- Binary Fuse filter construction from interest entries (§H2)
- Gap detection and backfill logic (§H3)
- Block normalization for multi-chain support (§H4)
- Connection pool with HTTP fallback (§H5)
- Metrics collection and reporting (§H6)

---

## Cross-references

- See [16-triage-curiosity-midas.md](./16-triage-curiosity-midas.md) for the downstream consumer of witness output
- See [17-chain-client-wallet-traits.md](./17-chain-client-wallet-traits.md) for the `ChainClient` trait that provides RPC methods
- See [19-chain-agent-heartbeat.md](./19-chain-agent-heartbeat.md) for how the witness feeds into the 9-step heartbeat (OBSERVE step)
- See [01-korai-chain-spec.md](./01-korai-chain-spec.md) for the chain architecture that the witness monitors
