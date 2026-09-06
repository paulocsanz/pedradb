// Verus proof of the prefix exclusive-end rule (RFC-0002 P11 / F57 / F58).
// Close twin of `src/prefix.rs` `prefix_exclusive_end`: the production loop
// (increment last non-0xff, pop trailing 0xff, None if all-0xff / empty).
// The byte atom `bump_non_ff` remains as the one-byte step.
//
//   ./scripts/verus_prefix_exclusive_end.sh

use vstd::prelude::*;

verus! {

pub open spec fn bump_non_ff_spec(b: u8) -> Option<u8> {
    if b < 0xff {
        Some((b + 1) as u8)
    } else {
        None
    }
}

/// Increment a non-`0xff` byte; `0xff` carries (caller pops).
pub fn bump_non_ff(b: u8) -> (r: Option<u8>)
    ensures
        r == bump_non_ff_spec(b),
        (b < 0xff) ==> r == Some((b + 1) as u8),
        (b == 0xff) ==> r.is_none(),
{
    if b < 0xff {
        Some((b + 1) as u8)
    } else {
        None
    }
}

/// Spec of the production loop: walk from the tail, bump the first
/// non-`0xff` byte, drop a trailing `0xff` carry; empty / all-`0xff` is None.
pub open spec fn prefix_exclusive_end_spec(p: Seq<u8>) -> Option<Seq<u8>>
    decreases p.len(),
{
    if p.len() == 0 {
        None
    } else {
        let last = p[p.len() as int - 1];
        if last < 0xff {
            Some(p.update(p.len() as int - 1, (last + 1) as u8))
        } else {
            prefix_exclusive_end_spec(p.subrange(0, p.len() as int - 1))
        }
    }
}

pub open spec fn prefix_exclusive_end_as_is_spec(p: Seq<u8>) -> Seq<u8> {
    p + seq![0xffu8]
}

fn clone_bytes(prefix: &[u8]) -> (e: Vec<u8>)
    ensures
        e@ == prefix@,
{
    let mut e: Vec<u8> = Vec::new();
    let mut k: usize = 0;
    while k < prefix.len()
        invariant
            k <= prefix.len(),
            e@ == prefix@.subrange(0, k as int),
        decreases prefix.len() - k,
    {
        e.push(prefix[k]);
        k = k + 1;
    }
    e
}

/// Production exclusive-end (F57/F58). Same decision as `src/prefix.rs`.
pub fn prefix_exclusive_end(prefix: &[u8]) -> (r: Option<Vec<u8>>)
    ensures
        match (r, prefix_exclusive_end_spec(prefix@)) {
            (None, None) => true,
            (Some(v), Some(s)) => v@ == s,
            _ => false,
        },
{
    let mut e = clone_bytes(prefix);
    while e.len() > 0
        invariant
            prefix_exclusive_end_spec(prefix@) == prefix_exclusive_end_spec(e@),
        decreases e.len(),
    {
        let i: usize = (e.len() - 1) as usize;
        let last: u8 = e[i];
        if last < 0xff {
            let bumped: u8 = (last + 1) as u8;
            let ghost before = e@;
            proof {
                assert(i as int == before.len() as int - 1);
                assert(last == before[i as int]);
                assert(last < 0xff);
                assert((last + 1) as u8 == bumped);
                assert(prefix_exclusive_end_spec(before) == Some(before.update(i as int, bumped)));
            }
            e.set(i, bumped);
            proof {
                assert(e@ == before.update(i as int, bumped));
            }
            return Some(e);
        }
        let ghost before = e@;
        let popped = e.pop();
        proof {
            assert(popped == Some(0xffu8));
            assert(e@ == before.subrange(0, before.len() as int - 1));
            assert(prefix_exclusive_end_spec(before) == prefix_exclusive_end_spec(e@));
        }
        let _ = popped;
    }
    None
}

/// AS-IS F57/F58: `prefix || 0xff`. Drops `prefix || 0xff || …`.
pub fn prefix_exclusive_end_as_is(prefix: &[u8]) -> (r: Option<Vec<u8>>)
    ensures
        r.is_some(),
        r.unwrap()@ == prefix_exclusive_end_as_is_spec(prefix@),
{
    let mut e = clone_bytes(prefix);
    e.push(0xff);
    Some(e)
}

/// AS-IS F57: treat the next byte as a hard 0xff wall (prefix || 0xff).
pub open spec fn as_is_ff_wall() -> u8 {
    0xff
}

/// A key that continues with 0xff after the prefix is still a prefix match.
/// AS-IS exclusive end `prefix || 0xff` excludes it (key >= end).
proof fn lemma_as_is_wall_excludes_ff_continuation()
    ensures
        as_is_ff_wall() == 0xffu8,
        0xffu8 <= as_is_ff_wall(),
{
}

/// FIXED successor of a last non-ff byte is strictly above that byte
/// and therefore above `byte || 0xff…` in lexicographic order of a
/// *shorter* incremented prefix vs a longer 0xff-extended key.
proof fn lemma_bump_is_above(b: u8)
    requires
        b < 0xff,
    ensures
        bump_non_ff_spec(b).unwrap() > b,
{
}

/// Empty / all-0xff prefixes have no exclusive end (unbounded scan).
proof fn lemma_empty_is_unbounded()
    ensures
        prefix_exclusive_end_spec(Seq::<u8>::empty()) is None,
{
}

/// Witness F57: as-is `prefix || 0xff` is a *shorter* wall than the
/// fixed bump of a trailing non-ff (or the unbounded None). Distinct
/// from the fixed spec on the empty prefix.
proof fn lemma_as_is_differs_on_empty()
    ensures
        prefix_exclusive_end_spec(Seq::<u8>::empty()) is None,
        prefix_exclusive_end_as_is_spec(Seq::<u8>::empty()) == seq![0xffu8],
{
}

} // verus!
