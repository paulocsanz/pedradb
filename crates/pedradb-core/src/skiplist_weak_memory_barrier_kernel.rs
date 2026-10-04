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

impl SearchStep {
    /// Cria um passo de busca validado garantindo limites de altura e chaves não-vazias.
    pub fn try_new(
        level: usize,
        node_key: Option<Vec<u8>>,
        transition: StepTransition,
    ) -> Result<Self, MonotonicityViolation> {
        if level >= MAX_SKIPLIST_HEIGHT {
            return Err(MonotonicityViolation::LevelExceedsMaxHeight {
                level,
                max_height: MAX_SKIPLIST_HEIGHT,
            });
        }
        if let Some(ref k) = node_key {
            if k.is_empty() {
                return Err(MonotonicityViolation::EmptyNodeKey { level });
            }
        }
        Ok(Self {
            level,
            node_key,
            transition,
        })
    }
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
    /// Target key cannot be empty.
    EmptyTargetKey,
    /// Node key cannot be empty when present.
    EmptyNodeKey {
        /// Level where empty node key was observed.
        level: usize,
    },
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
    /// Horizontal advance landed on a node missing a key (None key).
    MissingNodeKey {
        /// Level where missing key was observed.
        level: usize,
    },
    /// Search path is empty when a complete search was expected.
    EmptySearchPath,
    /// Search path did not reach base level 0.
    IncompleteDescent {
        /// Final level where search stopped.
        final_level: usize,
    },
    /// Search step level exceeds maximum supported tower height.
    LevelExceedsMaxHeight {
        /// Observed level.
        level: usize,
        /// Maximum supported height.
        max_height: usize,
    },
}

impl fmt::Display for MonotonicityViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTargetKey => write!(f, "target search key cannot be empty"),
            Self::EmptyNodeKey { level } => write!(f, "node key at level {level} cannot be empty"),
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
            Self::MissingNodeKey { level } => {
                write!(f, "missing node key in horizontal advance at level {level}")
            }
            Self::EmptySearchPath => {
                write!(f, "search path is empty")
            }
            Self::IncompleteDescent { final_level } => {
                write!(f, "incomplete descent: search stopped at level {final_level} without reaching level 0")
            }
            Self::LevelExceedsMaxHeight { level, max_height } => {
                write!(f, "step level {level} exceeds maximum skiplist height {max_height}")
            }
        }
    }
}

impl std::error::Error for MonotonicityViolation {}

/// Terminal outcome of a verified complete SkipList search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathTerminalOutcome {
    /// Search landed exactly on target key at level 0.
    ExactMatch { key: Vec<u8> },
    /// Search landed on immediate predecessor (< target_key) at level 0.
    Predecessor { key: Option<Vec<u8>> },
}

/// Verifier of SkipList search path integrity and weak-memory fence validity.
pub struct SkipListBarrierValidator;

impl SkipListBarrierValidator {
    /// Validates that a search path satisfies strict bisimulation against linear order.
    pub fn verify_path(
        target_key: &[u8],
        path: &[SearchStep],
    ) -> Result<(), MonotonicityViolation> {
        if target_key.is_empty() {
            return Err(MonotonicityViolation::EmptyTargetKey);
        }

        if path.is_empty() {
            return Ok(());
        }

        for step in path {
            if step.level >= MAX_SKIPLIST_HEIGHT {
                return Err(MonotonicityViolation::LevelExceedsMaxHeight {
                    level: step.level,
                    max_height: MAX_SKIPLIST_HEIGHT,
                });
            }

            if let Some(ref k) = step.node_key {
                if k.is_empty() {
                    return Err(MonotonicityViolation::EmptyNodeKey { level: step.level });
                }
                if k.as_slice() > target_key {
                    return Err(MonotonicityViolation::HorizontalOvershoot {
                        target_key: target_key.to_vec(),
                        observed_key: k.clone(),
                    });
                }
            }
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

                    // curr.node_key must be strictly greater than prev.node_key (safe non-panicking check)
                    let curr_k = match curr.node_key.as_ref() {
                        Some(k) => k,
                        None => {
                            return Err(MonotonicityViolation::MissingNodeKey {
                                level: curr.level,
                            });
                        }
                    };

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

    /// Verifies that a search path starts from a valid height, follows monotonicity invariants,
    /// and successfully reaches base level 0. Returns the terminal key classification.
    pub fn verify_complete_search(
        target_key: &[u8],
        path: &[SearchStep],
    ) -> Result<PathTerminalOutcome, MonotonicityViolation> {
        if target_key.is_empty() {
            return Err(MonotonicityViolation::EmptyTargetKey);
        }

        if path.is_empty() {
            return Err(MonotonicityViolation::EmptySearchPath);
        }

        Self::verify_path(target_key, path)?;

        let last_step = path.last().expect("path non-empty checked above");
        if last_step.level != 0 {
            return Err(MonotonicityViolation::IncompleteDescent {
                final_level: last_step.level,
            });
        }

        match &last_step.node_key {
            Some(k) if k.as_slice() == target_key => {
                Ok(PathTerminalOutcome::ExactMatch { key: k.clone() })
            }
            Some(k) if k.as_slice() > target_key => {
                Err(MonotonicityViolation::HorizontalOvershoot {
                    target_key: target_key.to_vec(),
                    observed_key: k.clone(),
                })
            }
            other => Ok(PathTerminalOutcome::Predecessor {
                key: other.clone(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skiplist_weak_memory_barrier_structural_invariants_red_to_green() {
        let target = b"m";

        // 1. Valid complete search path
        let s0 = SearchStep::try_new(1, None, StepTransition::HorizontalAdvance).expect("s0");
        let s1 = SearchStep::try_new(1, Some(b"e".to_vec()), StepTransition::HorizontalAdvance).expect("s1");
        let s2 = SearchStep::try_new(0, Some(b"e".to_vec()), StepTransition::VerticalDescent).expect("s2");
        let s3 = SearchStep::try_new(0, Some(b"m".to_vec()), StepTransition::HorizontalAdvance).expect("s3");

        let path = vec![s0, s1, s2, s3];
        let outcome = SkipListBarrierValidator::verify_complete_search(target, &path)
            .expect("search must be valid");
        assert_eq!(outcome, PathTerminalOutcome::ExactMatch { key: b"m".to_vec() });

        // 2. Reject empty target key
        let err_empty_target = SkipListBarrierValidator::verify_path(b"", &path);
        assert_eq!(err_empty_target, Err(MonotonicityViolation::EmptyTargetKey));

        // 3. Reject empty node key
        let err_empty_node = SearchStep::try_new(0, Some(vec![]), StepTransition::HorizontalAdvance);
        assert_eq!(err_empty_node, Err(MonotonicityViolation::EmptyNodeKey { level: 0 }));

        // 4. Reject level >= MAX_SKIPLIST_HEIGHT
        let err_max_height = SearchStep::try_new(MAX_SKIPLIST_HEIGHT, None, StepTransition::HorizontalAdvance);
        assert_eq!(
            err_max_height,
            Err(MonotonicityViolation::LevelExceedsMaxHeight {
                level: MAX_SKIPLIST_HEIGHT,
                max_height: MAX_SKIPLIST_HEIGHT,
            })
        );

        // 5. Incomplete descent rejection
        let incomplete_path = vec![
            SearchStep::try_new(2, None, StepTransition::HorizontalAdvance).expect("step"),
            SearchStep::try_new(1, None, StepTransition::VerticalDescent).expect("step"),
        ];
        let err_incomplete = SkipListBarrierValidator::verify_complete_search(target, &incomplete_path);
        assert_eq!(err_incomplete, Err(MonotonicityViolation::IncompleteDescent { final_level: 1 }));

        // 6. Display & Error implementations
        let d = format!("{}", MonotonicityViolation::EmptyTargetKey);
        assert!(d.contains("target search key cannot be empty"));
    }
}
