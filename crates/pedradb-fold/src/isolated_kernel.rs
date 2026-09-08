//! Isolated id match (RFC-0002 P29 / F83).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_isolated_id.sh
//!
//! Production [`crate::in_prefixes`] / [`PrefixSet::push_isolated`] call this.
//! Raw `starts_with(id)` matches sibling ids (`/vm/vm-a` ⊃ `/vm/vm-ab`).

#![forbid(unsafe_code)]

/// Child separator after an isolated id (`/vm/vm-a/disk`).
pub const ISOLATED_CHILD_SEP: u8 = b'/';

/// `key` is this id, or a path child (`id || '/' || rest`).
///
/// Byte loop, not `starts_with` / `==` on slices: those extract to Aeneas
/// axioms. Indexing after a length check is in the Lean std.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn isolated_id_matches(key: &[u8], id: &[u8]) -> bool {
    if key.len() < id.len() {
        return false;
    }
    let mut i = 0;
    while i < id.len() {
        if key[i] != id[i] {
            return false;
        }
        i += 1;
    }
    key.len() == id.len() || key[i] == ISOLATED_CHILD_SEP
}

/// AS-IS F83: any prefix match — `/vm/vm-a` matches `/vm/vm-ab`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn isolated_id_matches_as_is(key: &[u8], id: &[u8]) -> bool {
    if key.len() < id.len() {
        return false;
    }
    let mut i = 0;
    while i < id.len() {
        if key[i] != id[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// After an exact id, a continuation byte is a child iff it is `'/'`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn isolated_child_byte(next: u8) -> bool {
    next == ISOLATED_CHILD_SEP
}

/// AS-IS: any next byte continues the prefix.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn isolated_child_byte_as_is(_next: u8) -> bool {
    true
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn isolated_child_sep() -> u8 {
    0x2f
}

pub open spec fn isolated_child_byte_spec(next: u8) -> bool {
    next == isolated_child_sep()
}

pub fn isolated_child_byte(next: u8) -> (d: bool)
    ensures
        d == isolated_child_byte_spec(next),
        d == (next == 0x2f),
{
    next == 0x2f
}

pub open spec fn isolated_child_byte_as_is_spec(_next: u8) -> bool {
    true
}

pub fn isolated_child_byte_as_is(_next: u8) -> (d: bool)
    ensures
        d == true,
        d == isolated_child_byte_as_is_spec(_next),
{
    true
}

proof fn lemma_slash_is_child()
    ensures
        isolated_child_byte_spec(0x2f),
{
}

proof fn lemma_as_is_leaks_sibling_b()
    ensures
        !isolated_child_byte_spec(0x62),
        isolated_child_byte_as_is_spec(0x62),
{
}

pub open spec fn isolated_id_matches_spec(key: Seq<u8>, id: Seq<u8>) -> bool {
    id.len() <= key.len()
        && (forall |j: int| 0 <= j && j < id.len() ==> key[j] == id[j])
        && (key.len() == id.len() || key[id.len() as int] == isolated_child_sep())
}

pub open spec fn isolated_id_matches_as_is_spec(key: Seq<u8>, id: Seq<u8>) -> bool {
    id.len() <= key.len()
        && (forall |j: int| 0 <= j && j < id.len() ==> key[j] == id[j])
}

pub fn isolated_id_matches(key: &[u8], id: &[u8]) -> (d: bool)
    ensures
        d == isolated_id_matches_spec(key@, id@),
{
    if key.len() < id.len() {
        assert(!(id.len() <= key.len()));
        return false;
    }
    let mut i: usize = 0;
    while i < id.len()
        invariant
            i <= id.len(),
            id.len() <= key.len(),
            forall |j: int| 0 <= j && j < i ==> key@[j] == id@[j],
        decreases id.len() - i,
    {
        if key[i] != id[i] {
            proof {
                assert(key@[i as int] != id@[i as int]);
                assert(!isolated_id_matches_spec(key@, id@));
            }
            return false;
        }
        i = i + 1;
    }
    key.len() == id.len() || key[i] == 0x2f
}

pub fn isolated_id_matches_as_is(key: &[u8], id: &[u8]) -> (d: bool)
    ensures
        d == isolated_id_matches_as_is_spec(key@, id@),
{
    if key.len() < id.len() {
        return false;
    }
    let mut i: usize = 0;
    while i < id.len()
        invariant
            i <= id.len(),
            id.len() <= key.len(),
            forall |j: int| 0 <= j && j < i ==> key@[j] == id@[j],
        decreases id.len() - i,
    {
        if key[i] != id[i] {
            return false;
        }
        i = i + 1;
    }
    true
}

proof fn lemma_as_is_leaks_sibling_key()
    ensures
        !isolated_id_matches_spec(seq![0x61u8, 0x62u8], seq![0x61u8]),
        isolated_id_matches_as_is_spec(seq![0x61u8, 0x62u8], seq![0x61u8]),
{
    assert(seq![0x61u8, 0x62u8][1] != isolated_child_sep());
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_and_child_stay() {
        assert!(isolated_id_matches(b"/vm/vm-a", b"/vm/vm-a"));
        assert!(isolated_id_matches(b"/vm/vm-a/disk", b"/vm/vm-a"));
        assert!(!isolated_id_matches(b"/vm/vm-ab", b"/vm/vm-a"));
        assert!(!isolated_id_matches(b"/vm/vm-a2", b"/vm/vm-a"));
    }

    #[test]
    fn as_is_leaks_sibling() {
        assert!(isolated_id_matches_as_is(b"/vm/vm-ab", b"/vm/vm-a"));
        assert!(isolated_id_matches_as_is(b"/vm/vm-a2", b"/vm/vm-a"));
        assert_ne!(
            isolated_id_matches(b"/vm/vm-ab", b"/vm/vm-a"),
            isolated_id_matches_as_is(b"/vm/vm-ab", b"/vm/vm-a")
        );
    }

    #[test]
    fn theorem_next_byte_domain() {
        let mut n = 0u32;
        for next in 0u8..=255 {
            let d = isolated_child_byte(next);
            assert_eq!(d, next == b'/');
            assert!(isolated_child_byte_as_is(next));
            if next != b'/' {
                assert!(!d);
            }
            n += 1;
        }
        assert_eq!(n, 256);
    }

    /// Catalog three-teeth plant. Direct `as_is_leaks_sibling` is **not** this tooth.
    #[test]
    fn isolated_id_matches_on_live_fold_is_not_ok() {
        assert!(!isolated_id_matches(b"/vm/vm-ab", b"/vm/vm-a"));
        assert!(
            isolated_id_matches_as_is(b"/vm/vm-ab", b"/vm/vm-a"),
            "AS-IS dente: starts_with leaks sibling /vm/vm-ab"
        );
        let mut set = crate::PrefixSet::new();
        set.push_isolated(b"/vm/vm-a");
        assert!(crate::in_prefixes(b"/vm/vm-a", &set));
        assert!(crate::in_prefixes(b"/vm/vm-a/disk", &set));
        assert!(
            !crate::in_prefixes(b"/vm/vm-ab", &set),
            "live in_prefixes must not treat vm-ab as vm-a"
        );
    }
}
