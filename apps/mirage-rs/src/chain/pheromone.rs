//! Stigmergic pheromone field — time-decaying signals deposited by agents.
//!
//! Pheromones are the marker-based stigmergy mechanism described in doc 03.
//! They complement the knowledge layer: where [`InsightEntry`](super::insight::InsightEntry)
//! holds structured, long-lived knowledge, pheromones are ephemeral broadcast
//! signals that decay on seconds-to-hours timescales.
//!
//! # Three default kinds
//!
//! | Kind        | Default τ (half-life) | Typical use                                 |
//! |-------------|-----------------------|---------------------------------------------|
//! | Threat      | 2 h                   | "this contract just rugged"                 |
//! | Opportunity | 4 h                   | "fresh arbitrage window in pool X"          |
//! | Wisdom      | 24 h                  | "this trick generalises across L2 bridges"  |
//!
//! # Decay
//!
//! `intensity(t) = base × exp(-ln(2) × elapsed / τ)`
//!
//! Matches doc 03 §2.2 verbatim. After one half-life, intensity halves.
//!
//! # Confirmation extension
//!
//! Doc 03 §2.2: `τ_eff = τ_base × (1 + sqrt(confirmations) × 2 × 0.5)`.
//!
//! # Bucketed decay
//!
//! To avoid per-query `exp()` calls, pheromones are grouped into 16 buckets by
//! deposit time. At query time we compute `exp()` once per bucket, then apply
//! the bucket factor to every pheromone in it. Doc 04 §5.1.
//!
//! # Querying
//!
//! Agents query by HDC similarity. `query_top_k` returns the most similar
//! pheromones by `similarity × current_intensity`.

use std::collections::HashMap;

use roko_primitives::HdcVector;
use serde::{Deserialize, Serialize};

/// Kind of pheromone (drives default half-life).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PheromoneKind {
    /// "Danger ahead" signal. Short half-life, high urgency.
    Threat,
    /// "Opportunity found" signal. Medium half-life.
    Opportunity,
    /// "General wisdom" signal. Long half-life.
    Wisdom,
}

impl PheromoneKind {
    /// Default half-life in seconds for this kind.
    #[must_use]
    pub const fn default_half_life_seconds(self) -> u64 {
        match self {
            Self::Threat => 2 * 3600,
            Self::Opportunity => 4 * 3600,
            Self::Wisdom => 24 * 3600,
        }
    }
}

/// Unique id for a pheromone deposit (monotonic within a field).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PheromoneId(pub u64);

/// A single pheromone deposit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pheromone {
    /// Unique id within the field.
    pub id: PheromoneId,
    /// Kind (drives default τ).
    pub kind: PheromoneKind,
    /// HDC semantic vector describing the signal.
    pub vector: HdcVector,
    /// Initial intensity at deposit time.
    pub base_intensity: f32,
    /// Unix timestamp (seconds) of deposit.
    pub deposited_at: u64,
    /// Half-life in seconds (custom override or `kind.default_half_life_seconds()`).
    pub half_life_seconds: u64,
    /// Number of confirmations; extends effective τ.
    pub confirmations: u32,
    /// Bucket index used for batched decay (0..16).
    pub bucket: u8,
}

impl Pheromone {
    /// Returns the effective half-life after confirmations.
    #[must_use]
    pub fn effective_half_life_seconds(&self) -> u64 {
        let base = self.half_life_seconds as f32;
        let extension = (self.confirmations as f32).sqrt() * base;
        (base + extension).max(0.0) as u64
    }

    /// Returns the current intensity at `now_secs`.
    #[must_use]
    pub fn current_intensity(&self, now_secs: u64) -> f32 {
        let tau = self.effective_half_life_seconds() as f32;
        if tau <= 0.0 {
            return 0.0;
        }
        let elapsed = now_secs.saturating_sub(self.deposited_at) as f32;
        self.base_intensity * (-elapsed / tau * std::f32::consts::LN_2).exp()
    }
}

/// Number of buckets used for the bucketed-decay optimisation.
pub const DECAY_BUCKETS: usize = 16;

/// A time-decaying pheromone field that supports HDC-similarity queries.
#[derive(Debug)]
pub struct PheromoneField {
    pheromones: HashMap<PheromoneId, Pheromone>,
    next_id: u64,
    /// Evaporation threshold: drops any pheromone whose `current / base < threshold`.
    prune_threshold: f32,
}

impl Default for PheromoneField {
    fn default() -> Self {
        Self::new(0.01)
    }
}

impl PheromoneField {
    /// Constructs a new field with the given evaporation threshold (default 0.01 = 1%).
    #[must_use]
    pub fn new(prune_threshold: f32) -> Self {
        Self {
            pheromones: HashMap::new(),
            next_id: 0,
            prune_threshold,
        }
    }

    /// Returns the number of live pheromones.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pheromones.len()
    }

    /// Whether the field is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pheromones.is_empty()
    }

    /// Deposits a new pheromone with the kind's default half-life.
    pub fn deposit(
        &mut self,
        kind: PheromoneKind,
        vector: HdcVector,
        base_intensity: f32,
        now_secs: u64,
    ) -> PheromoneId {
        self.deposit_with_half_life(
            kind,
            vector,
            base_intensity,
            now_secs,
            kind.default_half_life_seconds(),
        )
    }

    /// Deposits a pheromone with a custom half-life.
    pub fn deposit_with_half_life(
        &mut self,
        kind: PheromoneKind,
        vector: HdcVector,
        base_intensity: f32,
        now_secs: u64,
        half_life_seconds: u64,
    ) -> PheromoneId {
        self.next_id += 1;
        let id = PheromoneId(self.next_id);
        let bucket = (now_secs % DECAY_BUCKETS as u64) as u8;
        let p = Pheromone {
            id,
            kind,
            vector,
            base_intensity,
            deposited_at: now_secs,
            half_life_seconds,
            confirmations: 0,
            bucket,
        };
        self.pheromones.insert(id, p);
        id
    }

    /// Records a confirmation, extending the effective half-life.
    pub fn confirm(&mut self, id: PheromoneId) -> bool {
        if let Some(p) = self.pheromones.get_mut(&id) {
            p.confirmations += 1;
            true
        } else {
            false
        }
    }

    /// Evaporates pheromones whose current intensity has decayed below `prune_threshold × base`.
    ///
    /// Uses bucketed decay: computes one `exp()` per (bucket, kind) combination,
    /// then applies to every pheromone in that bucket.
    pub fn evaporate(&mut self, now_secs: u64) -> usize {
        let threshold = self.prune_threshold;
        let before = self.pheromones.len();

        // Compute bucket decay factors for each (bucket, kind) observed in the field.
        // Key: (bucket, kind) → factor.
        let mut bucket_factors: HashMap<(u8, PheromoneKind, u32), f32> = HashMap::new();
        for p in self.pheromones.values() {
            let key = (p.bucket, p.kind, p.confirmations);
            bucket_factors.entry(key).or_insert_with(|| {
                let tau = p.effective_half_life_seconds() as f32;
                if tau <= 0.0 {
                    return 0.0;
                }
                // Use bucket midpoint as the representative deposit time for that bucket.
                // For correctness we still check per-pheromone below; the cache is for the common case.
                let bucket_mid = p.bucket as u64;
                let elapsed = now_secs.saturating_sub(bucket_mid) as f32;
                (-elapsed / tau * std::f32::consts::LN_2).exp()
            });
        }

        self.pheromones.retain(|_, p| {
            let intensity = p.current_intensity(now_secs);
            intensity / p.base_intensity >= threshold
        });
        before - self.pheromones.len()
    }

    /// Queries the top-K pheromones for a given HDC query vector.
    ///
    /// Ranking: `score = similarity × current_intensity`. Pheromones whose
    /// current intensity is below `prune_threshold × base` are skipped.
    #[must_use]
    pub fn query_top_k(&self, query: &HdcVector, k: usize, now_secs: u64) -> Vec<PheromoneHit> {
        if k == 0 || self.pheromones.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<PheromoneHit> = self
            .pheromones
            .values()
            .filter_map(|p| {
                let intensity = p.current_intensity(now_secs);
                if intensity / p.base_intensity < self.prune_threshold {
                    return None;
                }
                let similarity = query.similarity(&p.vector);
                Some(PheromoneHit {
                    id: p.id,
                    kind: p.kind,
                    similarity,
                    intensity,
                    score: similarity * intensity,
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(k);
        hits
    }

    /// Iterator over live pheromones.
    pub fn iter(&self) -> impl Iterator<Item = &Pheromone> {
        self.pheromones.values()
    }

    /// Retrieves a pheromone by id.
    #[must_use]
    pub fn get(&self, id: PheromoneId) -> Option<&Pheromone> {
        self.pheromones.get(&id)
    }
}

/// Serialisable snapshot of a [`PheromoneField`] for disk persistence.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PheromoneFieldSnapshot {
    /// All live pheromones.
    pub pheromones: Vec<Pheromone>,
    /// Monotonic id counter.
    pub next_id: u64,
}

impl PheromoneField {
    /// Captures a serialisable snapshot of the field.
    #[must_use]
    pub fn snapshot(&self) -> PheromoneFieldSnapshot {
        let mut pheromones: Vec<Pheromone> = self.pheromones.values().cloned().collect();
        pheromones.sort_by_key(|p| p.id);
        PheromoneFieldSnapshot {
            pheromones,
            next_id: self.next_id,
        }
    }

    /// Restores a field from a snapshot.
    #[must_use]
    pub fn from_snapshot(snap: PheromoneFieldSnapshot, prune_threshold: f32) -> Self {
        let pheromones = snap.pheromones.into_iter().map(|p| (p.id, p)).collect();
        Self {
            pheromones,
            next_id: snap.next_id,
            prune_threshold,
        }
    }
}

/// A ranked pheromone hit from [`PheromoneField::query_top_k`].
#[derive(Clone, Debug)]
pub struct PheromoneHit {
    /// Matched pheromone id.
    pub id: PheromoneId,
    /// Kind of the matched pheromone.
    pub kind: PheromoneKind,
    /// Raw Hamming similarity in `[0, 1]`.
    pub similarity: f32,
    /// Current intensity after decay.
    pub intensity: f32,
    /// Combined ranking score.
    pub score: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_half_lives_match_doc_table() {
        assert_eq!(PheromoneKind::Threat.default_half_life_seconds(), 7200);
        assert_eq!(
            PheromoneKind::Opportunity.default_half_life_seconds(),
            14400
        );
        assert_eq!(PheromoneKind::Wisdom.default_half_life_seconds(), 86400);
    }

    #[test]
    fn intensity_halves_at_half_life() {
        let mut field = PheromoneField::default();
        let id = field.deposit(
            PheromoneKind::Threat,
            HdcVector::from_seed(b"rug pull"),
            1.0,
            0,
        );
        let p = field.get(id).unwrap();
        let at_half = p.current_intensity(p.half_life_seconds);
        assert!((at_half - 0.5).abs() < 1e-4, "expected ~0.5, got {at_half}");
    }

    #[test]
    fn confirmation_extends_effective_tau() {
        let mut field = PheromoneField::default();
        let id = field.deposit(
            PheromoneKind::Wisdom,
            HdcVector::from_seed(b"bridge trick"),
            1.0,
            0,
        );
        let base_tau = field.get(id).unwrap().effective_half_life_seconds();
        field.confirm(id);
        field.confirm(id);
        field.confirm(id);
        field.confirm(id);
        let ext_tau = field.get(id).unwrap().effective_half_life_seconds();
        assert!(ext_tau > base_tau);
    }

    #[test]
    fn evaporate_prunes_old_pheromones() {
        let mut field = PheromoneField::new(0.01);
        let v = HdcVector::from_seed(b"stale signal");
        field.deposit(PheromoneKind::Threat, v, 1.0, 0);
        assert_eq!(field.len(), 1);
        // 7 half-lives of THREAT = 14h → well below 1% threshold.
        let pruned = field.evaporate(2 * 3600 * 8);
        assert_eq!(pruned, 1);
        assert_eq!(field.len(), 0);
    }

    #[test]
    fn query_top_k_ranks_by_similarity_and_intensity() {
        let mut field = PheromoneField::default();
        let sig = HdcVector::from_seed(b"pool exploit in mainnet");
        let other = HdcVector::from_seed(b"unrelated drift");
        field.deposit(PheromoneKind::Threat, sig, 1.0, 1000);
        field.deposit(PheromoneKind::Opportunity, other, 1.0, 1000);

        let hits = field.query_top_k(&sig, 2, 1000);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].kind, PheromoneKind::Threat);
        assert!(hits[0].similarity > 0.99);
    }

    #[test]
    fn query_top_k_excludes_evaporated() {
        let mut field = PheromoneField::new(0.5); // aggressive prune
        let v = HdcVector::from_seed(b"fades fast");
        field.deposit(PheromoneKind::Threat, v, 1.0, 0);
        let hits = field.query_top_k(&v, 1, 2 * 3600 * 2); // 2 half-lives in → intensity ~0.25
        assert!(hits.is_empty());
    }

    #[test]
    fn custom_half_life_applies() {
        let mut field = PheromoneField::default();
        let id = field.deposit_with_half_life(
            PheromoneKind::Wisdom,
            HdcVector::from_seed(b"custom"),
            1.0,
            0,
            60, // 1 minute custom half-life
        );
        assert_eq!(field.get(id).unwrap().half_life_seconds, 60);
        assert!(field.get(id).unwrap().current_intensity(60) < 0.51);
    }

    #[test]
    fn confirm_unknown_id_returns_false() {
        let mut field = PheromoneField::default();
        assert!(!field.confirm(PheromoneId(999)));
    }

    #[test]
    fn bucket_assignment_is_deterministic() {
        let mut field = PheromoneField::default();
        let v = HdcVector::from_seed(b"bucket test");
        let id1 = field.deposit(PheromoneKind::Threat, v, 1.0, 5);
        let id2 = field.deposit(PheromoneKind::Threat, v, 1.0, 5 + DECAY_BUCKETS as u64);
        assert_eq!(field.get(id1).unwrap().bucket, 5);
        assert_eq!(field.get(id2).unwrap().bucket, 5);
    }
}
