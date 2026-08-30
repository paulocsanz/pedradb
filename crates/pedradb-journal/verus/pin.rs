// Verus proof of journal pin rules (Slipstream H1 / F54 cousin).
// Twin of `src/pin_kernel.rs`. Not linked into production.
//
//   ./scripts/verus_journal_pin.sh

use vstd::prelude::*;

verus! {

pub fn peek_pins_cursor() -> (d: bool)
    ensures
        !d,
{
    false
}

pub open spec fn peek_pins_cursor_as_is() -> bool {
    true
}

pub fn catch_up_pins_on_read() -> (d: bool)
    ensures
        d,
{
    true
}

/// Spec twin of the kernel decision — proofs must not call exec fns
/// (pinned Verus rejects exec-mode calls in spec position).
pub open spec fn fold_pins_on_read_spec() -> bool {
    false
}

pub fn fold_pins_on_read() -> (d: bool)
    ensures
        d == fold_pins_on_read_spec(),
{
    false
}

pub fn may_advance_pin(pin: u64, applied_through: u64) -> (d: bool)
    ensures
        d == (applied_through > pin),
{
    applied_through > pin
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
    match batch_max {
        Some(m) if m > pin => m,
        _ => pin,
    }
}

proof fn lemma_as_is_pins_on_peek()
    ensures
        peek_pins_cursor_as_is(),
        !fold_pins_on_read_spec(),
{
}

proof fn lemma_empty_batch_keeps_pin(pin: u64)
    ensures
        next_pin_spec(pin, None) == pin,
{
}

} // verus!
