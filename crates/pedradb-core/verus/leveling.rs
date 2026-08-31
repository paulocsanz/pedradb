// Verus twin, catalog pair `leveling` (close tier): the level-size ladder
// of the leveled-compaction kernel.
//
// Source of truth for production remains `src/leveling.rs` (called by
// `Db::prepare_l0_compact`, `Db::prepare_pushdown_compact`, and
// `Db::compact_leveled` via `leveled_enabled`/`level_target_bytes`).
//
// `level_target_bytes` transcribed exec == spec under the production
// no-saturation bound — the ladder the scheduler trusts to bound job
// cascades. The selection decisions (pick_l0_to_l1/pick_pushdown) are the
// atom-tier pair `leveling_pick`, twin in `verus/leveling_pick.rs`.
//
//   ./scripts/verus_leveling.sh
//
// Do not link this into the production crate — it is a twin of the pure kernel.

use vstd::prelude::*;
use vstd::arithmetic::mul::*;

verus! {

broadcast use vstd::arithmetic::mul::lemma_mul_is_commutative, vstd::arithmetic::mul::lemma_mul_inequality;

pub open spec fn ten_pow(exp: int) -> int
    decreases exp,
{
    if exp <= 0 { 1int } else { 10int * ten_pow(exp - 1) }
}

/// Spec domain is `int`; `level == 0` has no target, the ladder is
/// `l1_target * LEVEL_FANOUT^min(level-1, 18)` with `LEVEL_FANOUT == 10`.
pub open spec fn level_target_bytes_spec(level: int, l1_target: int) -> int {
    if level <= 0 {
        0int
    } else {
        let exp = if level >= 19 { 18int } else { level - 1 };
        l1_target * ten_pow(exp)
    }
}

/// Mirrors `leveling::level_target_bytes` bit-for-bit on every input where
/// the saturating arms do not engage (the precondition states exactly
/// that). Production `l1_target` is a configured byte target and levels
/// stay single-digit, far inside this bound.
pub fn level_target_bytes(level: u32, l1_target: u64) -> (t: u64)
    requires
        level_target_bytes_spec(level as int, l1_target as int) <= 0xffff_ffff_ffff_ffffint,
    ensures
        t as int == level_target_bytes_spec(level as int, l1_target as int),
        level == 0 ==> t == 0,
        level >= 1 ==> t >= l1_target,
{
    if level == 0 {
        0
    } else if l1_target == 0 {
        0
    } else {
        let exp: u32 = if level >= 19 { 18 } else { level - 1 };
        let p = ten_pow_exec(exp, l1_target);
        assert(ten_pow(exp as int) >= 1) by {
            lemma_ten_pow_pos(exp as int);
        };
        assert(1int * l1_target as int <= ten_pow(exp as int) * l1_target as int);
        l1_target * p
    }
}

/// Exec mirror of `ten_pow` carrying the caller's no-overflow bound as its
/// precondition: `cap * 10^exp ≤ u64::MAX` with `cap ≥ 1` makes every
/// multiplication in the recursion provably in-range (the saturating arms
/// never engage).
fn ten_pow_exec(exp: u32, cap: u64) -> (p: u64)
    requires
        cap >= 1,
        cap as int * ten_pow(exp as int) <= 0xffff_ffff_ffff_ffffint,
    ensures
        p as int == ten_pow(exp as int),
    decreases exp,
{
    if exp == 0 {
        1
    } else {
        assert(ten_pow((exp - 1) as int) <= ten_pow(exp as int)) by {
            lemma_ten_pow_monotone((exp - 1) as int, exp as int);
        };
        assert(ten_pow((exp - 1) as int) * cap as int <= ten_pow(exp as int) * cap as int);
        assert(ten_pow((exp - 1) as int) * cap as int <= 0xffff_ffff_ffff_ffffint);
        assert(1int * ten_pow(exp as int) <= cap as int * ten_pow(exp as int));
        assert(ten_pow(exp as int) <= 0xffff_ffff_ffff_ffffint);
        let child = ten_pow_exec((exp - 1) as u32, cap);
        child * 10
    }
}

proof fn lemma_ten_pow_pos(exp: int)
    requires
        exp >= 0,
    ensures
        ten_pow(exp) >= 1,
    decreases exp,
{
    if exp == 0 {
    } else {
        lemma_ten_pow_pos(exp - 1);
    }
}

proof fn lemma_ten_pow_monotone(a: int, b: int)
    requires
        0 <= a,
        a <= b,
    ensures
        ten_pow(a) <= ten_pow(b),
    decreases b - a,
{
    if a == b {
    } else {
        lemma_ten_pow_monotone(a, b - 1);
        lemma_ten_pow_pos(a);
    }
}

/// Named lemma (ladder): targets never shrink as the level grows.
proof fn lemma_targets_monotone(a: int, b: int, l1_target: int)
    requires
        1 <= a,
        a <= b,
        l1_target >= 1,
    ensures
        level_target_bytes_spec(a, l1_target) <= level_target_bytes_spec(b, l1_target),
{
    let ea = if a >= 19 { 18int } else { a - 1 };
    let eb = if b >= 19 { 18int } else { b - 1 };
    assert(ea <= eb) by {
        if a >= 19 {
            assert(eb <= 18);
        } else if b >= 19 {
            assert(a - 1 <= 18);
        }
    };
    lemma_ten_pow_monotone(ea, eb);
    assert(ten_pow(ea) * l1_target <= ten_pow(eb) * l1_target);
}

} // verus!
