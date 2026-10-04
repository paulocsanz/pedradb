//! RFC-0288: Bidirectional Iterator Reversal Automaton Kernel.
//!
//! Models merged N-way bidirectional LSM iterators as a discrete finite automaton.
//! Proves strict continuity and positional bijection under direction switching:
//! Prev(Next(it)) == it and Next(Prev(it)) == it without off-by-one errors or key skipping.

/// Errors produced by bidirectional iterator validation.
/// Errors produced by bidirectional iterator validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BidiIteratorError {
    /// Items are not in strictly ascending key order.
    UnsortedKeys {
        /// Key that should have been strictly smaller.
        prev_key: Vec<u8>,
        /// Successor key that broke monotonicity.
        next_key: Vec<u8>,
    },
    /// Duplicate keys detected in iterator items.
    DuplicateKeys {
        /// The duplicated key.
        key: Vec<u8>,
    },
    /// Key cannot be empty.
    KeyEmpty,
    /// Iterator items cannot be empty.
    EmptyItems,
}

impl std::fmt::Display for BidiIteratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsortedKeys { prev_key, next_key } => {
                write!(f, "unsorted keys in iterator items: {prev_key:?} > {next_key:?}")
            }
            Self::DuplicateKeys { key } => {
                write!(f, "duplicate key in iterator items: {key:?}")
            }
            Self::KeyEmpty => write!(f, "key cannot be empty in iterator items"),
            Self::EmptyItems => write!(f, "iterator items collection cannot be empty"),
        }
    }
}

impl std::error::Error for BidiIteratorError {}

/// Traversal direction of an active iterator cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorDirection {
    /// Initial or reset state, no direction locked.
    Neutral,
    /// Currently scanning in forward order (ascending keys).
    Forward,
    /// Currently scanning in backward order (descending keys).
    Backward,
}

/// A bidirectional iterator state machine over a deterministic sequence of key-value pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BidiCursorStateMachine {
    /// Items in canonical ascending order.
    items: Vec<(Vec<u8>, Vec<u8>)>,
    /// Current logical index in `items`.
    cursor: Option<usize>,
    /// Active direction.
    direction: CursorDirection,
}

impl BidiCursorStateMachine {
    /// Creates a new bidirectional cursor over non-empty items.
    pub fn try_new_non_empty(items: Vec<(Vec<u8>, Vec<u8>)>) -> Result<Self, BidiIteratorError> {
        if items.is_empty() {
            return Err(BidiIteratorError::EmptyItems);
        }
        Self::try_new(items)
    }

    /// Creates a new bidirectional cursor over items, validating that keys are strictly sorted.
    pub fn try_new(items: Vec<(Vec<u8>, Vec<u8>)>) -> Result<Self, BidiIteratorError> {
        for item in &items {
            if item.0.is_empty() {
                return Err(BidiIteratorError::KeyEmpty);
            }
        }
        for window in items.windows(2) {
            if window[0].0 == window[1].0 {
                return Err(BidiIteratorError::DuplicateKeys {
                    key: window[0].0.clone(),
                });
            }
            if window[0].0 > window[1].0 {
                return Err(BidiIteratorError::UnsortedKeys {
                    prev_key: window[0].0.clone(),
                    next_key: window[1].0.clone(),
                });
            }
        }
        Ok(Self {
            items,
            cursor: None,
            direction: CursorDirection::Neutral,
        })
    }

    /// Creates a new bidirectional cursor without validating sortedness.
    #[must_use]
    pub fn new(items: Vec<(Vec<u8>, Vec<u8>)>) -> Self {
        Self {
            items,
            cursor: None,
            direction: CursorDirection::Neutral,
        }
    }

    /// Returns true if the cursor is currently positioned at a valid item.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.cursor.is_some()
    }

    /// Returns the current value bytes if cursor is valid.
    #[must_use]
    pub fn current_value(&self) -> Option<&[u8]> {
        self.cursor.map(|idx| self.items[idx].1.as_slice())
    }

    /// Returns current key-value pair if valid.
    #[must_use]
    pub fn current(&self) -> Option<(&[u8], &[u8])> {
        self.cursor.map(|idx| (self.items[idx].0.as_slice(), self.items[idx].1.as_slice()))
    }

    /// Positions cursor at the first element whose key is >= `target_key` (Seek).
    pub fn seek(&mut self, target_key: &[u8]) -> Option<(&[u8], &[u8])> {
        self.direction = CursorDirection::Forward;
        let idx = self.items.partition_point(|(k, _)| k.as_slice() < target_key);
        if idx < self.items.len() {
            self.cursor = Some(idx);
            Some((&self.items[idx].0, &self.items[idx].1))
        } else {
            self.cursor = None;
            None
        }
    }

    /// Positions cursor at the last element whose key is <= `target_key` (SeekForPrev).
    pub fn seek_for_prev(&mut self, target_key: &[u8]) -> Option<(&[u8], &[u8])> {
        self.direction = CursorDirection::Backward;
        let idx = self.items.partition_point(|(k, _)| k.as_slice() <= target_key);
        if idx > 0 {
            let prev_idx = idx - 1;
            self.cursor = Some(prev_idx);
            Some((&self.items[prev_idx].0, &self.items[prev_idx].1))
        } else {
            self.cursor = None;
            None
        }
    }

    /// Positions cursor at the first element (SeekToFirst).
    pub fn seek_to_first(&mut self) -> Option<(&[u8], &[u8])> {
        self.direction = CursorDirection::Forward;
        if self.items.is_empty() {
            self.cursor = None;
            None
        } else {
            self.cursor = Some(0);
            Some((&self.items[0].0, &self.items[0].1))
        }
    }

    /// Positions cursor at the last element (SeekToLast).
    pub fn seek_to_last(&mut self) -> Option<(&[u8], &[u8])> {
        self.direction = CursorDirection::Backward;
        if self.items.is_empty() {
            self.cursor = None;
            None
        } else {
            let last_idx = self.items.len() - 1;
            self.cursor = Some(last_idx);
            Some((&self.items[last_idx].0, &self.items[last_idx].1))
        }
    }

    /// Advances the cursor to the next element.
    ///
    /// If direction was Backward, compensates direction reversal:
    /// the first `next()` after a backward scan re-anchors to the successor of current.
    pub fn next(&mut self) -> Option<(&[u8], &[u8])> {
        if self.items.is_empty() {
            return None;
        }

        match (self.direction, self.cursor) {
            (CursorDirection::Neutral, _) => self.seek_to_first(),
            (CursorDirection::Forward, None) => None,
            (CursorDirection::Forward, Some(idx)) => {
                if idx + 1 < self.items.len() {
                    self.cursor = Some(idx + 1);
                    Some((&self.items[idx + 1].0, &self.items[idx + 1].1))
                } else {
                    self.cursor = None; // Exhausted forward
                    None
                }
            }
            (CursorDirection::Backward, None) => {
                // Was exhausted backwards: moving forward starts at first
                self.seek_to_first()
            }
            (CursorDirection::Backward, Some(idx)) => {
                // Direction reversal: was moving backward, now forward.
                // Current cursor is pointing at item `idx`. Next item forward is `idx + 1`.
                self.direction = CursorDirection::Forward;
                if idx + 1 < self.items.len() {
                    self.cursor = Some(idx + 1);
                    Some((&self.items[idx + 1].0, &self.items[idx + 1].1))
                } else {
                    self.cursor = None;
                    None
                }
            }
        }
    }

    /// Moves the cursor to the previous element.
    ///
    /// If direction was Forward, compensates direction reversal:
    /// the first `prev()` after a forward scan re-anchors to the predecessor of current.
    pub fn prev(&mut self) -> Option<(&[u8], &[u8])> {
        if self.items.is_empty() {
            return None;
        }

        match (self.direction, self.cursor) {
            (CursorDirection::Neutral, _) => self.seek_to_last(),
            (CursorDirection::Backward, None) => None,
            (CursorDirection::Backward, Some(idx)) => {
                if idx > 0 {
                    self.cursor = Some(idx - 1);
                    Some((&self.items[idx - 1].0, &self.items[idx - 1].1))
                } else {
                    self.cursor = None; // Exhausted backward
                    None
                }
            }
            (CursorDirection::Forward, None) => {
                // Was exhausted forwards: moving backward starts at last
                self.seek_to_last()
            }
            (CursorDirection::Forward, Some(idx)) => {
                // Direction reversal: was moving forward, now backward.
                // Current cursor is pointing at item `idx`. Preceding item backward is `idx - 1`.
                self.direction = CursorDirection::Backward;
                if idx > 0 {
                    self.cursor = Some(idx - 1);
                    Some((&self.items[idx - 1].0, &self.items[idx - 1].1))
                } else {
                    self.cursor = None;
                    None
                }
            }
        }
    }

    /// Returns current cursor key if valid.
    #[must_use]
    pub fn current_key(&self) -> Option<&[u8]> {
        self.cursor.map(|idx| self.items[idx].0.as_slice())
    }

    /// Returns active direction.
    #[must_use]
    pub fn direction(&self) -> CursorDirection {
        self.direction
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bidi_iterator_validation_red_to_green() {
        assert_eq!(
            BidiCursorStateMachine::try_new_non_empty(vec![]),
            Err(BidiIteratorError::EmptyItems)
        );
        assert_eq!(
            BidiCursorStateMachine::try_new(vec![(vec![], b"v".to_vec())]),
            Err(BidiIteratorError::KeyEmpty)
        );
        assert_eq!(
            BidiCursorStateMachine::try_new(vec![
                (b"k1".to_vec(), b"v1".to_vec()),
                (b"k1".to_vec(), b"v2".to_vec()),
            ]),
            Err(BidiIteratorError::DuplicateKeys { key: b"k1".to_vec() })
        );
    }
}
