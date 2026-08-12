//! Optional buggify annotation sites (RFC-0018 P2.5 / confidence C1.4).
//!
//! **Off in production default:** without `feature = "buggify"`, [`maybe_arm`] is a
//! pure no-op. With the feature, a process-local [`BuggifyTable`] (SeedRng-derived)
//! decides whether a named site fires — **never** `thread_rng` / wall clock.

use std::cell::RefCell;
use std::collections::HashMap;

/// Named site id (matches seam inventory / engine annotations).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BuggifySite(pub &'static str);

/// Inventory-aligned hot path labels.
pub mod sites {
    use super::BuggifySite;
    /// After WAL append, before return from put.
    pub const AFTER_WAL_APPEND: BuggifySite = BuggifySite("engine.after_wal_append");
    /// Before MANIFEST rename on flush.
    pub const BEFORE_MANIFEST_RENAME: BuggifySite = BuggifySite("engine.before_manifest_rename");
    /// Before apply of a committed raft entry (store).
    pub const BEFORE_RAFT_APPLY: BuggifySite = BuggifySite("engine.before_raft_apply");
    /// After memtable insert, before WAL sync complete.
    pub const AFTER_MEM_INSERT: BuggifySite = BuggifySite("engine.after_mem_insert");
    /// Before SST temp rename on flush.
    pub const BEFORE_SST_RENAME: BuggifySite = BuggifySite("engine.before_sst_rename");
    /// Before compact merge write.
    pub const BEFORE_COMPACT_WRITE: BuggifySite = BuggifySite("engine.before_compact_write");
    /// On open after lock acquired.
    pub const AFTER_OPEN_LOCK: BuggifySite = BuggifySite("engine.after_open_lock");
    /// Before vlog append (large value).
    pub const BEFORE_VLOG_APPEND: BuggifySite = BuggifySite("engine.before_vlog_append");
}

/// All named sites (for trials / coverage).
pub const ALL_SITES: &[BuggifySite] = &[
    sites::AFTER_WAL_APPEND,
    sites::BEFORE_MANIFEST_RENAME,
    sites::BEFORE_RAFT_APPLY,
    sites::AFTER_MEM_INSERT,
    sites::BEFORE_SST_RENAME,
    sites::BEFORE_COMPACT_WRITE,
    sites::AFTER_OPEN_LOCK,
    sites::BEFORE_VLOG_APPEND,
];

/// Process-local arm table installed by DST (feature `buggify` only).
#[derive(Debug, Clone)]
pub struct BuggifyTable {
    /// site name → fire probability in ppm (`0..=1_000_000`).
    ppm: HashMap<&'static str, u32>,
    /// Deterministic counter stream.
    state: u64,
    /// Fire counts per site (observability).
    fires: HashMap<&'static str, u64>,
}

impl BuggifyTable {
    /// Build from seed: each site gets a stable ppm in 1%..=50%.
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        let mut state = crate::rng::mix_seed(seed);
        let mut ppm = HashMap::new();
        for s in ALL_SITES {
            state = xorshift(state);
            let p = 10_000 + (state % 490_000) as u32; // 1%..50%
            ppm.insert(s.0, p);
        }
        Self {
            ppm,
            state,
            fires: HashMap::new(),
        }
    }

    /// Whether `site` fires on this call (advances stream).
    pub fn should_fire(&mut self, site: BuggifySite) -> bool {
        let p = *self.ppm.get(site.0).unwrap_or(&0);
        self.state = xorshift(self.state);
        let roll = (self.state % 1_000_000) as u32;
        let fire = roll < p;
        if fire {
            *self.fires.entry(site.0).or_insert(0) += 1;
        }
        fire
    }

    /// Fire count for site.
    #[must_use]
    pub fn fire_count(&self, site: BuggifySite) -> u64 {
        self.fires.get(site.0).copied().unwrap_or(0)
    }
}

fn xorshift(mut s: u64) -> u64 {
    s ^= s >> 12;
    s ^= s << 25;
    s ^= s >> 27;
    s.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

thread_local! {
    static TABLE: RefCell<Option<BuggifyTable>> = const { RefCell::new(None) };
}

/// Install process-local table (DST only). No-op without feature.
pub fn install_table(_table: BuggifyTable) {
    #[cfg(feature = "buggify")]
    {
        TABLE.with(|t| *t.borrow_mut() = Some(_table));
    }
}

/// Clear table.
pub fn clear_table() {
    #[cfg(feature = "buggify")]
    {
        TABLE.with(|t| *t.borrow_mut() = None);
    }
}

/// Call at annotated sites.
///
/// Returns `true` if the site fired (injection point). Always `false` without
/// `feature = "buggify"` or without an installed table.
#[inline]
#[must_use]
pub fn maybe_arm(site: BuggifySite) -> bool {
    #[cfg(feature = "buggify")]
    {
        return TABLE.with(|t| {
            let mut g = t.borrow_mut();
            match g.as_mut() {
                Some(table) => table.should_fire(site),
                None => false,
            }
        });
    }
    #[cfg(not(feature = "buggify"))]
    {
        let _ = site;
        false
    }
}

/// Seed-gated install convenience.
pub fn install_from_seed(seed: u64) {
    install_table(BuggifyTable::from_seed(seed));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hooks_are_callable_no_op_without_table() {
        clear_table();
        for s in ALL_SITES {
            assert!(!maybe_arm(*s) || cfg!(feature = "buggify"));
        }
        // Without feature, always false; with feature but no table, false.
        assert!(!maybe_arm(sites::AFTER_WAL_APPEND) || cfg!(feature = "buggify"));
        if !cfg!(feature = "buggify") {
            assert!(!maybe_arm(sites::AFTER_WAL_APPEND));
        }
    }

    #[test]
    fn all_sites_named_at_least_five() {
        assert!(ALL_SITES.len() >= 5);
        let mut names = std::collections::HashSet::new();
        for s in ALL_SITES {
            assert!(names.insert(s.0), "duplicate {}", s.0);
        }
    }

    #[test]
    fn table_from_seed_replayable() {
        let mut a = BuggifyTable::from_seed(0xC0FFEE);
        let mut b = BuggifyTable::from_seed(0xC0FFEE);
        for _ in 0..32 {
            for s in ALL_SITES {
                assert_eq!(a.should_fire(*s), b.should_fire(*s));
            }
        }
    }
}

#[cfg(all(test, feature = "buggify"))]
mod feature_tests {
    use super::*;

    #[test]
    fn active_hooks_fire_deterministically_five_sites() {
        clear_table();
        install_from_seed(99);
        let mut fired_sites = 0u32;
        for s in ALL_SITES {
            let mut any = false;
            for _ in 0..500 {
                if maybe_arm(*s) {
                    any = true;
                    break;
                }
            }
            if any {
                fired_sites += 1;
            }
        }
        assert!(
            fired_sites >= 5,
            "expected ≥5 sites to fire under seed table, got {fired_sites}"
        );
        // Replay same seed → same fire counts after same call sequence
        clear_table();
        install_from_seed(99);
        let mut counts = Vec::new();
        for s in ALL_SITES {
            let mut c = 0u64;
            for _ in 0..100 {
                if maybe_arm(*s) {
                    c += 1;
                }
            }
            counts.push(c);
        }
        clear_table();
        install_from_seed(99);
        for (i, s) in ALL_SITES.iter().enumerate() {
            let mut c = 0u64;
            for _ in 0..100 {
                if maybe_arm(*s) {
                    c += 1;
                }
            }
            assert_eq!(c, counts[i], "site {} not replayable", s.0);
        }
        clear_table();
    }
}
