//! Resource profiles and usage reporting for `mirage-rs`.
//!
//! Memory pressure tiers (see [`ResourceUsage::is_warning`], [`ResourceUsage::is_throttled`],
//! [`ResourceUsage::is_emergency`]) use ratio `used / profile.max_memory_bytes` clamped to
//! **[0.0, 1.0]**:
//! - **Below 0.5** — normal ([`PressureAction::None`])
//! - **0.5+** — warning ([`PressureAction::EvictCache`]: shrink read cache toward half capacity)
//! - **0.7+** — throttle ([`PressureAction::Throttle`]: stronger LRU eviction, demote auto-protocol watches)
//! - **0.9+** — emergency ([`PressureAction::DemoteToProxy`]: clear read cache, proxy mode)
//!
//! [`Profile`] names follow the plan envelope (**Micro** / **Standard** / **Power**), not Dev/Production.

use std::{num::NonZeroUsize, time::Duration};

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::{MirageError, Result};

const MICRO_MEMORY_BYTES: u64 = 256 * 1024 * 1024;
const STANDARD_MEMORY_BYTES: u64 = 512 * 1024 * 1024;
const POWER_MEMORY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Max watched contracts for [`Profile::Micro`] (plan envelope).
const MICRO_MAX_WATCHED_CONTRACTS: usize = 32;
/// Max watched contracts for [`Profile::Standard`] (plan envelope).
const STANDARD_MAX_WATCHED_CONTRACTS: usize = 64;
/// Max watched contracts for [`Profile::Power`] (plan envelope).
const POWER_MAX_WATCHED_CONTRACTS: usize = 256;
/// Read-cache entry cap for [`Profile::Micro`].
const MICRO_CACHE_CAPACITY: usize = 5_000;
/// Read-cache entry cap for [`Profile::Standard`].
const STANDARD_CACHE_CAPACITY: usize = 10_000;
/// Read-cache entry cap for [`Profile::Power`].
const POWER_CACHE_CAPACITY: usize = 50_000;
const SPAWN_HEADROOM_BYTES: u64 = 128 * 1024 * 1024;

fn ensure_spawn_budget_from_available_memory(required: u64, available: u64) -> Result<()> {
    if available == 0 || available < required {
        return Err(MirageError::Unsupported(format!(
            "insufficient memory: available={available} required={required}"
        )));
    }
    Ok(())
}

/// Runtime operating mode for the fork.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MirageMode {
    /// Live lazy-read mode with optional targeted follower support.
    Live,
    /// Historical pinned-block mode.
    Historical,
    /// Proxy-only mode with replay disabled due to pressure.
    Proxy,
}

/// Predefined resource envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Profile {
    /// 256 MB / 32 watched contracts / 5,000 cache entries.
    Micro,
    /// 512 MB / 64 watched contracts / 10,000 cache entries.
    #[default]
    Standard,
    /// 2 GB / 256 watched contracts / 50,000 cache entries.
    Power,
}

/// Static resource model used for startup checks and runtime pressure handling.
#[derive(Debug, Clone)]
pub struct ResourceModel {
    /// Selected resource profile.
    pub profile: Profile,
    /// Hard memory ceiling in bytes.
    pub max_memory_bytes: u64,
    /// Maximum number of watched contracts.
    pub max_watched_contracts: usize,
    /// Read-cache entry capacity.
    pub cache_capacity: usize,
    /// Read-cache TTL.
    pub cache_ttl: Duration,
}

impl ResourceModel {
    /// Creates a model from a named profile.
    #[must_use]
    pub fn for_profile(profile: Profile, cache_ttl: Duration) -> Self {
        match profile {
            Profile::Micro => Self {
                profile,
                max_memory_bytes: MICRO_MEMORY_BYTES,
                max_watched_contracts: MICRO_MAX_WATCHED_CONTRACTS,
                cache_capacity: MICRO_CACHE_CAPACITY,
                cache_ttl,
            },
            Profile::Standard => Self {
                profile,
                max_memory_bytes: STANDARD_MEMORY_BYTES,
                max_watched_contracts: STANDARD_MAX_WATCHED_CONTRACTS,
                cache_capacity: STANDARD_CACHE_CAPACITY,
                cache_ttl,
            },
            Profile::Power => Self {
                profile,
                max_memory_bytes: POWER_MEMORY_BYTES,
                max_watched_contracts: POWER_MAX_WATCHED_CONTRACTS,
                cache_capacity: POWER_CACHE_CAPACITY,
                cache_ttl,
            },
        }
    }

    /// Returns the derived bytecode cache capacity.
    #[must_use]
    pub fn bytecode_cache_capacity(&self) -> NonZeroUsize {
        NonZeroUsize::new((self.cache_capacity / 5).clamp(1, 10_000)).unwrap_or(NonZeroUsize::MIN)
    }

    /// Computes resource pressure from a memory sample.
    ///
    /// Returns `memory_bytes / max_memory_bytes` clamped to **[0.0, 1.0]** (INV-020 / plan).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn pressure_for_memory(&self, memory_bytes: u64) -> f64 {
        if self.max_memory_bytes == 0 {
            return 0.0;
        }
        (memory_bytes as f64 / self.max_memory_bytes as f64).clamp(0.0, 1.0)
    }

    /// Checks whether the process should be allowed to start.
    ///
    /// # Errors
    ///
    /// Returns [`MirageError::Unsupported`] if the available memory is known
    /// and does not satisfy the configured memory budget plus headroom.
    pub fn ensure_spawn_budget(&self) -> Result<()> {
        let mut system = System::new();
        system.refresh_memory();
        let available = std::env::var("BARDO_AVAILABLE_MEMORY_BYTES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or_else(|| system.available_memory());
        // sysinfo returns 0 when it cannot read available memory (e.g. on macOS
        // when the Mach host_statistics64 call fails).  Treat 0 as unknown and
        // skip the check rather than blocking startup on a machine with plenty
        // of RAM.
        if available == 0 {
            return Ok(());
        }
        let required = self.max_memory_bytes.saturating_add(SPAWN_HEADROOM_BYTES);
        ensure_spawn_budget_from_available_memory(required, available)
    }

    /// Captures the current process memory footprint.
    #[must_use]
    pub fn current_process_memory_bytes() -> u64 {
        let mut system = System::new_all();
        let pid = Pid::from_u32(std::process::id());
        system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        system.process(pid).map_or(0, sysinfo::Process::memory)
    }
}

/// Runtime pressure response selected from the latest resource sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureAction {
    /// No runtime changes are needed.
    None,
    /// Trim caches and keep the fork live.
    EvictCache,
    /// Trim caches and demote auto-classified contracts to slot-only reads.
    Throttle,
    /// Trim caches and demote runtime mode to proxy.
    DemoteToProxy,
}

impl ResourceUsage {
    /// Returns the runtime action implied by this usage sample.
    #[must_use]
    pub fn pressure_action(&self) -> PressureAction {
        if self.is_emergency() {
            PressureAction::DemoteToProxy
        } else if self.is_throttled() {
            PressureAction::Throttle
        } else if self.is_warning() {
            PressureAction::EvictCache
        } else {
            PressureAction::None
        }
    }
}

/// Snapshot of the runtime's current resource usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceUsage {
    /// Resident memory estimate for the current process.
    pub memory_bytes: u64,
    /// Configured memory limit for the current profile.
    pub memory_limit_bytes: u64,
    /// Pressure score in the inclusive range `[0.0, 1.0+]`.
    pub resource_pressure: f64,
    /// Read-cache hit rate across the process lifetime.
    pub cache_hit_rate: f64,
    /// Current read-cache entry count.
    pub cache_entries: usize,
    /// Configured read-cache capacity.
    pub cache_capacity: usize,
    /// Number of watched contracts.
    pub watch_list_size: usize,
    /// Number of locally dirty storage slots.
    pub dirty_slot_count: u64,
    /// Total upstream RPC calls.
    pub upstream_rpc_calls: u64,
    /// Total upstream RPC errors.
    pub upstream_rpc_errors: u64,
    /// Current operating mode.
    pub mode: MirageMode,
    /// Estimated disk usage for artifacts in bytes.
    pub disk_usage_bytes: u64,
}

impl ResourceUsage {
    /// Builds a usage snapshot.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        model: &ResourceModel,
        memory_bytes: u64,
        cache_hit_rate: f64,
        cache_entries: usize,
        watch_list_size: usize,
        dirty_slot_count: u64,
        upstream_rpc_calls: u64,
        upstream_rpc_errors: u64,
        mode: MirageMode,
        disk_usage_bytes: u64,
    ) -> Self {
        Self {
            memory_bytes,
            memory_limit_bytes: model.max_memory_bytes,
            resource_pressure: model.pressure_for_memory(memory_bytes),
            cache_hit_rate,
            cache_entries,
            cache_capacity: model.cache_capacity,
            watch_list_size,
            dirty_slot_count,
            upstream_rpc_calls,
            upstream_rpc_errors,
            mode,
            disk_usage_bytes,
        }
    }

    /// Returns whether the usage has entered the warning tier.
    #[must_use]
    pub fn is_warning(&self) -> bool {
        self.resource_pressure >= 0.5
    }

    /// Returns whether the usage has entered the throttle tier.
    #[must_use]
    pub fn is_throttled(&self) -> bool {
        self.resource_pressure >= 0.7
    }

    /// Returns whether the usage has entered the emergency tier.
    #[must_use]
    pub fn is_emergency(&self) -> bool {
        self.resource_pressure >= 0.9
    }
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, sync::Arc, time::Duration};

    use alloy_primitives::{Address, U256, address};

    use super::{PressureAction, Profile, ResourceModel, ResourceUsage};
    use crate::provider::UpstreamRpc;
    use crate::resources::MirageMode;
    use crate::{AccountInfo, Bytecode, fork::HybridDB, fork::ReadCache};

    #[test]
    fn pressure_tiers() {
        let model =
            ResourceModel::for_profile(Profile::Standard, std::time::Duration::from_secs(12));
        let idle = ResourceUsage::new(
            &model,
            64 * 1024 * 1024,
            0.8,
            10,
            0,
            0,
            0,
            0,
            MirageMode::Live,
            0,
        );
        assert!(!idle.is_warning());

        let warning = ResourceUsage::new(
            &model,
            300 * 1024 * 1024,
            0.8,
            10,
            0,
            0,
            0,
            0,
            MirageMode::Live,
            0,
        );
        assert!(warning.is_warning());
        assert!(!warning.is_throttled());

        let throttle = ResourceUsage::new(
            &model,
            380 * 1024 * 1024,
            0.8,
            10,
            0,
            0,
            0,
            0,
            MirageMode::Live,
            0,
        );
        assert!(throttle.is_throttled());

        let emergency = ResourceUsage::new(
            &model,
            470 * 1024 * 1024,
            0.8,
            10,
            0,
            0,
            0,
            0,
            MirageMode::Live,
            0,
        );
        assert!(emergency.is_emergency());
        assert_eq!(warning.pressure_action(), PressureAction::EvictCache);
        assert_eq!(throttle.pressure_action(), PressureAction::Throttle);
        assert_eq!(emergency.pressure_action(), PressureAction::DemoteToProxy);
    }

    #[test]
    fn lru_eviction_at_capacity() {
        let mut cache = ReadCache::new(2, Duration::from_secs(12));
        let info = AccountInfo {
            balance: U256::from(1_u64),
            nonce: 0,
            code_hash: Bytecode::default().hash_slow(),
            code: Some(Bytecode::default()),
        };

        cache.insert_account(
            address!("0x1000000000000000000000000000000000000001"),
            info.clone(),
        );
        cache.insert_account(
            address!("0x1000000000000000000000000000000000000002"),
            info.clone(),
        );
        cache.insert_account(address!("0x1000000000000000000000000000000000000003"), info);

        assert_eq!(cache.entry_count(), 2);
    }

    /// INV-028 / throttle path: at ≥70% memory pressure the runtime selects
    /// [`PressureAction::Throttle`]; read-cache rows are shed via LRU
    /// (`ReadCache::evict_to`) toward a lower target.
    #[test]
    fn test_lru_eviction_at_throttle_tier() {
        let model = ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12));
        let throttled = ResourceUsage::new(
            &model,
            360 * 1024 * 1024,
            0.5,
            8,
            0,
            0,
            0,
            0,
            MirageMode::Live,
            0,
        );
        assert!(throttled.is_throttled());
        assert_eq!(throttled.pressure_action(), PressureAction::Throttle);

        let mut cache = ReadCache::new(32, Duration::from_secs(12));
        let info = AccountInfo {
            balance: U256::from(1_u64),
            nonce: 0,
            code_hash: Bytecode::default().hash_slow(),
            code: Some(Bytecode::default()),
        };
        for i in 1_u8..=6 {
            let addr = Address::from_slice(&[i; 20]);
            cache.insert_account(addr, info.clone());
        }
        assert_eq!(cache.entry_count(), 6);
        cache.evict_to(2);
        assert_eq!(cache.entry_count(), 2);

        // Throttle handling must only shrink the read cache; [`DirtyStore`] is untouched.
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let addr = address!("0x51000000000000000000000000000000000000aa");
        upstream.set_mock_account(
            addr,
            AccountInfo {
                balance: U256::from(1_u64),
                nonce: 0,
                code_hash: Bytecode::default().hash_slow(),
                code: Some(Bytecode::default()),
            },
        );
        upstream.set_mock_storage(addr, U256::from(1_u64), U256::from(42_u64));
        let bytecode_cap = NonZeroUsize::new(64).expect("nonzero");
        let mut db = HybridDB::new(upstream, 32, Duration::from_secs(60), bytecode_cap, 1);
        db.set_storage(addr, U256::from(99_u64), U256::from(7_u64));
        let dirty_accounts_before = db.dirty.accounts.len();
        let dirty_slots_before = db.dirty.total_dirty_slots;
        db.storage(addr, U256::from(1_u64))
            .expect("populate read cache from upstream");
        let _ = db.basic(addr);
        assert!(db.read_cache.entry_count() > 0);
        db.evict_read_cache_to(0);
        assert_eq!(db.dirty.accounts.len(), dirty_accounts_before);
        assert_eq!(db.dirty.total_dirty_slots, dirty_slots_before);
        assert_eq!(
            db.storage(addr, U256::from(99_u64))
                .expect("dirty slot still readable"),
            U256::from(7_u64)
        );
    }

    #[test]
    fn zero_available_memory_is_rejected() {
        let err = super::ensure_spawn_budget_from_available_memory(640 * 1024 * 1024, 0)
            .expect_err("zero available memory should fail");

        assert!(err.to_string().contains("available=0"));
    }

    #[test]
    fn test_resource_pressure_bounds() {
        let model = ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12));
        assert_eq!(model.pressure_for_memory(0), 0.0);
        assert!(model.pressure_for_memory(256 * 1024 * 1024) >= 0.0);
        assert!(model.pressure_for_memory(256 * 1024 * 1024) <= 1.0);
        // Over the cap: clamped to 1.0 (INV-020)
        let over = model.pressure_for_memory(2 * 1024 * 1024 * 1024);
        assert!((over - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_resource_pressure_tier_transitions() {
        let model = ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12));
        // Standard = 512 MB. 50% = 256 MB, 70% = 358 MB, 90% = 460 MB.
        let below_warning = ResourceUsage::new(
            &model,
            200 * 1024 * 1024,
            0.8,
            10,
            0,
            0,
            0,
            0,
            MirageMode::Live,
            0,
        );
        assert!(!below_warning.is_warning());
        assert_eq!(below_warning.pressure_action(), PressureAction::None);

        let at_warning = ResourceUsage::new(
            &model,
            256 * 1024 * 1024,
            0.8,
            10,
            0,
            0,
            0,
            0,
            MirageMode::Live,
            0,
        );
        assert!(at_warning.is_warning());
        assert!(!at_warning.is_throttled());
        assert_eq!(at_warning.pressure_action(), PressureAction::EvictCache);

        let at_throttle = ResourceUsage::new(
            &model,
            360 * 1024 * 1024,
            0.8,
            10,
            0,
            0,
            0,
            0,
            MirageMode::Live,
            0,
        );
        assert!(at_throttle.is_throttled());
        assert!(!at_throttle.is_emergency());
        assert_eq!(at_throttle.pressure_action(), PressureAction::Throttle);

        let at_emergency = ResourceUsage::new(
            &model,
            470 * 1024 * 1024,
            0.8,
            10,
            0,
            0,
            0,
            0,
            MirageMode::Live,
            0,
        );
        assert!(at_emergency.is_emergency());
        assert_eq!(
            at_emergency.pressure_action(),
            PressureAction::DemoteToProxy
        );
    }

    #[test]
    fn test_pressure_tiers() {
        let model = ResourceModel::for_profile(Profile::Micro, Duration::from_secs(12));
        // Micro = 256 MB.
        let idle = ResourceUsage::new(
            &model,
            50 * 1024 * 1024,
            0.9,
            5,
            0,
            0,
            0,
            0,
            MirageMode::Live,
            0,
        );
        assert_eq!(idle.pressure_action(), PressureAction::None);

        let warning = ResourceUsage::new(
            &model,
            130 * 1024 * 1024,
            0.9,
            5,
            0,
            0,
            0,
            0,
            MirageMode::Live,
            0,
        );
        assert_eq!(warning.pressure_action(), PressureAction::EvictCache);

        let emergency = ResourceUsage::new(
            &model,
            240 * 1024 * 1024,
            0.9,
            5,
            0,
            0,
            0,
            0,
            MirageMode::Live,
            0,
        );
        assert_eq!(emergency.pressure_action(), PressureAction::DemoteToProxy);
    }

    #[test]
    fn test_cache_hit_rate_bounds() {
        let mut cache = ReadCache::new(100, Duration::from_secs(60));
        // No lookups: hit rate should be 1.0 (perfect by default)
        assert!((cache.hit_rate() - 1.0).abs() < f64::EPSILON);

        // Insert and hit
        let info = AccountInfo {
            balance: U256::from(1_u64),
            nonce: 0,
            code_hash: Bytecode::default().hash_slow(),
            code: Some(Bytecode::default()),
        };
        let addr = address!("0x1000000000000000000000000000000000000001");
        cache.insert_account(addr, info);
        let _ = cache.get_account(&addr); // hit
        assert!(cache.hit_rate() >= 0.0);
        assert!(cache.hit_rate() <= 1.0);

        // Miss
        let missing = address!("0x2000000000000000000000000000000000000002");
        let _ = cache.get_account(&missing); // miss
        assert!(cache.hit_rate() >= 0.0);
        assert!(cache.hit_rate() <= 1.0);
    }

    #[test]
    fn test_resource_model_profiles_envelope() {
        let ttl = Duration::from_secs(12);
        let micro = ResourceModel::for_profile(Profile::Micro, ttl);
        let standard = ResourceModel::for_profile(Profile::Standard, ttl);
        let power = ResourceModel::for_profile(Profile::Power, ttl);

        assert_eq!(micro.profile, Profile::Micro);
        assert_eq!(micro.max_memory_bytes, super::MICRO_MEMORY_BYTES);
        assert_eq!(
            micro.max_watched_contracts,
            super::MICRO_MAX_WATCHED_CONTRACTS
        );

        assert_eq!(standard.profile, Profile::Standard);
        assert_eq!(standard.max_memory_bytes, super::STANDARD_MEMORY_BYTES);
        assert_eq!(
            standard.max_watched_contracts,
            super::STANDARD_MAX_WATCHED_CONTRACTS
        );

        assert_eq!(power.profile, Profile::Power);
        assert_eq!(power.max_memory_bytes, super::POWER_MEMORY_BYTES);
        assert_eq!(
            power.max_watched_contracts,
            super::POWER_MAX_WATCHED_CONTRACTS
        );
    }

    #[test]
    fn test_profile_memory_allocations() {
        let micro = ResourceModel::for_profile(Profile::Micro, Duration::from_secs(12));
        let standard = ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12));
        let power = ResourceModel::for_profile(Profile::Power, Duration::from_secs(12));

        assert_eq!(micro.max_memory_bytes, super::MICRO_MEMORY_BYTES);
        assert_eq!(standard.max_memory_bytes, super::STANDARD_MEMORY_BYTES);
        assert_eq!(power.max_memory_bytes, super::POWER_MEMORY_BYTES);
    }

    #[test]
    fn test_profile_watch_limits() {
        let micro = ResourceModel::for_profile(Profile::Micro, Duration::from_secs(12));
        let standard = ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12));
        let power = ResourceModel::for_profile(Profile::Power, Duration::from_secs(12));

        assert_eq!(
            micro.max_watched_contracts,
            super::MICRO_MAX_WATCHED_CONTRACTS
        );
        assert_eq!(
            standard.max_watched_contracts,
            super::STANDARD_MAX_WATCHED_CONTRACTS
        );
        assert_eq!(
            power.max_watched_contracts,
            super::POWER_MAX_WATCHED_CONTRACTS
        );
    }

    #[test]
    fn test_profile_cache_capacities() {
        let micro = ResourceModel::for_profile(Profile::Micro, Duration::from_secs(12));
        let standard = ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12));
        let power = ResourceModel::for_profile(Profile::Power, Duration::from_secs(12));

        assert_eq!(micro.cache_capacity, super::MICRO_CACHE_CAPACITY);
        assert_eq!(standard.cache_capacity, super::STANDARD_CACHE_CAPACITY);
        assert_eq!(power.cache_capacity, super::POWER_CACHE_CAPACITY);

        // Bytecode cache is cache_capacity / 5
        assert_eq!(micro.bytecode_cache_capacity().get(), 1_000);
        assert_eq!(standard.bytecode_cache_capacity().get(), 2_000);
        assert_eq!(power.bytecode_cache_capacity().get(), 10_000);
    }

    #[test]
    fn test_bytecode_cache_capacity() {
        for profile in [Profile::Micro, Profile::Standard, Profile::Power] {
            let model = ResourceModel::for_profile(profile, Duration::from_secs(12));
            let expected = (model.cache_capacity / 5).clamp(1, 10_000);
            assert_eq!(model.bytecode_cache_capacity().get(), expected);
            assert!(model.bytecode_cache_capacity().get() <= 10_000);
            assert!(model.bytecode_cache_capacity().get() <= model.cache_capacity);
        }
    }

    #[test]
    fn test_memory_startup_gate() {
        let model = ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12));
        let required = model.max_memory_bytes + super::SPAWN_HEADROOM_BYTES;

        super::ensure_spawn_budget_from_available_memory(required, required)
            .unwrap_or_else(|error| panic!("exact startup budget should pass: {error}"));

        let error =
            super::ensure_spawn_budget_from_available_memory(required, required.saturating_sub(1))
                .expect_err("startup should fail when available memory is below the required gate");
        let message = error.to_string();
        assert!(message.contains("insufficient memory"));
        assert!(message.contains(&format!("required={required}")));
    }
}
