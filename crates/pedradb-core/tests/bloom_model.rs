//! Stateright model over the **real** [`pedradb_core::bloom_header_ok`]
//! (F166).
//!
//! Faithfulness: the predicate is production `bloom`; an accepted header is
//! what `BloomFilter::decode` turns into a filter whose `may_contain` loops
//! `k` times and indexes `bits[..nbytes]`.
//!
//! - **Inv-bounded-probes:** accept ⇒ `1 <= k <= MAX_K` (corrupt near-`u32::MAX`
//!   `k` must fail closed, not hang every point lookup).
//! - **Inv-bits-cover-nbits:** accept ⇒ `nbytes * 8 >= nbits`.
//! - **Inv-bits-fit-buffer:** accept ⇒ `nbytes <= residual`.
//! - AS-IS (no `k` bound) must accept a hostile `k` (teeth).

use pedradb_core::{bloom_header_ok, bloom_header_ok_as_is, MAX_K};
use stateright::{Checker, Model, Property};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct St {
    probed: bool,
    unbounded_k: bool,
    bits_miss_nbits: bool,
    bits_overrun: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Open,
}

#[derive(Clone)]
struct BloomModel {
    nbits: u32,
    k: u32,
    nbytes: u32,
    residual: u64,
    fixed: bool,
}

impl BloomModel {
    fn accept(&self) -> bool {
        if self.fixed {
            bloom_header_ok(self.nbits, self.k, self.nbytes, self.residual)
        } else {
            bloom_header_ok_as_is(self.nbits, self.k, self.nbytes, self.residual)
        }
    }
}

impl Model for BloomModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            probed: false,
            unbounded_k: false,
            bits_miss_nbits: false,
            bits_overrun: false,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if !state.probed {
            actions.push(Act::Open);
        }
    }

    fn next_state(&self, state: &Self::State, _action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        next.probed = true;
        if self.accept() {
            if self.k < 1 || self.k > MAX_K {
                next.unbounded_k = true;
            }
            if u64::from(self.nbytes) * 8 < u64::from(self.nbits) {
                next.bits_miss_nbits = true;
            }
            if u64::from(self.nbytes) > self.residual {
                next.bits_overrun = true;
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-bounded-probes", inv_bounded_probes),
            Property::always("Inv-bits-cover-nbits", inv_bits_cover_nbits),
            Property::always("Inv-bits-fit-buffer", inv_bits_fit_buffer),
            Property::sometimes("non-vacuity-open", non_vacuity_open),
        ]
    }
}

fn inv_bounded_probes(_: &BloomModel, s: &St) -> bool {
    !s.unbounded_k
}

fn inv_bits_cover_nbits(_: &BloomModel, s: &St) -> bool {
    !s.bits_miss_nbits
}

fn inv_bits_fit_buffer(_: &BloomModel, s: &St) -> bool {
    !s.bits_overrun
}

fn non_vacuity_open(_: &BloomModel, s: &St) -> bool {
    s.probed
}

fn worlds() -> Vec<BloomModel> {
    let mut out = Vec::new();
    for nbits in [1u32, 8, 64, 1_000] {
        for k in [0u32, 1, 7, MAX_K, MAX_K + 1, 1_000_000, u32::MAX] {
            for nbytes in [0u32, 1, 8, 125, 126] {
                for residual in [0u64, 8, 125, 1_000] {
                    out.push(BloomModel {
                        nbits,
                        k,
                        nbytes,
                        residual,
                        fixed: true,
                    });
                }
            }
        }
    }
    out
}

#[test]
fn fixed_header_holds_in_all_worlds() {
    for model in worlds() {
        let checker = model.checker().spawn_bfs().join();
        checker.assert_properties();
    }
}

#[test]
fn as_is_accepts_hostile_probe_count() {
    // F166 teeth: corrupt SST trailer with k near u32::MAX passes AS-IS.
    let model = BloomModel {
        nbits: 64,
        k: u32::MAX,
        nbytes: 8,
        residual: 8,
        fixed: false,
    };
    assert!(
        model.accept(),
        "AS-IS must accept the hostile header for the teeth to mean anything"
    );
    let checker = model.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-bounded-probes").is_some(),
        "AS-IS must accept unbounded k (F166 teeth)"
    );
}
