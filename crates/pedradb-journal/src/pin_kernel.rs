//! Pure journal pin decisions (Slipstream H1 / stream F54 cousin).
//!
//! Production [`crate::JournalConsumer::peek`] / [`crate::JournalConsumer::pin_after_apply`]
//! / [`crate::JournalConsumer::catch_up`] call these. Persist of the pin is
//! **caller + axiom**.

#![forbid(unsafe_code)]

/// Peek must **not** persist the pin (fold / `watch_applied`).
#[must_use]
pub fn peek_pins_cursor() -> bool {
    false
}

/// AS-IS H1: pin on receipt (pre-fix `catch_up` / stream `next`).
#[must_use]
pub fn peek_pins_cursor_as_is() -> bool {
    true
}

/// Canary `catch_up` pins on read (W4). Fold must not use that path.
#[must_use]
pub fn catch_up_pins_on_read() -> bool {
    true
}

/// Fold-safe: pin only after apply.
#[must_use]
pub fn fold_pins_on_read() -> bool {
    false
}

/// Advance the pin only past what the caller has applied.
#[must_use]
pub fn may_advance_pin(pin: u64, applied_through: u64) -> bool {
    applied_through > pin
}

/// Next pin after a catch-up batch (`None` = empty).
#[must_use]
pub fn next_pin(pin: u64, batch_max: Option<u64>) -> u64 {
    match batch_max {
        Some(m) if m > pin => m,
        _ => pin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peek_does_not_pin() {
        assert!(!peek_pins_cursor());
        assert!(peek_pins_cursor_as_is());
        assert_ne!(peek_pins_cursor(), peek_pins_cursor_as_is());
    }

    #[test]
    fn fold_must_not_pin_on_read() {
        assert!(!fold_pins_on_read());
        assert!(catch_up_pins_on_read());
    }

    #[test]
    fn pin_only_after_applied() {
        assert!(may_advance_pin(0, 1));
        assert!(!may_advance_pin(3, 3));
        assert!(!may_advance_pin(3, 2));
    }

    #[test]
    fn next_pin_monotonic() {
        assert_eq!(next_pin(3, None), 3);
        assert_eq!(next_pin(3, Some(2)), 3);
        assert_eq!(next_pin(3, Some(5)), 5);
    }
}
