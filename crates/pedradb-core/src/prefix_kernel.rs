//! Exclusive end of a prefix scan (F57 / F58).
//!
//! **Term:** this file is what `rustc` links. Aeneas extracts that body
//! (`scripts/aeneas_prefix.sh`). A Seq / clone_bytes view of the rustc
//! `&[u8]` `to_vec` body is a model twin — not last-wins (deleted).
//!
//!   ./scripts/aeneas_prefix.sh --required
//!
//! Increment the last non-`0xff` byte. `None` = unbounded (empty or all-`0xff`).
//! Store, fold, and SQL must call **this** function — not `prefix || [0xff]`.

#![forbid(unsafe_code)]

/// Next key after every key that starts with `prefix` (exclusive end).
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
#[must_use]
pub fn prefix_exclusive_end_as_is(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut e = prefix.to_vec();
    e.push(0xff);
    Some(e)
}

/// Whether `key` is in `[prefix, end)` (bytewise). `end = None` is unbounded.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_rs_has_no_verus_cartoon() {
        let src = include_str!("prefix_kernel.rs");
        let block = concat!("verus", "!", " {");
        let cfg = concat!("cfg(", "verus", "_keep", "_ghost)");
        assert!(
            !src.contains(block),
            "Seq/clone_bytes stand-in is not last-wins of rustc &[u8]"
        );
        assert!(
            !src.contains(cfg),
            "cfg split hides rustc types from the prover"
        );
    }

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
        let src = include_str!("db_kernel.rs");
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
