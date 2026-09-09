//! Pure journal pin decisions (Slipstream H1 / stream F54 cousin).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_journal_pin.sh
//!
//! Production [`crate::JournalConsumer::peek`] / [`crate::JournalConsumer::pin_after_apply`]
//! / [`crate::JournalConsumer::catch_up`] call these. Persist of the pin is
//! **caller + axiom**.

#![forbid(unsafe_code)]

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
macro_rules! catch_up_pins_on_read_body {
    () => {
        true
    };
}
macro_rules! catch_up_pins_on_read_as_is_body {
    () => {
        false
    };
}
macro_rules! fold_pins_on_read_body {
    () => {
        false
    };
}
macro_rules! fold_pins_on_read_as_is_body {
    () => {
        true
    };
}
macro_rules! may_advance_pin_body {
    ($pin:expr, $applied_through:expr) => {
        $applied_through > $pin
    };
}
macro_rules! may_advance_pin_as_is_body {
    ($pin:expr, $applied_through:expr) => {{
        let _ = ($pin, $applied_through);
        true
    }};
}
macro_rules! next_pin_body {
    ($pin:expr, $batch_max:expr) => {
        match $batch_max {
            Some(m) if m > $pin => m,
            _ => $pin,
        }
    };
}
macro_rules! next_pin_as_is_body {
    ($pin:expr, $batch_max:expr) => {
        match $batch_max {
            Some(m) => m,
            None => $pin,
        }
    };
}

/// Peek must **not** persist the pin (fold / `watch_applied`).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn peek_pins_cursor() -> bool {
    peek_pins_cursor_body!()
}

/// AS-IS H1: pin on receipt (pre-fix `catch_up` / stream `next`).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn peek_pins_cursor_as_is() -> bool {
    peek_pins_cursor_as_is_body!()
}

/// Canary `catch_up` pins on read (W4). Fold must not use that path.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn catch_up_pins_on_read() -> bool {
    catch_up_pins_on_read_body!()
}

/// AS-IS W4: dead canary — `catch_up` never pins (eternal re-read).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn catch_up_pins_on_read_as_is() -> bool {
    catch_up_pins_on_read_as_is_body!()
}

/// Fold-safe: pin only after apply.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn fold_pins_on_read() -> bool {
    fold_pins_on_read_body!()
}

/// AS-IS H1: fold pins on read — entry lost between read and apply.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn fold_pins_on_read_as_is() -> bool {
    fold_pins_on_read_as_is_body!()
}

/// Advance the pin only past what the caller has applied.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn may_advance_pin(pin: u64, applied_through: u64) -> bool {
    may_advance_pin_body!(pin, applied_through)
}

/// AS-IS H1: always advance (pin on receipt / go backwards).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn may_advance_pin_as_is(pin: u64, applied_through: u64) -> bool {
    may_advance_pin_as_is_body!(pin, applied_through)
}

/// Next pin after a catch-up batch (`None` = empty).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn next_pin(pin: u64, batch_max: Option<u64>) -> u64 {
    next_pin_body!(pin, batch_max)
}

/// AS-IS H1: pin-on-receipt trusts an older batch max — pin regresses.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn next_pin_as_is(pin: u64, batch_max: Option<u64>) -> u64 {
    next_pin_as_is_body!(pin, batch_max)
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

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

pub fn catch_up_pins_on_read() -> (d: bool)
    ensures
        d,
{
    catch_up_pins_on_read_body!()
}

pub open spec fn catch_up_pins_on_read_as_is_spec() -> bool {
    false
}

pub fn catch_up_pins_on_read_as_is() -> (d: bool)
    ensures
        d == false,
        d == catch_up_pins_on_read_as_is_spec(),
{
    catch_up_pins_on_read_as_is_body!()
}

pub open spec fn fold_pins_on_read_spec() -> bool {
    false
}

pub fn fold_pins_on_read() -> (d: bool)
    ensures
        d == fold_pins_on_read_spec(),
{
    fold_pins_on_read_body!()
}

pub open spec fn fold_pins_on_read_as_is_spec() -> bool {
    true
}

pub fn fold_pins_on_read_as_is() -> (d: bool)
    ensures
        d == true,
        d == fold_pins_on_read_as_is_spec(),
{
    fold_pins_on_read_as_is_body!()
}

pub fn may_advance_pin(pin: u64, applied_through: u64) -> (d: bool)
    ensures
        d == (applied_through > pin),
{
    may_advance_pin_body!(pin, applied_through)
}

pub fn may_advance_pin_as_is(pin: u64, applied_through: u64) -> (d: bool)
    ensures
        d == true,
{
    may_advance_pin_as_is_body!(pin, applied_through)
}

pub open spec fn next_pin_spec(pin: u64, batch_max: Option<u64>) -> u64 {
    match batch_max {
        Some(m) if m > pin => m,
        _ => pin,
    }
}

pub fn next_pin(pin: u64, batch_max: Option<u64>) -> (n: u64)
    ensures
        n == next_pin_spec(pin, batch_max),
        n >= pin,
{
    next_pin_body!(pin, batch_max)
}

pub open spec fn next_pin_as_is_spec(pin: u64, batch_max: Option<u64>) -> u64 {
    match batch_max {
        Some(m) => m,
        None => pin,
    }
}

pub fn next_pin_as_is(pin: u64, batch_max: Option<u64>) -> (n: u64)
    ensures
        n == next_pin_as_is_spec(pin, batch_max),
{
    next_pin_as_is_body!(pin, batch_max)
}

proof fn lemma_next_pin_as_is_regresses(pin: u64, m: u64)
    requires
        m < pin,
    ensures
        next_pin_spec(pin, Some(m)) == pin,
        next_pin_as_is_spec(pin, Some(m)) < pin,
{
}

proof fn lemma_as_is_pins_on_peek()
    ensures
        peek_pins_cursor_as_is_spec(),
        !fold_pins_on_read_spec(),
{
}

proof fn lemma_empty_batch_keeps_pin(pin: u64)
    ensures
        next_pin_spec(pin, None) == pin,
{
}

} // verus!

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

    fn temp_db() -> (std::path::PathBuf, pedradb_core::Db<pedradb_core::StdEnv>) {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("pedra-pin-kernel-{n}"));
        let _ = std::fs::remove_dir_all(&d);
        let db = pedradb_core::Db::open(&d).unwrap();
        (d, db)
    }

    /// Catalog three-teeth plant: the canary gate must actually pin.
    #[test]
    fn catch_up_pins_on_read_on_live_journal_is_not_ok() {
        let (dir, mut db) = temp_db();
        let s = db.put_with_seq(b"a", b"1").unwrap();
        assert!(
            catch_up_pins_on_read(),
            "live catch_up gate must pin on read"
        );
        let mut c = crate::JournalConsumer::new();
        let batch = c.catch_up(&db);
        assert!(!batch.is_empty());
        assert_eq!(c.pin, s, "live catch_up advances the pin to batch max");
        assert!(
            !catch_up_pins_on_read_as_is(),
            "AS-IS dente: dead canary — catch_up never pins, the same batch is re-read forever"
        );
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Catalog three-teeth plant: fold must pin only after apply.
    #[test]
    fn fold_pins_on_read_on_live_journal_is_not_ok() {
        let (dir, mut db) = temp_db();
        let s = db.put_with_seq(b"a", b"1").unwrap();
        assert!(!fold_pins_on_read(), "live fold must not pin on read");
        let mut c = crate::JournalConsumer::new();
        let batch = c.peek(&db);
        assert!(!batch.is_empty());
        assert_eq!(c.pin, 0, "peek leaves the pin untouched");
        c.pin_after_apply(s);
        assert_eq!(c.pin, s, "pin moves only after apply");
        assert!(
            fold_pins_on_read_as_is(),
            "AS-IS dente: fold pins on read — a crash between read and apply loses the entry"
        );
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Catalog three-teeth plant: the pin never regresses on an older receipt.
    #[test]
    fn next_pin_on_live_journal_is_not_ok() {
        assert_eq!(next_pin(5, Some(3)), 5, "production never regresses");
        assert_eq!(next_pin(5, None), 5, "empty batch keeps the pin");
        let mut c = crate::JournalConsumer::new();
        c.pin_after_apply(5);
        assert_eq!(c.pin, 5);
        c.pin_after_apply(3);
        assert_eq!(c.pin, 5, "live pin never regresses on older receipt");
        assert_eq!(
            next_pin_as_is(5, Some(3)),
            3,
            "AS-IS dente: pin-on-receipt trusts an older batch max — pin regresses past applied"
        );
    }
}
