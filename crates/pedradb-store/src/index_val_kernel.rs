//! Exact index-value range (RFC-0002 P26 / F80).
//!
//! **Single artifact (Aeneas-paid):** this file is what `rustc` links and
//! what the Lean theorems run over — Charon+Aeneas extract of these exact
//! bodies. No Verus twin stands in for them.
//!
//!   ./scripts/aeneas_index_val.sh
//!
//! Production [`crate::layers::table_index_value_range`] and
//! [`crate::fdb_layers::IdempotentIndex`] call these.
//! Raw `[val||0x00, val||0x01)` includes `val||0x00||foo` (F78 / F80).

#![forbid(unsafe_code)]

macro_rules! value_len_tag_body {
    ($len:expr) => {
        $len
    };
}

macro_rules! value_len_tag_as_is_body {
    ($len:expr) => {{
        let _ = $len;
        0u32
    }};
}

/// Length tag of an index value (FIXED records `|val|`).
#[must_use]
pub fn value_len_tag(len: u32) -> u32 {
    value_len_tag_body!(len)
}

/// AS-IS F80: no length tag — `red` is a byte prefix of `red\\0foo`.
#[must_use]
pub fn value_len_tag_as_is(len: u32) -> u32 {
    value_len_tag_as_is_body!(len)
}

/// `u32be(|val|) || val`.
#[must_use]
pub fn len_pref_value(val: &[u8]) -> Vec<u8> {
    let mut k = Vec::with_capacity(4 + val.len());
    let n = u32::try_from(val.len()).expect("value len fits u32");
    k.extend_from_slice(&n.to_be_bytes());
    k.extend_from_slice(val);
    k
}

/// AS-IS F80: raw `val` (no length).
#[must_use]
pub fn len_pref_value_as_is(val: &[u8]) -> Vec<u8> {
    val.to_vec()
}

/// Children of an exact-value prefix: `[p||0x00, p||0x01)`.
#[must_use]
pub fn exact_value_children(prefix: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut start = prefix.to_vec();
    start.push(0x00);
    let mut end = prefix.to_vec();
    end.push(0x01);
    (start, end)
}

/// AS-IS F80 range: `[val||0x00, val||0x01)`.
#[must_use]
pub fn exact_value_children_as_is(val: &[u8]) -> (Vec<u8>, Vec<u8>) {
    exact_value_children(&len_pref_value_as_is(val))
}



#[cfg(test)]
fn in_range(key: &[u8], start: &[u8], end: &[u8]) -> bool {
    key >= start && key < end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_is_leaks_nul_sibling() {
        let red = b"red";
        let mut sib = red.to_vec();
        sib.push(0x00);
        sib.extend_from_slice(b"foo");
        let (s_as, e_as) = exact_value_children_as_is(red);
        let mut sib_key = sib.clone();
        sib_key.push(0x00);
        assert!(
            in_range(&sib_key, &s_as, &e_as),
            "AS-IS [val||0x00, val||0x01) must include red\\0foo"
        );
        let (s, e) = exact_value_children(&len_pref_value(red));
        let mut sib_fixed = len_pref_value(&sib);
        sib_fixed.push(0x00);
        assert!(
            !in_range(&sib_fixed, &s, &e),
            "FIXED length-prefix must drop red\\0foo"
        );
        let mut red_key = len_pref_value(red);
        red_key.push(0x00);
        red_key.push(b'1');
        assert!(in_range(&red_key, &s, &e));
    }

    #[test]
    fn len_tag_is_injective() {
        assert_eq!(value_len_tag(3), 3);
        assert_eq!(value_len_tag_as_is(3), 0);
        assert_ne!(value_len_tag(3), value_len_tag(7));
        assert_eq!(value_len_tag_as_is(3), value_len_tag_as_is(7));
    }

    #[test]
    fn theorem_on_small_domain() {
        let mut n = 0u32;
        for a in 0u32..8 {
            for b in 0u32..8 {
                assert_eq!(value_len_tag(a), a);
                assert_eq!(value_len_tag_as_is(a), 0);
                if a != b {
                    assert_ne!(value_len_tag(a), value_len_tag(b));
                    assert_eq!(value_len_tag_as_is(a), value_len_tag_as_is(b));
                }
                n += 1;
            }
        }
        assert_eq!(n, 64);
    }
}
