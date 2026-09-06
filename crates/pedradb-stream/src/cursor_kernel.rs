//! Pure consumer-cursor decisions (RFC-0002 P20 / F54).
//!
//! Production [`crate::Stream::peek`] / [`crate::Stream::ack`] call these
//! helpers. Persist of the cursor is **caller + axiom**.

#![forbid(unsafe_code)]

/// Next sequence after last **acked** (`0` = none).
#[must_use]
pub fn next_seq(last_acked: u64) -> u64 {
    last_acked.saturating_add(1)
}

/// F54: ack only the immediate next seq (no holes, no skip).
#[must_use]
pub fn ack_in_order(last_acked: u64, seq: u64) -> bool {
    seq == next_seq(last_acked) && seq > last_acked
}

/// AS-IS F54: any `seq > last` pins (skips unacked messages on reopen).
#[must_use]
pub fn ack_in_order_as_is(last_acked: u64, seq: u64) -> bool {
    seq > last_acked
}

/// Peek must **not** persist the cursor (pin-on-read).
#[must_use]
pub fn peek_pins_cursor() -> bool {
    false
}

/// AS-IS: `next` persisted cursor before returning the payload.
#[must_use]
pub fn peek_pins_cursor_as_is() -> bool {
    true
}

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
