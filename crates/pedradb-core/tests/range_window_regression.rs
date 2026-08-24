use pedradb_core::Db;

/// F220 regression: `range_limited` over a window whose bounds carry no NUL
/// (`["d/m/", "d/m0")`) must still return keys that DO carry one
/// (`d/m/\0\0\0\x0a/ha/leader` — the DCS on-disk shape). The tail-index
/// shard gate used to pick the empty shard for such bounds and the scan
/// silently returned less than `get` saw.
#[test]
fn range_window_dcs_full_shape_regression() {
    let dir = std::env::temp_dir().join(format!("range-reg3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    {
        let mut db = Db::open(&dir).unwrap();
        db.put(b"d/rev", &[1u8]).unwrap();
        let mut tx = db.begin();
        tx.put(b"d/kv/\x00\x00\x00\x0a/ha/leader", b"node-a")
            .unwrap();
        tx.put(b"d/m/\x00\x00\x00\x0a/ha/leader", b"meta-lease-1")
            .unwrap();
        tx.commit().unwrap();
        db.close().unwrap();
    }
    let db = Db::open(&dir).unwrap();
    let win = db.range_limited(
        std::ops::Bound::Included(b"d/m/".as_slice()),
        std::ops::Bound::Excluded(b"d/m0".as_slice()),
        None,
    );
    let win_n = win.iter().count();
    let got = db.get(b"d/m/\x00\x00\x00\x0a/ha/leader");
    assert!(win_n == 1, "window={win_n} but get={got:?}");
}

/// Plain window (no embedded NULs) — the shape the fast path was built for.
#[test]
fn range_window_plain_keys_regression() {
    let dir = std::env::temp_dir().join(format!("range-reg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    {
        let mut db = Db::open(&dir).unwrap();
        db.put(b"d/m/ha/leader", b"meta-with-lease-1").unwrap();
        db.put(b"other/key", b"v").unwrap();
        db.close().unwrap();
    }
    let db = Db::open(&dir).unwrap();
    let got = db.range_limited(
        std::ops::Bound::Included(b"d/m/".as_slice()),
        std::ops::Bound::Excluded(b"d/m0".as_slice()),
        None,
    );
    let keys: Vec<Vec<u8>> = got.iter().map(|(k, _)| k.to_vec()).collect();
    assert_eq!(keys, vec![b"d/m/ha/leader".to_vec()], "lost: {keys:?}");
}

/// Embedded-NUL key written through a plain put (no tx) — covers the
/// non-tx tail path of the same shard gate.
#[test]
fn range_window_embedded_nul_put_regression() {
    let dir = std::env::temp_dir().join(format!("range-reg2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    {
        let mut db = Db::open(&dir).unwrap();
        db.put(b"d/m/\x00\x00\x00\x0a/ha/leader", b"meta").unwrap();
        db.put(b"d/kv/\x00\x00\x00\x0a/ha/leader", b"val").unwrap();
        db.close().unwrap();
    }
    let db = Db::open(&dir).unwrap();
    let win = db.range_limited(
        std::ops::Bound::Included(b"d/m/".as_slice()),
        std::ops::Bound::Excluded(b"d/m0".as_slice()),
        None,
    );
    let win_n = win.iter().count();
    assert_eq!(win_n, 1, "window lost the embedded-NUL key ({win_n})");
}
