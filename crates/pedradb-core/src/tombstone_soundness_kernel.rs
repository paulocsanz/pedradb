//! RFC-0281 P1.1 — Tombstone Soundness & Unmasking Prevention Kernel.
//!
//! Formalizes the safety invariants governing tombstone deletion during compaction:
//!   1. Snapshot Visibility: A tombstone cannot be dropped if any active snapshot
//!      could observe the key (i.e. snapshot_seq >= tombstone_seq or snapshot requires
//!      deletion visibility against older versions).
//!   2. Unmasking Prevention: A tombstone cannot be dropped at level L if an older
//!      version of the same key exists in any deeper level L' \in [L+1, L_max].
//!
//! Dropping a tombstone when shadows exist in lower levels causes the older version
//! to be silently unmasked, corrupting data integrity.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

/// Violations resulting from unsafe tombstone elimination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TombstoneSoundnessViolation {
    /// A tombstone was purged while an active snapshot still requires it.
    SnapshotViewCorrupted {
        /// Key associated with the tombstone.
        key: Vec<u8>,
        /// Sequence number of the tombstone.
        tombstone_seq: u64,
        /// Active snapshot sequence number that is compromised.
        snapshot_seq: u64,
    },
    /// A tombstone was purged while older versions exist in deeper levels (unmasking bug).
    OlderVersionUnmasked {
        /// Key whose older version was unmasked.
        key: Vec<u8>,
        /// Level where the tombstone was prematurely dropped.
        compaction_level: usize,
        /// Deeper level where an older shadowed version still exists.
        shadow_level: usize,
    },
}

/// The decision produced by the safety oracle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TombstonePurgeDecision {
    /// Safe to permanently eliminate the tombstone.
    SafeToPurge,
    /// Must retain because an active snapshot intersects with this sequence.
    RetainForSnapshot {
        /// Oldest snapshot sequence preventing purge.
        blocking_snapshot: u64,
    },
    /// Must retain and push to deeper level because older version of key exists below.
    RetainForShadowedKey {
        /// The deepest level containing an older version.
        shadow_level: usize,
    },
}

/// Abstraction representing key presence across LSM levels.
pub trait LevelKeyPresenceOracle {
    /// Returns true if level `level` may contain the given key.
    fn level_may_contain_key(&self, level: usize, key: &[u8]) -> bool;
}

/// In-memory implementation of the presence oracle for verification and testing.
#[derive(Clone, Debug, Default)]
pub struct MockLevelCatalog {
    /// Level index -> Set of keys stored at that level.
    pub levels: BTreeMap<usize, BTreeSet<Vec<u8>>>,
}

impl LevelKeyPresenceOracle for MockLevelCatalog {
    fn level_may_contain_key(&self, level: usize, key: &[u8]) -> bool {
        self.levels
            .get(&level)
            .map_or(false, |keys| keys.contains(key))
    }
}

/// Oracle that governs the soundness of tombstone purging.
pub struct TombstonePurgeOracle;

impl TombstonePurgeOracle {
    /// Decides whether a tombstone at `current_level` can be safely discarded.
    #[must_use]
    pub fn evaluate_purge(
        key: &[u8],
        tombstone_seq: u64,
        current_level: usize,
        max_level: usize,
        active_snapshots: &[u64],
        presence_oracle: &impl LevelKeyPresenceOracle,
    ) -> TombstonePurgeDecision {
        // Condition 1: Check active snapshots.
        // If any snapshot has snapshot_seq >= tombstone_seq, it may need to see that the key is deleted
        // instead of falling through to an older version or seeing no tombstone.
        if let Some(&min_blocking) = active_snapshots
            .iter()
            .filter(|&&s| s >= tombstone_seq)
            .min()
        {
            return TombstonePurgeDecision::RetainForSnapshot {
                blocking_snapshot: min_blocking,
            };
        }

        // Condition 2: Check deeper levels for older shadowed versions of this key.
        for l in (current_level + 1)..=max_level {
            if presence_oracle.level_may_contain_key(l, key) {
                return TombstonePurgeDecision::RetainForShadowedKey { shadow_level: l };
            }
        }

        TombstonePurgeDecision::SafeToPurge
    }

    /// Verifies that a proposed compaction output that purged a tombstone is sound.
    ///
    /// # Errors
    /// Returns `TombstoneSoundnessViolation` if purging would violate snapshot isolation
    /// or unmask an older record.
    pub fn verify_compaction_purge(
        key: &[u8],
        tombstone_seq: u64,
        current_level: usize,
        max_level: usize,
        active_snapshots: &[u64],
        presence_oracle: &impl LevelKeyPresenceOracle,
        purged: bool,
    ) -> Result<(), TombstoneSoundnessViolation> {
        let decision = Self::evaluate_purge(
            key,
            tombstone_seq,
            current_level,
            max_level,
            active_snapshots,
            presence_oracle,
        );

        if purged {
            match decision {
                TombstonePurgeDecision::SafeToPurge => Ok(()),
                TombstonePurgeDecision::RetainForSnapshot { blocking_snapshot } => {
                    Err(TombstoneSoundnessViolation::SnapshotViewCorrupted {
                        key: key.to_vec(),
                        tombstone_seq,
                        snapshot_seq: blocking_snapshot,
                    })
                }
                TombstonePurgeDecision::RetainForShadowedKey { shadow_level } => {
                    Err(TombstoneSoundnessViolation::OlderVersionUnmasked {
                        key: key.to_vec(),
                        compaction_level: current_level,
                        shadow_level,
                    })
                }
            }
        } else {
            Ok(())
        }
    }
}
