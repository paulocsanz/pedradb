//! Pure consumer-cursor decisions (RFC-0002 P20 / F54).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_stream_cursor.sh
//!
//! Production [`crate::Stream::peek`] / [`crate::Stream::ack`] call these
//! helpers. Persist of the cursor is **caller + axiom**.

#![forbid(unsafe_code)]

macro_rules! next_seq_body {
    ($last_acked:expr) => {
        $last_acked.saturating_add(1)
    };
}

macro_rules! next_seq_as_is_body {
    ($last_acked:expr) => {
        $last_acked
    };
}

macro_rules! ack_in_order_body {
    ($last_acked:expr, $seq:expr) => {
        $seq == next_seq($last_acked) && $seq > $last_acked
    };
}

macro_rules! ack_in_order_as_is_body {
    ($last_acked:expr, $seq:expr) => {
        $seq > $last_acked
    };
}

macro_rules! peek_pins_cursor_body {
    () => {
        false
    };
}

macro_rules! peek_pins_cursor_as_is_body {
    () => {
        true
    };
}

/// Next sequence after last **acked** (`0` = none).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn next_seq(last_acked: u64) -> u64 {
    next_seq_body!(last_acked)
}

/// AS-IS F54: the cursor read as *next-to-read* (off-by-one) — `peek`
/// re-hands the already-acked message (or nothing at cursor 0) and the
/// consumer never advances. Used only to prove the fixed rule has teeth.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn next_seq_as_is(last_acked: u64) -> u64 {
    next_seq_as_is_body!(last_acked)
}

/// F54: ack only the immediate next seq (no holes, no skip).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn ack_in_order(last_acked: u64, seq: u64) -> bool {
    ack_in_order_body!(last_acked, seq)
}

/// AS-IS F54: any `seq > last` pins (skips unacked messages on reopen).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn ack_in_order_as_is(last_acked: u64, seq: u64) -> bool {
    ack_in_order_as_is_body!(last_acked, seq)
}

/// Peek must **not** persist the cursor (pin-on-read).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn peek_pins_cursor() -> bool {
    peek_pins_cursor_body!()
}

/// AS-IS: `next` persisted cursor before returning the payload.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn peek_pins_cursor_as_is() -> bool {
    peek_pins_cursor_as_is_body!()
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn sat_add1_spec(x: u64) -> u64 {
    if x == u64::MAX {
        x
    } else {
        (x + 1) as u64
    }
}

pub fn next_seq(last_acked: u64) -> (n: u64)
    ensures
        n == sat_add1_spec(last_acked),
        last_acked < u64::MAX ==> n == last_acked + 1,
{
    if last_acked == u64::MAX {
        last_acked
    } else {
        last_acked + 1
    }
}

pub fn ack_in_order(last_acked: u64, seq: u64) -> (d: bool)
    ensures
        d == (seq == sat_add1_spec(last_acked) && seq > last_acked),
{
    let n = next_seq(last_acked);
    seq == n && seq > last_acked
}

pub open spec fn next_seq_as_is_spec(last_acked: u64) -> u64 {
    last_acked
}

pub fn next_seq_as_is(last_acked: u64) -> (n: u64)
    ensures
        n == last_acked,
        n == next_seq_as_is_spec(last_acked),
{
    next_seq_as_is_body!(last_acked)
}

proof fn lemma_as_is_stuck(last: u64)
    requires
        last < u64::MAX,
    ensures
        next_seq_as_is_spec(last) == last,
        next_seq_as_is_spec(last) != sat_add1_spec(last),
{
}

pub open spec fn ack_in_order_as_is_spec(last_acked: u64, seq: u64) -> bool {
    seq > last_acked
}

pub fn ack_in_order_as_is(last_acked: u64, seq: u64) -> (d: bool)
    ensures
        d == (seq > last_acked),
        d == ack_in_order_as_is_spec(last_acked, seq),
{
    ack_in_order_as_is_body!(last_acked, seq)
}

proof fn lemma_as_is_skips(last: u64)
    requires
        last + 2 <= u64::MAX,
    ensures
        ack_in_order_as_is_spec(last, (last + 2) as u64),
        !((last + 2) as u64 == sat_add1_spec(last) && (last + 2) as u64 > last),
{
}

pub fn peek_pins_cursor() -> (d: bool)
    ensures
        !d,
{
    peek_pins_cursor_body!()
}

pub open spec fn peek_pins_cursor_as_is_spec() -> bool {
    true
}

pub fn peek_pins_cursor_as_is() -> (d: bool)
    ensures
        d == true,
        d == peek_pins_cursor_as_is_spec(),
{
    peek_pins_cursor_as_is_body!()
}

proof fn lemma_as_is_pins()
    ensures
        peek_pins_cursor_as_is_spec(),
{
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_only_next() {
        assert!(ack_in_order(0, 1));
        assert!(!ack_in_order(0, 2));
        assert!(!ack_in_order(3, 3));
        assert!(ack_in_order(3, 4));
        assert!(ack_in_order_as_is(0, 2));
    }

    #[test]
    fn peek_does_not_pin() {
        assert!(!peek_pins_cursor());
        assert!(peek_pins_cursor_as_is());
    }

    /// F54 three-teeth plant: the live stream hands the *next* un-acked
    /// seq (`next_seq`), never the cursor itself — the AS-IS off-by-one
    /// re-hands the acked message (or nothing at cursor 0) and stalls.
    #[test]
    fn next_seq_on_live_stream_is_not_ok() {
        use crate::Stream;
        assert_eq!(next_seq(0), 1);
        assert_eq!(next_seq(6), 7);
        assert_eq!(next_seq(u64::MAX), u64::MAX, "saturating at the top");
        assert_eq!(
            next_seq_as_is(0),
            0,
            "AS-IS dente: cursor read as next-to-read"
        );
        assert_eq!(next_seq_as_is(6), 6);

        let dir = std::env::temp_dir().join(format!(
            "pedra-cursor-next-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        {
            let mut s = Stream::open(&dir, "events").unwrap();
            assert_eq!(s.publish(b"m1").unwrap(), 1);
            assert_eq!(s.publish(b"m2").unwrap(), 2);
            // Fresh consumer (cursor 0): the live stream hands seq 1; the
            // AS-IS tooth would `get(0)` and hand NOTHING forever.
            let m = s.peek("c1").unwrap().expect("first message");
            assert_eq!((m.seq, m.data.as_slice()), (1, b"m1".as_slice()));
            s.ack("c1", 1).unwrap();
            // After acking 1 the live stream hands seq 2; the AS-IS tooth
            // would re-hand the ALREADY-ACKED seq 1 (ack refuses it).
            let m = s.peek("c1").unwrap().expect("second message");
            assert_eq!((m.seq, m.data.as_slice()), (2, b"m2".as_slice()));
            s.ack("c1", 2).unwrap();
            assert_eq!(s.consumer_seq("c1"), 2);
            assert!(s.peek("c1").unwrap().is_none());
            s.close().unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn theorem_on_small_domain() {
        for last in 0u64..6 {
            let n = next_seq(last);
            if last < u64::MAX {
                assert_eq!(n, last + 1);
            }
            for seq in 0u64..8 {
                let d = ack_in_order(last, seq);
                assert_eq!(d, seq == n && seq > last);
                if seq > last + 1 {
                    assert!(ack_in_order_as_is(last, seq));
                    assert!(!d);
                }
            }
        }
        assert!(!ack_in_order(u64::MAX, u64::MAX));
    }

    /// Class-F agreement twin (F54 `peek_pin_journal_stream`): sweeps the
    /// shared clone domain against the LIVE journal `pin_kernel`, so drift
    /// on either side fails `cargo test`, not only the token lint. Full
    /// domain: both fns are argument-less bools.
    #[test]
    fn twin_agrees_with_journal_pin_kernel_on_full_domain() {
        let mut checks = 0usize;
        for (name, stream_fn, journal_fn) in [
            (
                "peek_pins_cursor",
                peek_pins_cursor as fn() -> bool,
                pedradb_journal::pin_kernel::peek_pins_cursor as fn() -> bool,
            ),
            (
                "peek_pins_cursor_as_is",
                peek_pins_cursor_as_is as fn() -> bool,
                pedradb_journal::pin_kernel::peek_pins_cursor_as_is as fn() -> bool,
            ),
        ] {
            let (s, j) = (stream_fn(), journal_fn());
            assert_eq!(
                s, j,
                "peek_pin_journal_stream drift at {name}: stream {s:?} vs journal {j:?}"
            );
            checks += 1;
        }
        // Teeth: the fixed tooth must refuse pin-on-read on BOTH sides, and
        // fixed/as-is must disagree (a both-sides flip would cancel out).
        assert!(!peek_pins_cursor(), "stream fixed tooth must not pin");
        assert!(
            !pedradb_journal::pin_kernel::peek_pins_cursor(),
            "journal fixed tooth must not pin"
        );
        assert!(peek_pins_cursor_as_is(), "stream as-is tooth pins");
        assert!(
            pedradb_journal::pin_kernel::peek_pins_cursor_as_is(),
            "journal as-is tooth pins"
        );
        assert_ne!(
            peek_pins_cursor(),
            peek_pins_cursor_as_is(),
            "teeth: fixed and as-is must disagree"
        );
        assert_eq!(checks, 2, "exact agreement count: 2 fns, full domain");
    }
}
