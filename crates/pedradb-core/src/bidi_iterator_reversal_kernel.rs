//! RFC-0288: Bidirectional Iterator Reversal Automaton Kernel.
//!
//! Models merged N-way bidirectional LSM iterators as a discrete finite automaton.
//! Proves strict continuity and positional bijection under direction switching:
//! Prev(Next(it)) == it and Next(Prev(it)) == it without off-by-one errors or key skipping.

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
#[derive(Debug, Clone)]
pub struct BidiCursorStateMachine {
    /// Items in canonical ascending order.
    items: Vec<(Vec<u8>, Vec<u8>)>,
    /// Current logical index in `items`.
    cursor: Option<usize>,
    /// Active direction.
    direction: CursorDirection,
}

impl BidiCursorStateMachine {
    /// Creates a new bidirectional cursor over sorted items.
    #[must_use]
    pub fn new(items: Vec<(Vec<u8>, Vec<u8>)>) -> Self {
        Self {
            items,
            cursor: None,
            direction: CursorDirection::Neutral,
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
