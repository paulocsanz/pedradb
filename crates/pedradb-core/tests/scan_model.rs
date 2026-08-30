//! Stateright model over the **real** [`pedradb_core::sst::scan_reads_file`]
//! (F167).
//!
//! Faithfulness: the predicate is the production fast-reject used by
//! `SstTable::entries_in_user_range`; a `false` result means the streaming
//! scan never sees the file's point entries **nor its range tombstones**
//! (both are dropped together when the file is skipped).
//!
//! Domain: `u8` key bytes (model twin of `[u8]` ordering — same total order,
//! finite so it can be enumerated). A file holds point keys within
//! `[smallest, largest]` plus an optional range tombstone `[tomb_start, tomb_end)`
//! whose start key is itself a point key in the file (writer invariant:
//! `tomb_end > tomb_start`, `tomb_start ∈ [smallest, largest]`).
//!
//! - **Inv-no-point-resurrect:** skip ⇒ no possible point key of the file
//!   falls inside the scan window.
//! - **Inv-no-cover-resurrect:** skip ⇒ no key inside the scan window is
//!   covered by the file's tombstone (skipping must not un-delete).
//! - **Inv-non-vacuous-skip:** some world skips (the fast path exists).
//! - AS-IS (bounds only) must skip a spanning tombstone (teeth).

use std::ops::Bound;

use pedradb_core::sst::{key_in_window, scan_reads_file, scan_reads_file_as_is};
use stateright::{Checker, Model, Property};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct St {
    decided: bool,
    point_resurrect: bool,
    cover_resurrect: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Scan,
}

#[derive(Clone)]
struct ScanModel {
    smallest: u8,
    largest: u8,
    tomb_start: u8,
    tomb_end: Option<u8>,
    start: BoundKey,
    end: BoundKey,
    fixed: bool,
}

/// Model stand-in for `Bound<&[u8]>` over a one-byte key domain.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum BoundKey {
    Unbounded,
    Included(u8),
    Excluded(u8),
}

impl BoundKey {
    /// Borrowing `Bound<&[u8]>` over the caller-provided one-byte buffer.
    fn bound<'a>(&'a self, buf: &'a mut [u8; 1]) -> Bound<&'a [u8]> {
        match self {
            BoundKey::Unbounded => Bound::Unbounded,
            BoundKey::Included(k) => {
                buf[0] = *k;
                Bound::Included(buf.as_slice())
            }
            BoundKey::Excluded(k) => {
                buf[0] = *k;
                Bound::Excluded(buf.as_slice())
            }
        }
    }
}

impl ScanModel {
    fn reads(&self) -> bool {
        let start_bytes = [self.tomb_start];
        let end_bytes;
        let tombs: Vec<(&[u8], &[u8])> = match self.tomb_end {
            Some(e) => {
                end_bytes = [e];
                vec![(start_bytes.as_slice(), end_bytes.as_slice())]
            }
            None => Vec::new(),
        };
        let mut sb = [0u8; 1];
        let mut eb = [0u8; 1];
        if self.fixed {
            scan_reads_file(
                Some(&[self.smallest]),
                Some(&[self.largest]),
                &tombs,
                self.start.bound(&mut sb),
                self.end.bound(&mut eb),
            )
        } else {
            scan_reads_file_as_is(
                Some(&[self.smallest]),
                Some(&[self.largest]),
                &tombs,
                self.start.bound(&mut sb),
                self.end.bound(&mut eb),
            )
        }
    }

    /// Any point key `[smallest, largest]` inside the window.
    fn point_in_window(&self) -> bool {
        let mut sb = [0u8; 1];
        let mut eb = [0u8; 1];
        (self.smallest..=self.largest)
            .any(|k| key_in_window(&[k], self.start.bound(&mut sb), self.end.bound(&mut eb)))
    }

    /// Any window key covered by the tombstone `[tomb_start, tomb_end)`.
    fn covers_window_key(&self) -> bool {
        let Some(tomb_end) = self.tomb_end else {
            return false;
        };
        let mut sb = [0u8; 1];
        let mut eb = [0u8; 1];
        (0..=u8::MAX)
            .filter(|w| key_in_window(&[*w], self.start.bound(&mut sb), self.end.bound(&mut eb)))
            .any(|w| w >= self.tomb_start && w < tomb_end)
    }
}

impl Model for ScanModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            decided: false,
            point_resurrect: false,
            cover_resurrect: false,
        }]
    }

    fn actions(&self, state: &St, actions: &mut Vec<Self::Action>) {
        if !state.decided {
            actions.push(Act::Scan);
        }
    }

    fn next_state(&self, state: &St, _action: Act) -> Option<St> {
        let mut next = state.clone();
        next.decided = true;
        if !self.reads() {
            if self.point_in_window() {
                next.point_resurrect = true;
            }
            if self.covers_window_key() {
                next.cover_resurrect = true;
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-no-point-resurrect", |_m, s: &St| !s.point_resurrect),
            Property::always("Inv-no-cover-resurrect", |_m, s: &St| !s.cover_resurrect),
            Property::sometimes("Inv-non-vacuous-skip", |_m, s: &St| {
                s.decided && !s.point_resurrect && !s.cover_resurrect
            }),
        ]
    }
}

fn worlds() -> Vec<ScanModel> {
    let mut out = Vec::new();
    let keys = [0u8, 1, 2, 3, 4, 5, 6, 7];
    let bounds = [
        BoundKey::Unbounded,
        BoundKey::Included(0),
        BoundKey::Included(2),
        BoundKey::Included(4),
        BoundKey::Included(6),
        BoundKey::Excluded(2),
        BoundKey::Excluded(4),
        BoundKey::Excluded(6),
    ];
    for smallest in keys {
        for &largest in keys.iter().filter(|&&k| k >= smallest) {
            // Tombstone start is a point key of the file; end strictly past it.
            for &tomb_start in keys.iter().filter(|&&k| (smallest..=largest).contains(&k)) {
                for tomb_end in [None::<u8>]
                    .into_iter()
                    .chain(keys.iter().filter(|&&k| k > tomb_start).map(|&k| Some(k)))
                {
                    for start in bounds {
                        for end in bounds {
                            for fixed in [true, false] {
                                out.push(ScanModel {
                                    smallest,
                                    largest,
                                    tomb_start,
                                    tomb_end,
                                    start,
                                    end,
                                    fixed,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

#[test]
fn fixed_guard_holds_in_all_worlds() {
    for model in worlds() {
        if !model.fixed {
            continue;
        }
        let checker = model.checker().spawn_bfs().join();
        checker.assert_properties();
    }
}

#[test]
fn some_worlds_skip_the_file() {
    // Non-vacuity: the fast path still prunes fully-disjoint files.
    let model = ScanModel {
        smallest: 0,
        largest: 1,
        tomb_start: 0,
        tomb_end: Some(2),
        start: BoundKey::Included(6),
        end: BoundKey::Included(7),
        fixed: true,
    };
    assert!(!model.reads(), "disjoint file must be skipped");
}

/// F167 teeth: AS-IS skips a file whose only entry is the tombstone, its span
/// reaching into the window (covered keys scan as live).
#[test]
fn as_is_misses_spanning_tombstone() {
    let model = ScanModel {
        smallest: 2,
        largest: 2,
        tomb_start: 2,
        tomb_end: Some(5),
        start: BoundKey::Included(4),
        end: BoundKey::Included(6),
        fixed: false,
    };
    assert!(
        !model.reads(),
        "AS-IS must skip the spanning file for the teeth to mean anything"
    );
    assert!(
        model.covers_window_key(),
        "window key 4 is covered by tombstone [2, 5)"
    );
    let checker = model.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-no-cover-resurrect").is_some(),
        "AS-IS must resurrect a covered key (F167 teeth)"
    );
}
