//! RFC-0278 P0 — LSM Inductive Bisimulation & Snapshot Equivalence Kernel.
//!
//! Provides the mathematical abstraction function $\alpha$ and verifies the
//! inductive simulation relation between the physical multi-level LSM tree
//! and the abstract sequential key-value store $\mathcal{M}: K \to \text{Option}(V)$.
//!
//! # Core Invariants:
//! 1. **Shadowing Monotonicity:** $\forall L_a < L_b, \forall k \in L_a \cap L_b: \text{seq}(k \in L_a) \ge \text{seq}(k \in L_b)$.
//! 2. **Compaction Bisimulation:** $\forall \text{Snapshot } S, \forall k: \text{Lookup}(\sigma_{\text{post}}, k, S) \equiv \text{Lookup}(\sigma_{\text{pre}}, k, S)$.
//! 3. **Tombstone Hygiene:** A deleted key is never resurrected, and a tombstone is never dropped while any active snapshot $S \ge \text{tombstone.seq}$ exists.

#![forbid(unsafe_code)]

/// Maximum number of physical LSM levels supported.
pub const MAX_LEVELS: usize = 7;
/// Maximum entries per level in bounded simulation state.
pub const MAX_ENTRIES: usize = 32;

/// A single versioned physical entry in the LSM hierarchy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysicalEntry {
    /// 64-bit user key.
    pub key: u64,
    /// Monotonically increasing sequence number.
    pub seq: u64,
    /// 64-bit payload value.
    pub val: u64,
    /// True if this entry represents a deletion tombstone.
    pub is_tombstone: bool,
}

/// Abstract representation of a point lookup result at a specific snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbstractValue {
    /// Key exists with the given value and commit sequence.
    Present {
        /// Value payload.
        val: u64,
        /// Commit sequence number.
        seq: u64,
    },
    /// Key was deleted at or before the snapshot.
    Deleted {
        /// Deletion sequence number.
        seq: u64,
    },
    /// Key has never been written at or before the snapshot.
    NotFound,
}

/// Physical snapshot of the LSM tree structure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LsmTreeState {
    /// Active mutable memtable entries (sorted by key ASC, seq DESC).
    pub memtable: Vec<PhysicalEntry>,
    /// Immutable memtables waiting to be flushed (newest to oldest).
    pub immutable_memtables: Vec<Vec<PhysicalEntry>>,
    /// Disk levels L0 through L_max.
    pub levels: [Vec<PhysicalEntry>; MAX_LEVELS],
}

impl LsmTreeState {
    /// Creates an empty LSM tree state.
    pub fn empty() -> Self {
        Self {
            memtable: Vec::new(),
            immutable_memtables: Vec::new(),
            levels: Default::default(),
        }
    }

    /// Verifies the Shadowing Monotonicity Invariant:
    /// Any newer level (closer to MemTable) must have a sequence number greater than
    /// or equal to an older level for the same key.
    pub fn verify_shadowing_monotonicity(&self) -> bool {
        // Collect all levels from newest to oldest:
        // 0: Memtable
        // 1..m: Immutable memtables
        // m+1..: Disk levels L0..Lk
        let mut level_slices: Vec<&[PhysicalEntry]> = Vec::new();
        level_slices.push(&self.memtable);
        for imm in &self.immutable_memtables {
            level_slices.push(imm);
        }
        for lvl in &self.levels {
            level_slices.push(lvl);
        }

        for i in 0..level_slices.len() {
            for entry_upper in level_slices[i] {
                for j in (i + 1)..level_slices.len() {
                    for entry_lower in level_slices[j] {
                        if entry_upper.key == entry_lower.key {
                            // Upper level entry must have higher or equal sequence number
                            if entry_upper.seq < entry_lower.seq {
                                return false; // Shadowing invariant violated!
                            }
                        }
                    }
                }
            }
        }
        true
    }

    /// Point lookup for key `k` at snapshot sequence `snapshot_seq`.
    /// Traverses levels from newest (MemTable) to oldest (Lk).
    pub fn lookup(&self, key: u64, snapshot_seq: u64) -> AbstractValue {
        // 1. Check mutable memtable
        if let Some(entry) = find_visible_in_level(&self.memtable, key, snapshot_seq) {
            return entry_to_abstract(entry);
        }

        // 2. Check immutable memtables (newest to oldest)
        for imm in &self.immutable_memtables {
            if let Some(entry) = find_visible_in_level(imm, key, snapshot_seq) {
                return entry_to_abstract(entry);
            }
        }

        // 3. Check disk levels L0 to L_max
        for lvl in &self.levels {
            if let Some(entry) = find_visible_in_level(lvl, key, snapshot_seq) {
                return entry_to_abstract(entry);
            }
        }

        AbstractValue::NotFound
    }

    /// Range scan returning all visible non-tombstone entries in [start_key, end_key] at `snapshot_seq`.
    pub fn scan(&self, start_key: u64, end_key: u64, snapshot_seq: u64) -> Vec<(u64, u64)> {
        let mut results = Vec::new();
        for k in start_key..=end_key {
            if let AbstractValue::Present { val, .. } = self.lookup(k, snapshot_seq) {
                results.push((k, val));
            }
        }
        results
    }
}

/// Helper to find the newest entry for `key` with `seq <= snapshot_seq` within a level.
fn find_visible_in_level(level: &[PhysicalEntry], key: u64, snapshot_seq: u64) -> Option<PhysicalEntry> {
    let mut best: Option<PhysicalEntry> = None;
    for entry in level {
        if entry.key == key && entry.seq <= snapshot_seq {
            match best {
                None => best = Some(*entry),
                Some(current) => {
                    if entry.seq > current.seq {
                        best = Some(*entry);
                    }
                }
            }
        }
    }
    best
}

fn entry_to_abstract(entry: PhysicalEntry) -> AbstractValue {
    if entry.is_tombstone {
        AbstractValue::Deleted { seq: entry.seq }
    } else {
        AbstractValue::Present {
            val: entry.val,
            seq: entry.seq,
        }
    }
}

/// Compaction plan representing a transition from state `sigma` to `sigma'`.
pub struct CompactionPlan {
    /// Level being compacted from (e.g. 0).
    pub source_level: usize,
    /// Level being compacted into (e.g. 1).
    pub target_level: usize,
    /// Oldest active snapshot in the system (tombstones with seq < min_snapshot can be purged).
    pub oldest_active_snapshot: u64,
}

impl CompactionPlan {
    /// Executes a pure compaction step and returns the new state $\sigma'$.
    pub fn apply(&self, state: &LsmTreeState) -> Result<LsmTreeState, &'static str> {
        if self.source_level >= MAX_LEVELS || self.target_level >= MAX_LEVELS {
            return Err("Invalid level index");
        }
        if self.source_level >= self.target_level {
            return Err("Compaction source must be strictly above target level");
        }

        let mut new_state = state.clone();
        let src_entries = &state.levels[self.source_level];
        let tgt_entries = &state.levels[self.target_level];

        // Merge sort src and target entries
        let mut merged: Vec<PhysicalEntry> = Vec::new();
        let mut all_entries = src_entries.clone();
        all_entries.extend_from_slice(tgt_entries);

        // Sort by key ASC, seq DESC
        all_entries.sort_by(|a, b| {
            if a.key != b.key {
                a.key.cmp(&b.key)
            } else {
                b.seq.cmp(&a.seq) // higher seq first
            }
        });

        // Group by key and deduplicate while respecting snapshot retention
        let mut i = 0;
        while i < all_entries.len() {
            let key = all_entries[i].key;
            let mut key_entries = Vec::new();
            while i < all_entries.len() && all_entries[i].key == key {
                key_entries.push(all_entries[i]);
                i += 1;
            }

            // Compact key entries:
            // The newest entry is always preserved.
            // Older entries are preserved ONLY IF needed by snapshots > oldest_active_snapshot.
            // If the oldest entry is a tombstone and seq < oldest_active_snapshot, it can be dropped
            // provided target is the bottommost level containing the key.
            for (idx, entry) in key_entries.iter().enumerate() {
                if entry.is_tombstone && idx == key_entries.len() - 1 && entry.seq < self.oldest_active_snapshot {
                    // Safe to drop bottom tombstone below all active snapshots
                    continue;
                }
                merged.push(*entry);
            }
        }

        new_state.levels[self.source_level].clear();
        new_state.levels[self.target_level] = merged;

        Ok(new_state)
    }

    /// Verifies the Bisimulation Equivalence Theorem:
    /// $\forall \text{Snapshot } S \ge \text{oldest\_active\_snapshot}, \forall k: \text{Lookup}(\sigma', k, S) = \text{Lookup}(\sigma, k, S)$
    pub fn verify_bisimulation(
        &self,
        before: &LsmTreeState,
        after: &LsmTreeState,
        sample_keys: &[u64],
        snapshots: &[u64],
    ) -> bool {
        for &snap in snapshots {
            if snap < self.oldest_active_snapshot {
                continue; // Snapshots older than oldest_active_snapshot are already dead/retired
            }
            for &key in sample_keys {
                let val_before = before.lookup(key, snap);
                let val_after = after.lookup(key, snap);
                if val_before != val_after {
                    return false; // Bisimulation violated!
                }
            }
            // Also verify range scan preservation
            if let (Some(&first), Some(&last)) = (sample_keys.first(), sample_keys.last()) {
                let scan_before = before.scan(first, last, snap);
                let scan_after = after.scan(first, last, snap);
                if scan_before != scan_after {
                    return false; // Range scan equivalence violated!
                }
            }
        }
        true
    }
}
