//! RFC-0288: SkipList Weak Memory Barrier and Path Bisimulation Kernel.
//!
//! Enforces monotonic search invariant along the descent path across SkipList towers
//! under weak memory models (ARM64/Graviton), preventing ghost skips over committed keys.

use std::fmt;

/// Maximum height supported for SkipList towers.
pub const MAX_SKIPLIST_HEIGHT: usize = 32;

/// A discrete navigation step in the SkipList search traversal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchStep {
    /// Level at which this step occurred (0 = base linked list, H-1 = apex).
    pub level: usize,
    /// Key observed at this node (None if head sentinel).
    pub node_key: Option<Vec<u8>>,
    /// Type of transition.
    pub transition: StepTransition,
}

/// Transition type between steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepTransition {
    /// Advance horizontally on the same level: current -> next.
    HorizontalAdvance,
    /// Descend vertically to lower level: level -> level - 1.
    VerticalDescent,
}

/// Violations detected in a search path under weak memory analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonotonicityViolation {
    /// Horizontal jump moved backwards or stalled on equal key.
    HorizontalOrderInversion {
        /// Level where violation occurred.
        level: usize,
        /// Predecessor key.
        from_key: Option<Vec<u8>>,
        /// Successor key.
        to_key: Vec<u8>,
    },
    /// Horizontal advance overshot the search target key.
    HorizontalOvershoot {
        /// Target key sought.
        target_key: Vec<u8>,
        /// Overshot node key.
        observed_key: Vec<u8>,
    },
    /// Vertical descent changed node identity (node key must be invariant when descending).
    VerticalKeyShift {
        /// Higher level.
        from_level: usize,
        /// Lower level.
        to_level: usize,
        /// Higher level key.
        from_key: Option<Vec<u8>>,
        /// Lower level key.
        to_key: Option<Vec<u8>>,
    },
    /// Illegal level jump (must descend by exactly 1 level).
    IllegalLevelJump {
        /// Starting level.
        from_level: usize,
        /// Ending level.
        to_level: usize,
    },
}

impl fmt::Display for MonotonicityViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HorizontalOrderInversion { level, from_key, to_key } => {
                write!(f, "horizontal order inversion at level {level}: from {from_key:?} to {to_key:?}")
            }
            Self::HorizontalOvershoot { target_key, observed_key } => {
                write!(f, "horizontal overshoot: target {target_key:?}, observed {observed_key:?}")
            }
            Self::VerticalKeyShift { from_level, to_level, from_key, to_key } => {
                write!(f, "vertical key shift {from_level}->{to_level}: from {from_key:?} to {to_key:?}")
            }
            Self::IllegalLevelJump { from_level, to_level } => {
                write!(f, "illegal level jump: {from_level} to {to_level}")
            }
        }
    }
}

impl std::error::Error for MonotonicityViolation {}

/// Verifier of SkipList search path integrity and weak-memory fence validity.
pub struct SkipListBarrierValidator;

impl SkipListBarrierValidator {
    /// Validates that a search path satisfies strict bisimulation against linear order.
    pub fn verify_path(
        target_key: &[u8],
        path: &[SearchStep],
    ) -> Result<(), MonotonicityViolation> {
        if path.is_empty() {
            return Ok(());
        }

        for window in path.windows(2) {
            let prev = &window[0];
            let curr = &window[1];

            match curr.transition {
                StepTransition::HorizontalAdvance => {
                    if prev.level != curr.level {
                        return Err(MonotonicityViolation::IllegalLevelJump {
                            from_level: prev.level,
                            to_level: curr.level,
                        });
                    }

                    // curr.node_key must be strictly greater than prev.node_key
                    let curr_k = curr.node_key.as_ref().expect("horizontal step target must have key");
                    if let Some(ref prev_k) = prev.node_key {
                        if prev_k >= curr_k {
                            return Err(MonotonicityViolation::HorizontalOrderInversion {
                                level: prev.level,
                                from_key: Some(prev_k.clone()),
                                to_key: curr_k.clone(),
                            });
                        }
                    }

                    // curr_k must not overshoot target if advance is supposed to stop at <= target
                    if curr_k.as_slice() > target_key {
                        return Err(MonotonicityViolation::HorizontalOvershoot {
                            target_key: target_key.to_vec(),
                            observed_key: curr_k.clone(),
                        });
                    }
                }
                StepTransition::VerticalDescent => {
                    if prev.level == 0 || curr.level != prev.level - 1 {
                        return Err(MonotonicityViolation::IllegalLevelJump {
                            from_level: prev.level,
                            to_level: curr.level,
                        });
                    }

                    // When descending, the cursor remains at the identical node
                    if prev.node_key != curr.node_key {
                        return Err(MonotonicityViolation::VerticalKeyShift {
                            from_level: prev.level,
                            to_level: curr.level,
                            from_key: prev.node_key.clone(),
                            to_key: curr.node_key.clone(),
                        });
                    }
                }
            }
        }

        Ok(())
    }
}
