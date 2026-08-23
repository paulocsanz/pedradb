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
    /// the caller's buffer). Production never takes this arm after a push.
    ReturnSubmitErr,
}

/// Decide what to do after a submit attempt plus a non-blocking CQ drain.
///
/// `submit_ok` is kept so as-is tests can contrast F203/F208. Production
/// ignores it: a pushed SQE is in flight until its CQE is harvested.
pub fn submit_complete_act(_submit_ok: bool, harvested: bool) -> SubmitCompleteAct {
    if harvested {
        SubmitCompleteAct::UseHarvested
    } else {
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
}
