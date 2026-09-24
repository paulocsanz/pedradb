//! WAL pwrite ticket (RFC-0193 / RFC-0230 P0.3). Integer arithmetic, no I/O.
//!
//! Under `wal.lock()` the leader only **reserves** `len` bytes at the
//! frontier and takes a ticket (the byte offset). The sink write happens
//! at that offset (`EnvFile::write_all_at`) without holding the meta lock
//! when the handle supports positional writes. File order = ticket order.
//!
//! AS-IS twin: no separate reservation cursor — the write stays serialized
//! at `reserved_to` under the lock (seek/write of today).

#![forbid(unsafe_code)]

/// Reserve `len` bytes at `reserved_to`.
///
/// Returns `(ticket, new_reserved_to)`. `ticket` is the offset the caller
/// must `write_all_at`. Empty `len` does not advance (a zero-byte reserve
/// is a no-op, not a hole).
#[must_use]
pub fn reserve_frame(reserved_to: u64, len: u64) -> (u64, u64) {
    if crate::write_admission_kernel::batch_is_empty(len) {
        return (reserved_to, reserved_to);
    }
    (reserved_to, reserved_to.saturating_add(len))
}

/// AS-IS twin of [`reserve_frame`]: do not advance a distinct reservation
/// cursor. The caller writes under the lock at `reserved_to` (the live
/// `position()`). Ticket equals `reserved_to`; new frontier is unchanged
/// by this fn — the sequential write itself advances `position`.
#[must_use]
pub fn reserve_frame_as_is(reserved_to: u64, _len: u64) -> (u64, u64) {
    (reserved_to, reserved_to)
}

/// Whether the opt-in pwrite path may leave the WAL meta lock.
/// `want` is the env pin; `can` is [`crate::env::EnvFile::positional_writes`].
#[must_use]
pub fn pwrite_off_lock(want: bool, can: bool) -> bool {
    want && can
}

/// AS-IS twin of [`pwrite_off_lock`]: always stay on the locked sequential
/// write (pre-0193).
#[must_use]
pub fn pwrite_off_lock_as_is(_want: bool, _can: bool) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc0193_reserve_frame_advances_ticket() {
        let (t0, r1) = reserve_frame(0, 10);
        assert_eq!(t0, 0);
        assert_eq!(r1, 10);
        let (t1, r2) = reserve_frame(r1, 5);
        assert_eq!(t1, 10);
        assert_eq!(r2, 15);
        let (tz, rz) = reserve_frame(15, 0);
        assert_eq!(tz, 15);
        assert_eq!(rz, 15, "zero-len reserve is a no-op");
    }

    #[test]
    fn rfc0193_as_is_does_not_advance_separate_cursor() {
        let (t, r) = reserve_frame_as_is(40, 100);
        assert_eq!(t, 40);
        assert_eq!(r, 40);
    }

    #[test]
    fn rfc0193_pwrite_off_lock_requires_want_and_capability() {
        assert!(pwrite_off_lock(true, true));
        assert!(!pwrite_off_lock(true, false));
        assert!(!pwrite_off_lock(false, true));
        assert!(!pwrite_off_lock(false, false));
        assert!(!pwrite_off_lock_as_is(true, true));
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    #[kani::proof]
    fn kani_reserve_frame_monotonic_and_partition() {
        let reserved_to: u64 = kani::any();
        let len: u64 = kani::any();
        let (ticket, new_reserved) = reserve_frame(reserved_to, len);
        assert_eq!(ticket, reserved_to);
        if len == 0 {
            assert_eq!(new_reserved, reserved_to);
        } else {
            assert!(new_reserved >= reserved_to);
            if reserved_to <= u64::MAX - len {
                assert_eq!(new_reserved, reserved_to + len);
            }
        }
    }

    #[kani::proof]
    fn kani_reserve_frame_two_steps_continuous() {
        let r0: u64 = kani::any();
        let l1: u64 = kani::any();
        let l2: u64 = kani::any();
        let (t1, r1) = reserve_frame(r0, l1);
        let (t2, r2) = reserve_frame(r1, l2);
        assert_eq!(t1, r0);
        assert_eq!(t2, r1);
        if l1 > 0 && r0 <= u64::MAX - l1 {
            assert_eq!(t2, t1 + l1);
        }
        if l2 > 0 && r1 <= u64::MAX - l2 {
            assert_eq!(r2, t2 + l2);
        }
    }

    #[kani::proof]
    fn kani_pwrite_off_lock_matches_truth_table() {
        let want: bool = kani::any();
        let can: bool = kani::any();
        let decision = pwrite_off_lock(want, can);
        assert_eq!(decision, want && can);
        let as_is = pwrite_off_lock_as_is(want, can);
        assert!(!as_is);
    }
}
