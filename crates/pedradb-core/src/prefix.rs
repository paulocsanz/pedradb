//! Exclusive end of a prefix scan (F57 / F58).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_prefix_exclusive_end.sh
//!
//! Increment the last non-`0xff` byte. `None` = unbounded (empty or all-`0xff`).
//! Store, fold, and SQL must call **this** function — not `prefix || [0xff]`.

#![forbid(unsafe_code)]

/// Next key after every key that starts with `prefix` (exclusive end).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn prefix_exclusive_end(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut e = prefix.to_vec();
    while e.len() > 0 {
        let i = e.len() - 1;
        if e[i] < 0xff {
            e[i] += 1;
            return Some(e);
        }
        e.pop();
    }
    None
}

/// AS-IS F57/F58: `prefix || 0xff`. Drops `prefix || 0xff || …`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn prefix_exclusive_end_as_is(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut e = prefix.to_vec();
    e.push(0xff);
    Some(e)
}

/// Whether `key` is in `[prefix, end)` (bytewise). `end = None` is unbounded.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn key_in_prefix_range(key: &[u8], prefix: &[u8], end: Option<&[u8]>) -> bool {
    if !key.starts_with(prefix) {
        return false;
    }
    match end {
        None => true,
        Some(e) => key < e,
    }
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn bump_non_ff_spec(b: u8) -> Option<u8> {
    if b < 0xff {
        Some((b + 1) as u8)
    } else {
        None
    }
}

fn bump_non_ff(b: u8) -> (r: Option<u8>)
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

pub fn prefix_exclusive_end_as_is(prefix: &[u8]) -> (r: Option<Vec<u8>>)
    ensures
        r.is_some(),
        r.unwrap()@ == prefix_exclusive_end_as_is_spec(prefix@),
{
    let mut e = clone_bytes(prefix);
    e.push(0xff);
    Some(e)
}

pub open spec fn as_is_ff_wall_spec() -> u8 {
    0xff
}

proof fn lemma_as_is_wall_excludes_ff_continuation()
    ensures
        as_is_ff_wall_spec() == 0xffu8,
        0xffu8 <= as_is_ff_wall_spec(),
{
}

proof fn lemma_bump_is_above(b: u8)
    requires
        b < 0xff,
    ensures
        bump_non_ff_spec(b).unwrap() > b,
{
}

proof fn lemma_empty_is_unbounded()
    ensures
        prefix_exclusive_end_spec(Seq::<u8>::empty()) is None,
{
}

proof fn lemma_as_is_differs_on_empty()
    ensures
        prefix_exclusive_end_spec(Seq::<u8>::empty()) is None,
        prefix_exclusive_end_as_is_spec(Seq::<u8>::empty()) == seq![0xffu8],
{
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_prefix_increments_last() {
        assert_eq!(prefix_exclusive_end(b"ab"), Some(b"ac".to_vec()));
    }

    #[test]
    fn trailing_ff_carries() {
        assert_eq!(prefix_exclusive_end(&[0x01, 0xff]), Some(vec![0x02]));
    }

    #[test]
    fn all_ff_is_unbounded() {
        assert_eq!(prefix_exclusive_end(&[0xff, 0xff]), None);
        assert_eq!(prefix_exclusive_end(b""), None);
    }

    #[test]
    fn key_in_prefix_range_on_live_last_is_not_ok() {
        assert!(key_in_prefix_range(b"ab", b"a", Some(b"b".as_ref())));
        assert!(!key_in_prefix_range(b"b", b"a", Some(b"b".as_ref())));
        assert!(key_in_prefix_range(b"z", b"", None));
        let src = include_str!("db.rs");
        let last = src
            .split("pub fn last_under_prefix")
            .nth(1)
            .and_then(|s| s.split("pub fn last_under_user_prefix").next())
            .expect("last_under_prefix");
        assert!(
            last.contains("key_in_prefix_range("),
            "last_under_prefix must match key_in_prefix_range"
        );
        assert!(
            !last.contains("!k.starts_with(prefix)"),
            "last_under_prefix must not keep a raw starts_with skip"
        );
    }

    #[test]
    fn as_is_drops_ff_suffix() {
        let p = b"/host/h1/";
        let mut key = p.to_vec();
        key.push(0xff);
        key.extend_from_slice(b"z");
        let fixed = prefix_exclusive_end(p);
        let as_is = prefix_exclusive_end_as_is(p);
        assert!(
            key_in_prefix_range(&key, p, fixed.as_deref()),
            "FIXED must include {key:?} end={fixed:?}"
        );
        assert!(
            !key_in_prefix_range(&key, p, as_is.as_deref()),
            "AS-IS must drop {key:?} end={as_is:?}"
        );
    }

    #[test]
    fn empty_as_is_drops_ff_keys() {
        let key = [0xff, b'z'];
        assert!(key_in_prefix_range(
            &key,
            b"",
            prefix_exclusive_end(b"").as_deref()
        ));
        assert!(!key_in_prefix_range(
            &key,
            b"",
            prefix_exclusive_end_as_is(b"").as_deref()
        ));
    }

    #[test]
    fn theorem_prefix_end_on_short_alphabet() {
        const A: [u8; 4] = [0, 1, 0xfe, 0xff];
        let mut n = 0u32;
        // prefixes of length 0..=2
        let mut prefixes: Vec<Vec<u8>> = vec![vec![]];
        for &a in &A {
            prefixes.push(vec![a]);
            for &b in &A {
                prefixes.push(vec![a, b]);
            }
        }
        for p in &prefixes {
            let end = prefix_exclusive_end(p);
            for key in &prefixes {
                if !key.starts_with(p) {
                    continue;
                }
                assert!(
                    key_in_prefix_range(key, p, end.as_deref()),
                    "FIXED dropped prefix-match p={p:?} key={key:?} end={end:?}"
                );
                n += 1;
            }
            // F57 witness when we can append 0xff
            let mut wit = p.clone();
            wit.push(0xff);
            wit.push(1);
            assert!(
                key_in_prefix_range(&wit, p, end.as_deref()),
                "FIXED must keep p||0xff||1 p={p:?} end={end:?}"
            );
            let as_is = prefix_exclusive_end_as_is(p);
            assert!(
                !key_in_prefix_range(&wit, p, as_is.as_deref()),
                "AS-IS must drop p||0xff||1 p={p:?}"
            );
        }
        assert!(n > 0);
    }
}
