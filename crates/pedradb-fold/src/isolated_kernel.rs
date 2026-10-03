//! Isolated id match (RFC-0002 P29 / F83).
//!
//! **Term:** this file is what `rustc` links. Aeneas extracts that body
//! (`scripts/aeneas_isolated.sh`). A Seq view of the rustc `&[u8]` index
//! loop is a model twin — not last-wins (deleted).
//!
//!   ./scripts/aeneas_isolated.sh --required
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
#[must_use]
pub fn isolated_child_byte(next: u8) -> bool {
    next == ISOLATED_CHILD_SEP
}

/// AS-IS: any next byte continues the prefix.
#[must_use]
pub fn isolated_child_byte_as_is(_next: u8) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_kernel_has_no_verus_cartoon() {
        let src = include_str!("isolated_kernel.rs");
        let block = concat!("verus", "!", " {");
        let cfg = concat!("cfg(", "verus", "_keep", "_ghost)");
        assert!(
            !src.contains(block),
            "Seq stand-in is not last-wins of rustc &[u8] isolated-id loop"
        );
        assert!(
            !src.contains(cfg),
            "cfg split hides rustc types from the prover"
        );
    }

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
