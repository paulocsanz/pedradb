//! RAM pressure watermarks and backpressure admission (OOM prevention).
//!
//! Under high memory usage:
//! 1. NEVER crash with OOM (Out Of Memory).
//! 2. First, aggressively evict read caches (SST decoded entries, payload pool,
//!    retired memtables, block cache).
//! 3. When memory reaches the hard watermark, backpressure/throttle writers
//!    and flush memtables to disk ("slow down, never OOM").

#![forbid(unsafe_code)]

/// Default soft watermark ratio numerator: 70% of max budget.
pub const DEFAULT_RAM_SOFT_NUM: u64 = 7;
/// Default soft watermark ratio denominator: 10.
pub const DEFAULT_RAM_SOFT_DENOM: u64 = 10;

/// Default hard watermark ratio numerator: 90% of max budget.
pub const DEFAULT_RAM_HARD_NUM: u64 = 9;
/// Default hard watermark ratio denominator: 10.
pub const DEFAULT_RAM_HARD_DENOM: u64 = 10;

/// Action to take on write admission under RAM pressure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RamPressureVerdict {
    /// Normal operating limits: RAM is below soft watermark. Proceed immediately.
    Admit,
    /// RAM is between soft and hard watermarks: evict caches, stage/flush dirty memory.
    EvictCaches,
    /// RAM is at or above hard watermark: throttle writer, aggressively flush and reclaim.
    ThrottleWriter,
}

/// Calculate soft and hard watermarks for a given budget in bytes.
#[must_use]
pub fn ram_watermarks(budget: u64) -> (u64, u64) {
    if budget == 0 {
        return (0, 0);
    }
    let soft = budget
        .saturating_mul(DEFAULT_RAM_SOFT_NUM)
        .checked_div(DEFAULT_RAM_SOFT_DENOM)
        .unwrap_or(0);
    let hard = budget
        .saturating_mul(DEFAULT_RAM_HARD_NUM)
        .checked_div(DEFAULT_RAM_HARD_DENOM)
        .unwrap_or(0);
    (soft, hard)
}

/// Evaluate RAM pressure verdict given current RAM usage and configured budget.
///
/// If `budget` is `None` or `0`, RAM pressure admission is inactive (returns `Admit`).
#[must_use]
pub fn ram_pressure_admit(current_bytes: u64, budget: Option<u64>) -> RamPressureVerdict {
    let Some(budget) = budget else {
        return RamPressureVerdict::Admit;
    };
    if budget == 0 {
        return RamPressureVerdict::Admit;
    }
    let (soft, hard) = ram_watermarks(budget);
    if current_bytes >= hard {
        RamPressureVerdict::ThrottleWriter
    } else if current_bytes >= soft {
        RamPressureVerdict::EvictCaches
    } else {
        RamPressureVerdict::Admit
    }
}

/// AS-IS comparison: legacy behavior without RAM backpressure always admits (OOMs).
#[must_use]
pub fn ram_pressure_admit_as_is(_current_bytes: u64, _budget: Option<u64>) -> RamPressureVerdict {
    RamPressureVerdict::Admit
}

/// Specific reclaim actions to take under RAM pressure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RamReclaimPlan {
    /// Evict cached decoded entries across all live SST tables (`clear_entries_cache`).
    pub evict_sst_entries: bool,
    /// Evict resident SST file bodies from the payload pool (`evict_all`).
    pub evict_payload_pool: bool,
    /// Evict block LRU cache (`clear`).
    pub evict_block_cache: bool,
    /// Evict point query answers cache (`clear`).
    pub evict_point_cache: bool,
    /// Drop retired memtable read accelerators (`retired_pending`, `retired_fold`).
    pub evict_retired_mem: bool,
    /// Trigger flush of dirty memtables to disk.
    pub flush_memtable: bool,
    /// Backpressure / throttle writer threads with sleep / yield.
    pub throttle_writer: bool,
}

/// Determine the reclaim plan for a given verdict.
#[must_use]
pub fn ram_reclaim_plan(verdict: RamPressureVerdict) -> RamReclaimPlan {
    match verdict {
        RamPressureVerdict::Admit => RamReclaimPlan {
            evict_sst_entries: false,
            evict_payload_pool: false,
            evict_block_cache: false,
            evict_point_cache: false,
            evict_retired_mem: false,
            flush_memtable: false,
            throttle_writer: false,
        },
        RamPressureVerdict::EvictCaches => RamReclaimPlan {
            evict_sst_entries: true,
            evict_payload_pool: true,
            evict_block_cache: false,
            evict_point_cache: true,
            evict_retired_mem: true,
            flush_memtable: false,
            throttle_writer: false,
        },
        RamPressureVerdict::ThrottleWriter => RamReclaimPlan {
            evict_sst_entries: true,
            evict_payload_pool: true,
            evict_block_cache: true,
            evict_point_cache: true,
            evict_retired_mem: true,
            flush_memtable: true,
            throttle_writer: true,
        },
    }
}

/// Predicate: whether to log a throttled admission warning (decimated to every 1000 events).
#[must_use]
pub fn should_log_throttle(count: u64) -> bool {
    count % 1000 == 1
}

/// Resolve the effective RAM budget from configured options, environment variable,
/// or physical system memory.
#[must_use]
pub fn resolve_ram_budget(configured: Option<usize>, physical_bytes: Option<u64>) -> Option<usize> {
    if let Ok(env_val) = std::env::var("PEDRA_MAX_RAM_BYTES") {
        if let Ok(val) = env_val.parse::<usize>() {
            if val > 0 {
                return Some(val);
            }
        }
    }
    if let Some(c) = configured {
        if c > 0 {
            return Some(c);
        }
    }
    // Auto-derive from physical RAM if available: allocate up to 60% of physical RAM,
    // clamped to at least 256 MiB.
    if let Some(phys) = physical_bytes {
        if phys > 0 {
            let auto = (phys.saturating_mul(6) / 10).max(256 * 1024 * 1024);
            if let Ok(auto_usize) = usize::try_from(auto) {
                return Some(auto_usize);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_watermarks_standard() {
        let budget = 100_000_000u64;
        let (soft, hard) = ram_watermarks(budget);
        assert_eq!(soft, 70_000_000);
        assert_eq!(hard, 90_000_000);
    }

    #[test]
    fn ram_watermarks_zero() {
        let (soft, hard) = ram_watermarks(0);
        assert_eq!(soft, 0);
        assert_eq!(hard, 0);
    }

    #[test]
    fn ram_pressure_admit_transitions() {
        let budget = Some(1000u64);
        assert_eq!(ram_pressure_admit(500, budget), RamPressureVerdict::Admit);
        assert_eq!(ram_pressure_admit(699, budget), RamPressureVerdict::Admit);
        assert_eq!(ram_pressure_admit(700, budget), RamPressureVerdict::EvictCaches);
        assert_eq!(ram_pressure_admit(899, budget), RamPressureVerdict::EvictCaches);
        assert_eq!(ram_pressure_admit(900, budget), RamPressureVerdict::ThrottleWriter);
        assert_eq!(ram_pressure_admit(1500, budget), RamPressureVerdict::ThrottleWriter);
    }

    #[test]
    fn ram_pressure_admit_none_budget() {
        assert_eq!(ram_pressure_admit(1_000_000_000, None), RamPressureVerdict::Admit);
        assert_eq!(ram_pressure_admit(1_000_000_000, Some(0)), RamPressureVerdict::Admit);
    }

    #[test]
    fn ram_reclaim_plan_actions() {
        let admit_plan = ram_reclaim_plan(RamPressureVerdict::Admit);
        assert!(!admit_plan.evict_sst_entries);
        assert!(!admit_plan.throttle_writer);

        let evict_plan = ram_reclaim_plan(RamPressureVerdict::EvictCaches);
        assert!(evict_plan.evict_sst_entries);
        assert!(evict_plan.evict_payload_pool);
        assert!(evict_plan.evict_retired_mem);
        assert!(!evict_plan.throttle_writer);

        let throttle_plan = ram_reclaim_plan(RamPressureVerdict::ThrottleWriter);
        assert!(throttle_plan.evict_sst_entries);
        assert!(throttle_plan.evict_payload_pool);
        assert!(throttle_plan.evict_block_cache);
        assert!(throttle_plan.flush_memtable);
        assert!(throttle_plan.throttle_writer);
    }

    #[test]
    fn resolve_ram_budget_logic() {
        assert_eq!(resolve_ram_budget(Some(12345), None), Some(12345));
        let phys = 10_000_000_000u64; // 10 GB
        let resolved = resolve_ram_budget(None, Some(phys));
        assert_eq!(resolved, Some(6_000_000_000));
    }
}
