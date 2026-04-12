# Knowledge Backup and Export

> **Layer**: L1 Framework (Substrate serialization) + L0 Runtime (snapshot persistence)
>
> **Prerequisites**: `docs/03-neuro/INDEX.md` (Neuro knowledge store, Engram format), `docs/17-lifecycle/00-vision-and-mortality-replaced.md` (lifecycle model)
>
> **Synapse traits**: Substrate (the knowledge store being backed up — all Engrams with their scores, decay state, tier, and provenance), Scorer (score metadata preserved in backup), Gate (gate results preserved as provenance on Engrams)

---

## Overview

Knowledge backup in Roko replaces the legacy "death testament" and "succession" system. Instead of an agent producing a compressed knowledge artifact at death, the operator initiates backups at any time via `roko neuro backup`. The backup captures the agent's entire Neuro store — all Engrams with their scores, decay state, knowledge tier, provenance chains, and metadata — in a portable format that can be restored into a different agent.

This is the first step of the four-step knowledge transfer process that replaces legacy succession:

```
BACKUP → DELETE → CREATE → RESTORE
```

Each step is user-initiated, explicit, and reversible (except DELETE, which is intentionally irreversible for the agent process — though the backup persists).

---

## Backup Command

```bash
# Full Neuro backup to default location
roko neuro backup

# Backup to a specific path
roko neuro backup --output ./backups/agent-V1St-2026-04-12.neuro

# Backup with compression
roko neuro backup --compress

# Backup with encryption (operator's key)
roko neuro backup --encrypt --key-file ~/.roko/backup.key

# Backup only specific knowledge types
roko neuro backup --types insight,heuristic,causal_link

# Backup only entries above a confidence threshold
roko neuro backup --min-confidence 0.3

# Dry-run: show what would be backed up
roko neuro backup --dry-run
```

### Default Backup Location

```
.roko/backups/{agent_id}/{timestamp}.neuro
```

Example: `.roko/backups/agent-V1StGXR8_Z5j/2026-04-12T14-30-00Z.neuro`

---

## Backup Format

The backup is a content-addressed archive containing all Engrams and their metadata. The format is designed for portability — a backup can be restored into any Roko agent regardless of version, domain, or configuration.

### Archive Structure

```
{agent_id}-{timestamp}.neuro
├── manifest.toml          # Backup metadata
├── engrams/
│   ├── {hash1}.engram     # Individual Engram files (BLAKE3 content-addressed)
│   ├── {hash2}.engram
│   └── ...
├── scores/
│   └── scores.jsonl       # All 7-axis scores for each Engram
├── tiers/
│   └── tiers.jsonl        # Tier assignments (Transient/Working/Consolidated/Persistent)
├── provenance/
│   └── provenance.jsonl   # Lineage chains and source attribution
├── decay/
│   └── decay_state.jsonl  # Current decay state per Engram (Ebbinghaus parameters)
├── playbook.md            # Machine-evolved heuristics (PLAYBOOK.md snapshot)
└── checksum.blake3        # BLAKE3 hash of entire archive
```

### Manifest

```toml
# manifest.toml — Backup metadata

[backup]
version = 1
agent_id = "agent-V1StGXR8_Z5j"
agent_name = "my-agent"
created_at = "2026-04-12T14:30:00Z"
roko_version = "0.1.0"

[stats]
total_engrams = 12847
engrams_by_type.insight = 3421
engrams_by_type.heuristic = 2156
engrams_by_type.warning = 1893
engrams_by_type.causal_link = 2744
engrams_by_type.strategy_fragment = 1821
engrams_by_type.anti_knowledge = 812
engrams_by_tier.transient = 4521
engrams_by_tier.working = 3892
engrams_by_tier.consolidated = 3156
engrams_by_tier.persistent = 1278
average_confidence = 0.63
median_confidence = 0.58
playbook_size_bytes = 8472

[provenance]
# Backup provenance — how this knowledge was generated
agent_lifetime_hours = 342.5
total_cognitive_loops = 48721
gate_pass_rate = 0.73
domains_active = ["price_direction", "volatility_regime", "yield_trend"]
```

### Engram File Format

Each Engram is stored as a self-contained file, content-addressed by BLAKE3 hash:

```rust
/// Engram as stored in backup archive.
/// This is the canonical serialization format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupEngram {
    /// Content-addressed hash: BLAKE3(kind + body + author + tags).
    pub hash: String,

    /// Engram kind (one of six knowledge types).
    pub kind: EngramKind,

    /// The knowledge content.
    pub body: String,

    /// Author agent ID.
    pub author: String,

    /// Tags for categorization and retrieval.
    pub tags: Vec<String>,

    /// 7-axis score at time of backup.
    pub score: EngramScore,

    /// Current knowledge tier.
    pub tier: KnowledgeTier,

    /// Decay state at time of backup.
    pub decay: DecayState,

    /// Provenance chain: how this Engram was created and validated.
    pub provenance: Vec<ProvenanceEntry>,

    /// Creation timestamp.
    pub created_at: u64,

    /// Last accessed timestamp.
    pub last_accessed_at: u64,

    /// Number of times this Engram has been retrieved and used.
    pub retrieval_count: u64,

    /// Number of times this Engram has been validated against ground truth.
    pub validation_count: u64,

    /// Optional: HDC vector (10,240-bit BSC, hex-encoded).
    /// Used for cross-domain similarity matching.
    pub hdc_vector: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum EngramKind {
    Insight,
    Heuristic,
    Warning,
    CausalLink,
    StrategyFragment,
    AntiKnowledge,
}

/// 7-axis Engram score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngramScore {
    pub confidence: f64,
    pub novelty: f64,
    pub utility: f64,
    pub reputation: f64,
    pub precision: f64,
    pub salience: f64,
    pub coherence: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum KnowledgeTier {
    /// Fast decay: 0.1× base half-life. Recently created, unvalidated.
    Transient,
    /// Moderate decay: 0.5× base half-life. Used but not consolidated.
    Working,
    /// Standard decay: 1.0× base half-life. Validated through experience.
    Consolidated,
    /// Slow decay: 5.0× base half-life. Repeatedly validated, high confidence.
    Persistent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecayState {
    /// Decay model variant.
    pub model: DecayModel,
    /// Current effective confidence after decay.
    pub effective_confidence: f64,
    /// Ticks since last access (for Ebbinghaus computation).
    pub ticks_since_access: u64,
    /// Tier multiplier applied.
    pub tier_multiplier: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DecayModel {
    /// No decay. Confidence remains constant.
    None,
    /// Exponential half-life decay.
    HalfLife { half_life_ms: u64 },
    /// Time-to-live. Binary: alive until TTL, then confidence drops to 0.
    Ttl { expires_at: u64 },
    /// Ebbinghaus forgetting curve: retention = e^(-t / (strength × scale_ms)).
    Ebbinghaus { strength: f64, scale_ms: u64 },
}
```

---

## Backup Integrity

Every backup includes a BLAKE3 checksum of the entire archive. On restore, the checksum is verified before any Engrams are loaded. If verification fails, the restore is aborted with an error.

```rust
/// Verify backup integrity before restore.
pub fn verify_backup(path: &Path) -> Result<BackupManifest, BackupError> {
    let archive = read_archive(path)?;
    let computed_hash = blake3::hash(&archive.raw_bytes);
    let expected_hash = read_checksum(path)?;

    if computed_hash != expected_hash {
        return Err(BackupError::IntegrityCheckFailed {
            expected: expected_hash.to_string(),
            computed: computed_hash.to_string(),
        });
    }

    parse_manifest(&archive)
}
```

---

## Backup Policies

### Automatic Backups

Operators can configure automatic periodic backups:

```toml
[neuro.backup]
# Automatic backup schedule (cron syntax)
schedule = "0 */6 * * *"      # Every 6 hours
# Maximum number of backups to retain
max_backups = 10
# Backup location
path = ".roko/backups/"
# Compression
compress = true
```

Automatic backups are belt-and-suspenders insurance. They ensure that even if the operator forgets to back up before deleting an agent, recent knowledge is available.

### Mesh-Synced Backups

For agents connected to the Agent Mesh, Neuro snapshots can be synced to the Mesh relay for off-site storage:

```toml
[neuro.backup]
mesh_sync = true               # Sync backups to Mesh relay
mesh_sync_interval_hours = 6   # Sync every 6 hours
```

The Mesh relay stores the most recent 5 snapshots per agent. If both the agent and local storage are lost, the operator can restore from the Mesh-synced backup.

---

## What Is NOT Backed Up

The backup captures the Neuro store (Engrams, scores, tiers, decay state, provenance). It does NOT capture:

- **Agent process state**: Running tasks, in-flight requests, temporary variables
- **Daimon state**: Current PAD vector, behavioral state (these are transient by design)
- **Dream journal**: Current dream cycle state (dreams are consolidated into Engrams)
- **Mesh connections**: Peer topology, gossip state (re-established on reconnect)
- **Configuration**: `roko.toml`, `STRATEGY.md` (these are operator-managed files, backed up separately)

The rationale: the backup captures knowledge (what the agent has learned), not state (what the agent is doing). A new agent created from a backup starts with the predecessor's knowledge but its own fresh operational state — including a fresh Daimon state that will adapt to current conditions rather than carrying over potentially stale emotional context.

---

## Genomic Bottleneck: Compressed Backups

For situations where the full backup is too large or where the operator wants to transfer only the most valuable knowledge, the compressed backup mode applies the genomic bottleneck principle from Shuvaev et al. (2024):

```bash
# Compressed backup: at most 2048 Engrams, selected by quality
roko neuro backup --compressed --max-engrams 2048
```

The compression algorithm:

1. **25% reserved** for priority inclusions: all Warning-type Engrams (critical safety knowledge) and all Persistent-tier Engrams (repeatedly validated, highest confidence)
2. **50% allocated** to diversity-sampled top Engrams across all knowledge types — ensuring coverage across Insight, Heuristic, CausalLink, StrategyFragment, and AntiKnowledge
3. **25% filled** with the highest-scored Engrams regardless of type — raw quality selection

This mirrors the biological genomic bottleneck: the human genome is approximately 1,000× smaller than the information required to specify brain connectivity, yet organisms are born with sophisticated innate behaviors. The genome encodes compressed rules for generating circuits, not the circuits themselves. Critically, neural networks compressed through a genomic-scale bottleneck exhibit **enhanced transfer learning to novel tasks** (Shuvaev et al. 2024) — the compression is a regularizer that strips regime-specific overfitting while preserving generalizable knowledge.

---

## Backup as Knowledge Artifact

Backups are portable knowledge artifacts that can be:

1. **Restored** into a new agent (see `docs/17-lifecycle/08-selective-restore.md`)
2. **Shared** with other operators via the Agent Mesh
3. **Inspected** by the operator to audit what the agent has learned
4. **Compared** across backups to track knowledge evolution over time
5. **Archived** for regulatory compliance (content-addressed causal replay)

The backup format is designed so that a fresh agent reading just the backup can understand the knowledge context without any other files. Each Engram is self-contained with its provenance chain.

---

## Related Topics

- `docs/17-lifecycle/06-agent-deletion.md` — Clean shutdown before backup
- `docs/17-lifecycle/07-new-agent-creation.md` — Creating a fresh agent for restore
- `docs/17-lifecycle/08-selective-restore.md` — Selective knowledge import
- `docs/03-neuro/INDEX.md` — Neuro store, Engram format, tier management
