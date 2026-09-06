// Close twin of ops_kernel::pitr_record_in_window (PITR replay filter).
//
// Domínio real: u64. Espec em int. O claim: um record com seq depois do
// target nunca entra no restore (as-is abençoa qualquer seq > base).

use vstd::prelude::*;

verus! {

pub open spec fn pitr_record_in_window_spec(ms: int, base: int, target: int) -> bool {
    ms > base && ms <= target
}

pub open spec fn pitr_record_in_window_as_is_spec(ms: int, base: int, target: int) -> bool {
    ms > base
}

pub fn entry(ms: u64, base: u64, target: u64) -> (r: bool)
    ensures r == pitr_record_in_window_spec(ms as int, base as int, target as int)
{
    ms > base && ms <= target
}

pub fn entry_as_is(ms: u64, base: u64, target: u64) -> (r: bool)
    ensures r == pitr_record_in_window_as_is_spec(ms as int, base as int, target as int)
{
    ms > base
}

// Seq depois do target: fixado recusa, as-is abençoa (PITR mente).
proof fn lemma_as_is_leaks_future_seq(ms: int, base: int, target: int)
    requires ms > target && ms > base
    ensures
        !pitr_record_in_window_spec(ms, base, target),
        pitr_record_in_window_as_is_spec(ms, base, target),
{
}

// Testemunha concreta: seq 4, base 1, target 3 (o dente do crate).
proof fn lemma_seq4_at_target3_as_is_blesses() {
    assert(!pitr_record_in_window_spec(4, 1, 3));
    assert(pitr_record_in_window_as_is_spec(4, 1, 3));
}

// O target é inclusivo: seq == target entra nos dois.
proof fn lemma_target_is_inclusive(base: int, target: int)
    requires target > base
    ensures
        pitr_record_in_window_spec(target, base, target),
        pitr_record_in_window_as_is_spec(target, base, target),
{
}

// O base é exclusivo: seq == base fica de fora nos dois (já está no checkpoint).
proof fn lemma_base_is_exclusive(base: int, target: int)
    requires target >= base
    ensures
        !pitr_record_in_window_spec(base, base, target),
        !pitr_record_in_window_as_is_spec(base, base, target),
{
}

fn main() {}

}
