//! CQE ownership for the Linux io_uring path (U1 / F203 follow-up).
//!
//! Production Linux `ring::UringState` is the only caller. Bytes on disk, the
//! ring, and `submit_and_wait` are **caller + axiom**.
//!
//! Named decisions (SilentWrong when inverted):
//! - SQE `user_data` is **unique per issued op**. Constant per-opcode tags
//!   (`0x77` write / `0x5f` fsync / `0xd1` dir) let a leftover CQE from an
//!   op whose `submit_and_wait` failed be taken as the *next* op of the same
//!   kind — wrong `res`, double cursor advance, **false Ok on fsync** (G1).
//! - After submit returns **Err**, still harvest a matching CQE if it is
//!   already in the CQ (I/O completed despite EINTR).
//! - If the CQE is **not** yet visible, **wait again** (F208). Returning
//!   Err here would drop the caller's `pwrite` buffer while the kernel may
//!   still DMA into it — unique tags (F203) only stop the *next* op from
//!   adopting the leftover; they do not keep `buf` alive. A later leftover
//!   is `Discard` only for ops that already harvested or never pushed.
//! - After submit returns **Ok** and the CQ has only leftovers, wait again
//!   until our tag arrives.

#![forbid(unsafe_code)]
// Production callers are `#[cfg(target_os = "linux")]`. Host builds (macOS
// CI) still compile this module so the as-is vs unique tests run everywhere.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// First tag issued on a new ring (0 is reserved / untagged).
pub const FIRST_USER_DATA: u64 = 1;

/// Allocate the next SQE `user_data`. Never returns 0.
pub fn next_user_data(counter: &mut u64) -> u64 {
    loop {
        let tag = *counter;
        *counter = counter.wrapping_add(1);
        if tag != 0 {
            return tag;
        }
    }
}

/// AS-IS F203: one constant tag per opcode, so leftover same-opcode CQEs
/// look like the current op.
#[cfg(test)]
fn next_user_data_as_is(_counter: &mut u64, opcode_tag: u64) -> u64 {
    opcode_tag
}

/// Constant tags F203 shipped with (write / fsync / dir).
#[cfg(test)]
const TAG_WRITE_AS_IS: u64 = 0x77;
#[cfg(test)]
const TAG_FSYNC_AS_IS: u64 = 0x5f;
#[cfg(test)]
const TAG_DIR_AS_IS: u64 = 0xd1;

/// How to treat one CQE while waiting for `want`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CqeAct {
    /// This completion belongs to the in-flight op.
    Take,
    /// Leftover from an op that already returned. Drop it.
    Discard,
}

/// Match a CQE `user_data` against the tag we issued for the current op.
pub fn cqe_act(user_data: u64, want: u64) -> CqeAct {
    if user_data == want {
        CqeAct::Take
    } else {
        CqeAct::Discard
    }
}

/// After `submit_and_wait` on the SQE tagged `want`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitCompleteAct {
    /// Use the harvested CQE as the op result (even if submit returned Err).
    UseHarvested,
    /// Our CQE is not in the CQ yet — wait again. Holds whether submit
    /// returned Ok or Err: the SQE was already pushed, so the kernel may
    /// still complete it (F208).
    WaitMore,
    /// AS-IS F203/F208: submit failed and CQ empty → return Err (releases
    /// the caller's buffer). Production never takes this arm after a push;
    /// only the as-is test twins construct it (Linux non-test builds see
    /// it pattern-only in `ring`).
    #[cfg_attr(not(test), allow(dead_code))]
    ReturnSubmitErr,
}

/// Times `WaitMore` was chosen after `submit_and_wait` returned Err with
/// an empty CQ (the F208 arm). Test/Linux soak only.
#[cfg(test)]
pub static F208_WAITMORE_AFTER_SUBMIT_ERR: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Admit a harvested CQE `res` (RFC-0074). Negative is a kernel errno, not Ok.
#[must_use]
pub fn cqe_res_ok(res: i32) -> bool {
    res >= 0
}

/// AS-IS: treat any CQE as success (the 0074 hole — false Ok on fsync).
#[must_use]
pub fn cqe_res_ok_as_is(_res: i32) -> bool {
    true
}

/// RFC-0074 P2.2 / R-uring: a Verus twin of the io_uring *ring* (submit_sqe,
/// harvest, SQE layout). Always false. `cqe_res_ok` is cataloged; the ring
/// stays TCB. AS-IS would treat the res-gate twin as a ring proof.
#[must_use]
pub fn cqe_ring_model_admitted() -> bool {
    false
}

/// AS-IS: the CQE res twin looks like a proven ring (the 0074 P2.2 hole).
#[must_use]
pub fn cqe_ring_model_admitted_as_is() -> bool {
    true
}

/// Decide what to do after a submit attempt plus a non-blocking CQ drain.
///
/// `submit_ok` is kept so as-is tests can contrast F203/F208 and so the
/// F208 counter can see a failed submit. Production still WaitMore either
/// way: a pushed SQE is in flight until its CQE is harvested.
#[cfg_attr(not(test), allow(unused_variables))]
pub fn submit_complete_act(submit_ok: bool, harvested: bool) -> SubmitCompleteAct {
    if harvested {
        SubmitCompleteAct::UseHarvested
    } else {
        #[cfg(test)]
        if !submit_ok {
            F208_WAITMORE_AFTER_SUBMIT_ERR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        SubmitCompleteAct::WaitMore
    }
}

/// AS-IS F203 after a failed submit: never harvest; next op may adopt the
/// leftover CQE because tags are constants.
#[cfg(test)]
fn submit_complete_act_as_is(submit_ok: bool, _harvested: bool) -> SubmitCompleteAct {
    if submit_ok {
        SubmitCompleteAct::WaitMore
    } else {
        SubmitCompleteAct::ReturnSubmitErr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_tags_skip_zero_and_do_not_collide() {
        let mut c = FIRST_USER_DATA;
        let a = next_user_data(&mut c);
        let b = next_user_data(&mut c);
        assert_ne!(a, 0);
        assert_ne!(b, 0);
        assert_ne!(a, b);
        assert_eq!(cqe_act(a, b), CqeAct::Discard);
        assert_eq!(cqe_act(a, a), CqeAct::Take);
    }

    #[test]
    fn wrapping_skips_zero() {
        let mut c = u64::MAX;
        assert_eq!(next_user_data(&mut c), u64::MAX);
        assert_eq!(next_user_data(&mut c), 1);
        assert_ne!(c, 0);
    }

    #[test]
    fn constant_opcode_tags_take_leftover_as_current() {
        // F203-as-is: every write uses 0x77. A leftover write CQE is the next
        // write's result — false Ok / wrong length (U1).
        let leftover = next_user_data_as_is(&mut 0, TAG_WRITE_AS_IS);
        let next = next_user_data_as_is(&mut 0, TAG_WRITE_AS_IS);
        assert_eq!(leftover, next);
        assert_eq!(cqe_act(leftover, next), CqeAct::Take);
        assert_eq!(cqe_act(TAG_WRITE_AS_IS, TAG_FSYNC_AS_IS), CqeAct::Discard);
        assert_eq!(cqe_act(TAG_DIR_AS_IS, TAG_DIR_AS_IS), CqeAct::Take);
    }

    #[test]
    fn unique_tags_discard_leftover_same_opcode() {
        let mut c = FIRST_USER_DATA;
        let leftover = next_user_data(&mut c);
        let next = next_user_data(&mut c);
        assert_eq!(cqe_act(leftover, next), CqeAct::Discard);
        assert_ne!(
            cqe_act(leftover, next),
            cqe_act(TAG_WRITE_AS_IS, TAG_WRITE_AS_IS)
        );
    }

    #[test]
    fn cqe_negative_res_is_not_ok() {
        assert!(cqe_res_ok(0));
        assert!(cqe_res_ok(16));
        assert!(!cqe_res_ok(-5));
        assert!(!cqe_res_ok(-1));
        assert!(cqe_res_ok_as_is(-5), "AS-IS dente: negative CQE looks Ok");
    }

    /// RFC-0074 P2.2: twin of `cqe_res_ok` is not a ring model.
    #[test]
    fn cqe_ring_model_is_not_admitted() {
        assert!(!cqe_ring_model_admitted());
        assert!(
            cqe_ring_model_admitted_as_is(),
            "AS-IS dente: res-gate twin looks like a ring proof"
        );
        #[cfg(not(miri))]
        {
            let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
            assert!(
                crate_dir.join("verus/cqe_res.rs").is_file(),
                "RFC-0074 P2.1: cqe_res_ok twin must exist"
            );
            assert!(
                !crate_dir.join("verus/ring_model.rs").exists(),
                "RFC-0074 P2.2: ring Verus twin must stay absent"
            );
        }
    }

    #[test]
    fn harvest_on_submit_err_uses_cqe() {
        assert_eq!(
            submit_complete_act(false, true),
            SubmitCompleteAct::UseHarvested
        );
        assert_eq!(
            submit_complete_act_as_is(false, true),
            SubmitCompleteAct::ReturnSubmitErr
        );
    }

    #[test]
    fn submit_err_without_cqe_returns_err() {
        // F203 as-is *and* F208 as-is: empty CQ + submit Err → return.
        assert_eq!(
            submit_complete_act_as_is(false, false),
            SubmitCompleteAct::ReturnSubmitErr
        );
        // Production (F208): SQE is already in the SQ; wait for its CQE.
        assert_eq!(
            submit_complete_act(false, false),
            SubmitCompleteAct::WaitMore
        );
    }

    /// Buffer-liveness oracle for the EINTR-after-push schedule.
    ///
    /// `pwrite` borrows `buf` until `submit_sqe` returns. The kernel may
    /// still DMA that buffer until the matching CQE. Returning from
    /// `submit_sqe` without harvesting drops `buf` in the caller — UAF if a
    /// CQE can still fire.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum BufLiveness {
        /// CQE taken while the caller still borrows `buf`.
        DmaWhileLive,
        /// Function returned Err; CQE (DMA) happens after `buf` is dropped.
        DmaAfterReturn,
    }

    fn eintr_then_late_cqe(
        policy: fn(bool, bool) -> SubmitCompleteAct,
    ) -> BufLiveness {
        // Drain 1: submit_and_wait EINTR, CQ empty.
        match policy(false, false) {
            SubmitCompleteAct::ReturnSubmitErr => BufLiveness::DmaAfterReturn,
            SubmitCompleteAct::WaitMore => {
                // Still inside submit_sqe; buf live. Late CQE appears.
                assert_eq!(
                    policy(false, true),
                    SubmitCompleteAct::UseHarvested
                );
                BufLiveness::DmaWhileLive
            }
            SubmitCompleteAct::UseHarvested => {
                panic!("no CQE on first drain");
            }
        }
    }

    #[test]
    fn f208_eintr_late_cqe_as_is_dma_after_return() {
        assert_eq!(
            eintr_then_late_cqe(submit_complete_act_as_is),
            BufLiveness::DmaAfterReturn
        );
        assert_eq!(
            eintr_then_late_cqe(submit_complete_act),
            BufLiveness::DmaWhileLive
        );
    }

    #[test]
    fn submit_ok_without_cqe_waits() {
        assert_eq!(
            submit_complete_act(true, false),
            SubmitCompleteAct::WaitMore
        );
        assert_eq!(
            submit_complete_act(true, true),
            SubmitCompleteAct::UseHarvested
        );
    }

    #[test]
    fn as_is_constant_fsync_false_ok() {
        // Leftover successful fsync (0x5f) is taken as the *next* fsync.
        let leftover = TAG_FSYNC_AS_IS;
        let next = next_user_data_as_is(&mut 0, TAG_FSYNC_AS_IS);
        assert_eq!(cqe_act(leftover, next), CqeAct::Take);
        let mut c = FIRST_USER_DATA;
        let leftover_u = next_user_data(&mut c);
        let next_u = next_user_data(&mut c);
        assert_eq!(cqe_act(leftover_u, next_u), CqeAct::Discard);
    }

    /// RFC-0156 P0.3 (R-uring): sequence sweep — 64 issued ops, and at
    /// every CQ drain position a leftover from every other op may be
    /// visible. Under unique `user_data`, a leftover is always
    /// `Discard`, `submit_complete_act(_, false)` is always `WaitMore`
    /// (never a false Ok from someone else's CQE), and a negative `res`
    /// on our own tag is never Ok. The false-Ok path exists only in the
    /// AS-IS twins (constant tags + `cqe_res_ok_as_is`) — the dente.
    #[test]
    fn cqe_leftover_sequence_never_false_ok() {
        const OPS: usize = 64;
        let mut counter = FIRST_USER_DATA;
        let tags: Vec<u64> = (0..OPS).map(|_| next_user_data(&mut counter)).collect();

        // Tags are unique and never zero (the F203 invariant).
        let mut seen = std::collections::HashSet::new();
        for t in &tags {
            assert_ne!(*t, 0);
            assert!(seen.insert(*t), "user_data tag collision: {t}");
        }

        // At every position i, every other tag is a leftover: Discard.
        // Only the own tag is Take — wrong-tag adoption never happens.
        for i in 0..OPS {
            for j in 0..OPS {
                assert_eq!(
                    cqe_act(tags[j], tags[i]),
                    if i == j { CqeAct::Take } else { CqeAct::Discard },
                    "leftover tag {} at op {} must not be adopted",
                    tags[j],
                    tags[i]
                );
            }
        }

        // Any drain that has not seen our tag waits again — a submit
        // error or an Ok submit with only leftovers is never a false
        // completion (F208 keeps the caller's buffer alive).
        for submit_ok in [true, false] {
            for i in 0..OPS {
                // Drain contains every leftover except our own tag.
                for j in 0..OPS {
                    if i == j {
                        continue;
                    }
                    assert_eq!(cqe_act(tags[j], tags[i]), CqeAct::Discard);
                }
                assert_eq!(
                    submit_complete_act(submit_ok, false),
                    SubmitCompleteAct::WaitMore,
                    "no own CQE yet must wait (submit_ok={submit_ok})"
                );
            }
        }
        // The harvested CQE is the only thing that can produce a result,
        // and its res is gated: kernel errno is not Ok.
        assert!(!cqe_res_ok(-5));
        assert!(cqe_res_ok(0));
        assert!(cqe_res_ok(4096));
        assert!(cqe_res_ok_as_is(-5), "AS-IS dente: any res is Ok");

        // AS-IS contrast: constant per-opcode tags make the leftover
        // fsync (res=0) look like the current fsync → false Ok.
        let mut c0 = 0u64;
        let fsync_tag = next_user_data_as_is(&mut c0, TAG_FSYNC_AS_IS);
        assert_eq!(cqe_act(TAG_FSYNC_AS_IS, fsync_tag), CqeAct::Take);
        assert!(cqe_res_ok_as_is(0));
    }
}
