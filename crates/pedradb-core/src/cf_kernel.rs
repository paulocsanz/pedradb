//! Pure column-family membership / encode (RFC-0150 P0).
//!
//! Compat stores `cf\\0user`. Kernel keys without a NUL share the `default`
//! family. A scan or compact of `default` that treats `lock\\0…` as in-family
//! is the CF-leak silent-wrong (foreign keys in a CF iterator / compact of
//! lock rewriting default SSTs).
//!
//! Production flush/compact/compat call these helpers. Bytes on disk stay
//! with the caller.
//!
//! Verus twin: `crates/pedradb-core/verus/cf_family.rs`.

#![forbid(unsafe_code)]

/// Family of a user key: bytes before the first `0x00`, or `"default"` when
/// the key has no NUL (kernel / default-raw) or a leading NUL.
#[must_use]
pub fn cf_family_of(user_key: &[u8]) -> String {
    match user_key.iter().position(|&b| b == 0) {
        Some(i) if i > 0 => String::from_utf8_lossy(&user_key[..i]).into_owned(),
        _ => "default".into(),
    }
}

/// Whether `user_key` belongs to `family` (`"default"` matches both raw keys
/// and the `default\0…` prefix).
#[must_use]
pub fn key_in_cf_family(user_key: &[u8], family: &str) -> bool {
    if family == "default" {
        match user_key.iter().position(|&b| b == 0) {
            None | Some(0) => true,
            Some(i) => &user_key[..i] == b"default",
        }
    } else {
        let p = family.as_bytes();
        user_key.len() > p.len() && user_key.starts_with(p) && user_key[p.len()] == 0
    }
}

/// AS-IS scan leak: every key is in-family (a `lock\0…` key scans as `default`).
#[must_use]
pub fn key_in_cf_family_as_is(_user_key: &[u8], _family: &str) -> bool {
    true
}

/// Effective CF prefix bytes: empty when `default` is stored raw.
#[must_use]
pub fn cf_encode_effective<'a>(cf: &'a str, default_raw: bool) -> &'a str {
    if cf == "default" && default_raw {
        ""
    } else {
        cf
    }
}

/// Encode `key` for `cf` (`cf\0key`, or raw when default-raw).
#[must_use]
pub fn encode_cf_key(cf: &str, key: &[u8], default_raw: bool) -> Vec<u8> {
    let effective = cf_encode_effective(cf, default_raw);
    if effective.is_empty() {
        return key.to_vec();
    }
    let mut out = Vec::with_capacity(effective.len() + 1 + key.len());
    out.extend_from_slice(effective.as_bytes());
    out.push(0);
    out.extend_from_slice(key);
    out
}

/// Inverse of [`encode_cf_key`]: strip the `cf\0` prefix, or return `encoded`
/// unchanged when default-raw.
#[must_use]
pub fn decode_cf_key<'a>(cf: &str, encoded: &'a [u8], default_raw: bool) -> &'a [u8] {
    let effective = cf_encode_effective(cf, default_raw);
    if effective.is_empty() {
        return encoded;
    }
    encoded.get(effective.len() + 1..).unwrap_or(&[])
}

/// SST CF tag from key bounds. Empty = mixed / prefix-era (more than one family).
#[must_use]
pub fn infer_sst_cf(smallest: Option<&[u8]>, largest: Option<&[u8]>) -> String {
    match (smallest, largest) {
        (Some(s), Some(l)) => {
            let a = cf_family_of(s);
            let b = cf_family_of(l);
            if a == b {
                a
            } else {
                String::new()
            }
        }
        (Some(s), None) | (None, Some(s)) => cf_family_of(s),
        (None, None) => String::new(),
    }
}

/// Whether compact of `family` rewrites an SST tagged `sst_cf`.
///
/// Mixed/legacy files (empty tag) are left alone. A tagged file is rewritten
/// only when a representative encoded key of that tag is in-family.
#[must_use]
pub fn compact_rewrites_sst_cf(sst_cf: &str, family: &str) -> bool {
    if sst_cf.is_empty() {
        return false;
    }
    key_in_cf_family(&encode_cf_key(sst_cf, &[], false), family)
}

/// AS-IS compact leak: rewrite every SST (lock compact walks default).
#[must_use]
pub fn compact_rewrites_sst_cf_as_is(_sst_cf: &str, _family: &str) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_raw_and_prefixed() {
        assert!(key_in_cf_family(b"aaa", "default"));
        assert!(key_in_cf_family(b"default\0k", "default"));
        assert!(!key_in_cf_family(b"lock\0k", "default"));
        assert!(key_in_cf_family(b"lock\0k", "lock"));
        assert_eq!(cf_family_of(b"aaa"), "default");
        assert_eq!(cf_family_of(b"default\0k"), "default");
        assert_eq!(cf_family_of(b"lock\0k"), "lock");
    }

    #[test]
    fn as_is_admits_scan_leak() {
        assert!(
            key_in_cf_family_as_is(b"lock\0k", "default"),
            "AS-IS dente: foreign-CF key treated as in-family"
        );
        assert!(
            !key_in_cf_family(b"lock\0k", "default"),
            "fixed kernel refuses the leak"
        );
    }

    #[test]
    fn key_in_cf_family_on_live_scan_is_not_ok() {
        let lock = encode_cf_key("lock", b"k", false);
        assert!(
            !key_in_cf_family(&lock, "default"),
            "live lock key must not scan as default"
        );
        assert!(
            key_in_cf_family_as_is(&lock, "default"),
            "AS-IS dente: scan leak"
        );
        let mut db = crate::Db::open(
            std::env::temp_dir().join(format!("pedra-cf-plant-{}", std::process::id())),
        )
        .unwrap();
        db.set_physical_cfs(vec!["default".into(), "lock".into()]);
        db.put(b"lock\0k", b"L").unwrap();
        db.put(b"default\0d", b"D").unwrap();
        let got = db.get(b"default\0d");
        assert_eq!(got.as_deref(), Some(b"D".as_ref()));
        assert!(!key_in_cf_family(b"lock\0k", "default"));
        let _ = db.close();
        let _ = std::fs::remove_dir_all(
            std::env::temp_dir().join(format!("pedra-cf-plant-{}", std::process::id())),
        );
    }

    #[test]
    fn encode_decode_roundtrip_named_and_default_raw() {
        for default_raw in [false, true] {
            for (cf, key) in [
                ("default", b"k".as_slice()),
                ("lock", b"lk".as_slice()),
                ("write", b"".as_slice()),
                ("write", &[0u8, 1, 2][..]),
            ] {
                let enc = encode_cf_key(cf, key, default_raw);
                assert_eq!(
                    decode_cf_key(cf, &enc, default_raw),
                    key,
                    "roundtrip cf={cf} default_raw={default_raw}"
                );
                assert!(
                    key_in_cf_family(&enc, cf),
                    "encoded key must be in-family cf={cf} default_raw={default_raw} enc={enc:?}"
                );
            }
        }
    }

    #[test]
    fn compact_lock_does_not_rewrite_default_tag() {
        assert!(compact_rewrites_sst_cf("lock", "lock"));
        assert!(!compact_rewrites_sst_cf("default", "lock"));
        assert!(!compact_rewrites_sst_cf("", "lock"));
        assert!(
            compact_rewrites_sst_cf_as_is("default", "lock"),
            "AS-IS dente: lock compact rewrites default"
        );
    }

    #[test]
    fn infer_mixed_bounds_empty_tag() {
        assert_eq!(infer_sst_cf(Some(b"lock\0a"), Some(b"lock\0z")), "lock");
        assert_eq!(infer_sst_cf(Some(b"aaa"), Some(b"lock\0z")), "");
        assert_eq!(infer_sst_cf(Some(b"aaa"), Some(b"zzz")), "default");
    }
}
