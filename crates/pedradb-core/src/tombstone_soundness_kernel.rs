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
use std::fmt;

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
    /// Key provided is empty.
    EmptyKey,
    /// Tombstone sequence number is zero.
    ZeroTombstoneSequence,
    /// Compaction level exceeds maximum level.
    InvalidLevelRange {
        /// Current level undergoing compaction.
        current_level: usize,
        /// Maximum level allowed in LSM hierarchy.
        max_level: usize,
    },
}

impl fmt::Display for TombstoneSoundnessViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SnapshotViewCorrupted {
                key,
                tombstone_seq,
                snapshot_seq,
            } => write!(
                f,
                "snapshot view corrupted: key {:?} with tombstone seq {tombstone_seq} purged while snapshot {snapshot_seq} active",
                key
            ),
            Self::OlderVersionUnmasked {
                key,
                compaction_level,
                shadow_level,
            } => write!(
                f,
                "older version unmasked: key {:?} purged at level {compaction_level} but shadowed at level {shadow_level}",
                key
            ),
            Self::EmptyKey => write!(f, "tombstone key cannot be empty"),
            Self::ZeroTombstoneSequence => write!(f, "tombstone sequence number cannot be zero"),
            Self::InvalidLevelRange {
                current_level,
                max_level,
            } => write!(
                f,
                "invalid level range: current level {current_level} exceeds max level {max_level}"
            ),
        }
    }
}

impl std::error::Error for TombstoneSoundnessViolation {}

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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MockLevelCatalog {
    /// Level index -> Set of keys stored at that level.
    pub levels: BTreeMap<usize, BTreeSet<Vec<u8>>>,
}

impl MockLevelCatalog {
    /// Tries to insert a key into a level, validating non-emptiness.
    pub fn try_insert(
        &mut self,
        level: usize,
        key: Vec<u8>,
    ) -> Result<(), TombstoneSoundnessViolation> {
        if key.is_empty() {
            return Err(TombstoneSoundnessViolation::EmptyKey);
        }
        self.levels.entry(level).or_default().insert(key);
        Ok(())
    }
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

    /// Evaluates tombstone purge after validating input invariants.
    pub fn try_evaluate_purge(
        key: &[u8],
        tombstone_seq: u64,
        current_level: usize,
        max_level: usize,
        active_snapshots: &[u64],
        presence_oracle: &impl LevelKeyPresenceOracle,
    ) -> Result<TombstonePurgeDecision, TombstoneSoundnessViolation> {
        if key.is_empty() {
            return Err(TombstoneSoundnessViolation::EmptyKey);
        }
        if tombstone_seq == 0 {
            return Err(TombstoneSoundnessViolation::ZeroTombstoneSequence);
        }
        if current_level > max_level {
            return Err(TombstoneSoundnessViolation::InvalidLevelRange {
                current_level,
                max_level,
            });
        }
        Ok(Self::evaluate_purge(
            key,
            tombstone_seq,
            current_level,
            max_level,
            active_snapshots,
            presence_oracle,
        ))
    }

    /// Verifies that a proposed compaction output that purged a tombstone is sound.
    ///
    /// # Errors
    /// Returns `TombstoneSoundnessViolation` if purging would violate snapshot isolation,
    /// unmask an older record, or violate input invariants.
    pub fn verify_compaction_purge(
        key: &[u8],
        tombstone_seq: u64,
        current_level: usize,
        max_level: usize,
        active_snapshots: &[u64],
        presence_oracle: &impl LevelKeyPresenceOracle,
        purged: bool,
    ) -> Result<(), TombstoneSoundnessViolation> {
        if key.is_empty() {
            return Err(TombstoneSoundnessViolation::EmptyKey);
        }
        if tombstone_seq == 0 {
            return Err(TombstoneSoundnessViolation::ZeroTombstoneSequence);
        }
        if current_level > max_level {
            return Err(TombstoneSoundnessViolation::InvalidLevelRange {
                current_level,
                max_level,
            });
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tombstone_soundness_structural_invariants_red_to_green() {
        // 1. Error Display & std::error::Error conformance
        let errs: Vec<TombstoneSoundnessViolation> = vec![
            TombstoneSoundnessViolation::SnapshotViewCorrupted {
                key: b"k1".to_vec(),
                tombstone_seq: 10,
                snapshot_seq: 20,
            },
            TombstoneSoundnessViolation::OlderVersionUnmasked {
                key: b"k2".to_vec(),
                compaction_level: 2,
                shadow_level: 4,
            },
            TombstoneSoundnessViolation::EmptyKey,
            TombstoneSoundnessViolation::ZeroTombstoneSequence,
            TombstoneSoundnessViolation::InvalidLevelRange {
                current_level: 5,
                max_level: 4,
            },
        ];
        for err in &errs {
            let msg = format!("{err}");
            assert!(!msg.is_empty());
            let dyn_err: &dyn std::error::Error = err;
            assert_eq!(dyn_err.to_string(), msg);
        }

        // 2. MockLevelCatalog validation
        let mut catalog = MockLevelCatalog::default();
        assert_eq!(
            catalog.try_insert(1, vec![]),
            Err(TombstoneSoundnessViolation::EmptyKey)
        );
        assert!(catalog.try_insert(2, b"active_key".to_vec()).is_ok());
        assert!(catalog.level_may_contain_key(2, b"active_key"));
        assert!(!catalog.level_may_contain_key(1, b"active_key"));

        // 3. Oracle invariant validation
        assert_eq!(
            TombstonePurgeOracle::try_evaluate_purge(b"", 10, 1, 4, &[], &catalog),
            Err(TombstoneSoundnessViolation::EmptyKey)
        );
        assert_eq!(
            TombstonePurgeOracle::try_evaluate_purge(b"k", 0, 1, 4, &[], &catalog),
            Err(TombstoneSoundnessViolation::ZeroTombstoneSequence)
        );
        assert_eq!(
            TombstonePurgeOracle::try_evaluate_purge(b"k", 10, 5, 4, &[], &catalog),
            Err(TombstoneSoundnessViolation::InvalidLevelRange {
                current_level: 5,
                max_level: 4
            })
        );

        // 4. verify_compaction_purge validation
        assert_eq!(
            TombstonePurgeOracle::verify_compaction_purge(b"", 10, 1, 4, &[], &catalog, true),
            Err(TombstoneSoundnessViolation::EmptyKey)
        );
        assert_eq!(
            TombstonePurgeOracle::verify_compaction_purge(b"k", 0, 1, 4, &[], &catalog, true),
            Err(TombstoneSoundnessViolation::ZeroTombstoneSequence)
        );
        assert_eq!(
            TombstonePurgeOracle::verify_compaction_purge(b"k", 10, 5, 4, &[], &catalog, true),
            Err(TombstoneSoundnessViolation::InvalidLevelRange {
                current_level: 5,
                max_level: 4
            })
        );
    }
}

