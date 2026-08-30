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

/// AS-IS H1: always advance (pin on receipt / go backwards).
#[must_use]
pub fn may_advance_pin_as_is(_pin: u64, _applied_through: u64) -> bool {
    true
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

    /// Catalog three-teeth plant. Direct `pin_only_after_applied` is **not** this tooth.
    #[test]
    fn may_advance_pin_on_live_journal_is_not_ok() {
        assert!(may_advance_pin(0, 1));
        assert!(!may_advance_pin(1, 1));
        assert!(
            may_advance_pin_as_is(1, 1),
            "AS-IS dente: pin advances even when applied_through <= pin"
        );
        let mut c = crate::JournalConsumer::new();
        c.pin_after_apply(0);
        assert_eq!(c.pin, 0, "live pin_after_apply must not move on applied==pin");
        c.pin_after_apply(1);
        assert_eq!(c.pin, 1);
    }
}
