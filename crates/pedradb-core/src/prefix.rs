//! Exclusive end of a prefix scan (F57 / F58).
//!
//! Increment the last non-`0xff` byte. `None` = unbounded (empty or all-`0xff`).
//! Store, fold, and SQL must call **this** function — not `prefix || [0xff]`.

#![forbid(unsafe_code)]

/// Next key after every key that starts with `prefix` (exclusive end).
#[must_use]
pub fn prefix_exclusive_end(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut e = prefix.to_vec();
    while let Some(last) = e.last_mut() {
        if *last < 0xff {
            *last += 1;
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
