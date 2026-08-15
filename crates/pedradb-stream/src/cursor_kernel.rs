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
}
