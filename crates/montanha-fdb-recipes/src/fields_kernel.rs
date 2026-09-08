//! Length-prefixed fields (RFC-0002 P24 / F60).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). Vec encode/decode is caller. No twin-cópia.
//!
//!   ./scripts/verus_fields_nul.sh
//!
//! Production payload write/read and [`crate::Subspace::child_suffix`] call
//! these. Do **not** split a field on raw `0x00`.
//!
//! AS-IS unpack: suffix after the *last* `0x00` (id `[a,0,b]` → `[b]`).
//! AS-IS payload: `a || 0x00 || b` (zip with NUL truncates; stale index).
//! Pack injectivity (`0x00 || part` collision) is F62.

#![forbid(unsafe_code)]

macro_rules! field_kept_body {
    ($len:expr, $nul_at:expr) => {{
        let _ = $nul_at;
        $len
    }};
}

macro_rules! field_kept_as_is_body {
    ($len:expr, $nul_at:expr) => {{
        let _ = $len;
        $nul_at
    }};
}

/// Bytes of a field of `len` that contains a NUL at `nul_at` (FIXED: all of it).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn field_kept(len: u64, nul_at: u64) -> u64 {
    field_kept_body!(len, nul_at)
}

/// AS-IS F60: first/last NUL is the delimiter — keep only `[0, nul_at)`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn field_kept_as_is(len: u64, nul_at: u64) -> u64 {
    field_kept_as_is_body!(len, nul_at)
}

/// Child payload after `start = pack || 0x00` (full suffix, NULs inside stay).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn child_bytes_after<'a>(key: &'a [u8], start: &[u8]) -> Option<&'a [u8]> {
    key.strip_prefix(start)
}

/// AS-IS F60: last `0x00` in the whole key is the delimiter.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn child_bytes_after_as_is<'a>(key: &'a [u8], _start: &[u8]) -> Option<&'a [u8]> {
    Some(key.rsplit(|b| *b == 0).next().unwrap_or(key))
}

/// Length-prefixed field join (payload write).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn encode_fields(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in parts {
        let n = u32::try_from(p.len()).expect("field len fits u32");
        out.extend_from_slice(&n.to_be_bytes());
        out.extend_from_slice(p);
    }
    out
}

/// AS-IS F60: join with raw `0x00`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn encode_fields_as_is(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.push(0);
        }
        out.extend_from_slice(p);
    }
    out
}

/// Decode [`encode_fields`] into `n` fields. `None` on short/corrupt input.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn decode_fields(raw: &[u8], n: usize) -> Option<Vec<Vec<u8>>> {
    let mut off = 0usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        if raw.len() < off + 4 {
            return None;
        }
        let len = u32::from_be_bytes(raw[off..off + 4].try_into().ok()?) as usize;
        off += 4;
        if raw.len() < off + len {
            return None;
        }
        out.push(raw[off..off + len].to_vec());
        off += len;
    }
    if off != raw.len() {
        return None;
    }
    Some(out)
}

/// AS-IS F60: first `0x00` splits a pair (NUL inside the first field truncates).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn decode_pair_first_nul(raw: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let sep = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let a = raw[..sep].to_vec();
    let b = if sep < raw.len() {
        raw[sep + 1..].to_vec()
    } else {
        Vec::new()
    };
    (a, b)
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn field_kept_spec(len: u64, _nul_at: u64) -> u64 {
    len
}

pub fn field_kept(len: u64, nul_at: u64) -> (k: u64)
    ensures
        k == field_kept_spec(len, nul_at),
        k == len,
{
    field_kept_body!(len, nul_at)
}

pub open spec fn field_kept_as_is_spec(_len: u64, nul_at: u64) -> u64 {
    nul_at
}

pub fn field_kept_as_is(len: u64, nul_at: u64) -> (k: u64)
    ensures
        k == field_kept_as_is_spec(len, nul_at),
        k == nul_at,
{
    field_kept_as_is_body!(len, nul_at)
}

proof fn lemma_as_is_truncates(len: u64, nul_at: u64)
    requires
        nul_at < len,
    ensures
        field_kept_as_is_spec(len, nul_at) < field_kept_spec(len, nul_at),
{
}

proof fn lemma_no_nul_same(len: u64)
    ensures
        field_kept_spec(len, len) == field_kept_as_is_spec(len, len),
{
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpack_keeps_nul_inside_id() {
        let start = b"zip\x0090\x00";
        let id = [b'a', 0x00, b'b'];
        let mut key = start.to_vec();
        key.extend_from_slice(&id);
        assert_eq!(child_bytes_after(&key, start), Some(id.as_slice()));
        assert_eq!(
            child_bytes_after_as_is(&key, start),
            Some(&[b'b'][..]),
            "AS-IS rsplit must truncate to last segment"
        );
    }

    #[test]
    fn payload_round_trips_nul_zip() {
        let zip = [b'9', 0x00, b'0'];
        let raw = encode_fields(&[&zip, b"alice"]);
        let got = decode_fields(&raw, 2).unwrap();
        assert_eq!(got[0], zip);
        assert_eq!(got[1], b"alice");
        let as_is = encode_fields_as_is(&[&zip, b"alice"]);
        let (a, _) = decode_pair_first_nul(&as_is);
        assert_eq!(a, b"9");
        assert_ne!(a.as_slice(), zip);
    }

    #[test]
    fn field_kept_not_nul_offset() {
        assert_eq!(field_kept(3, 1), 3);
        assert_eq!(field_kept_as_is(3, 1), 1);
        assert_eq!(field_kept(3, 3), field_kept_as_is(3, 3));
    }

    #[test]
    fn theorem_on_small_domain() {
        let mut n = 0u32;
        for len in 0u64..8 {
            for nul_at in 0..=len {
                let k = field_kept(len, nul_at);
                let a = field_kept_as_is(len, nul_at);
                assert_eq!(k, len);
                assert_eq!(a, nul_at);
                if nul_at < len {
                    assert!(a < k);
                }
                n += 1;
            }
        }
        assert_eq!(n, 36);
    }
}
