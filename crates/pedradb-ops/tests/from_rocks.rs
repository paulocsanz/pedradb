//! RFC-0186 P0.3 + P1 + P2.1: live Rocks snapshot → Pedra MANIFEST v5.
//!
//! Requires `--features from-rocks` (C++ toolchain). Default CI skips this.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pedradb_core::{encode_cf_key, db::Db};
use pedradb_ops::{classify_dir, inspect_format, migrate_from_rocks, DirKind};

fn temp(name: &str) -> std::path::PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let i = N.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("pedradb-from-rocks-{name}-{n}-{i}"));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn open_rocks(path: &std::path::Path) -> rocksdb::DB {
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    rocksdb::DB::open(&opts, path).expect("rocks open")
}

fn snapshot_dir(path: &std::path::Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for ent in rd.flatten() {
            let p = ent.path();
            let name = ent.file_name();
            if name == "LOCK" {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let rel = p.strip_prefix(path).unwrap().to_string_lossy().into_owned();
            out.insert(rel, std::fs::read(&p).unwrap_or_default());
        }
    }
    out
}

#[test]
fn migrate_from_rocks_default_cf_visible_snapshot_is_pedra_v5() {
    let src = temp("src");
    let dst = temp("dst");
    {
        let db = open_rocks(&src);
        db.put(b"keep", b"v1").unwrap();
        db.put(b"gone", b"v0").unwrap();
        db.delete(b"gone").unwrap();
        db.put(b"keep", b"v2").unwrap();
        db.flush().unwrap();
    }
    assert_eq!(classify_dir(&src).unwrap(), DirKind::Rocks);

    let report = migrate_from_rocks(&src, &dst).expect("copy");
    assert!(report.verified);
    assert_eq!(
        report.keys, 1,
        "delete must not resurrect; overwrite last-wins"
    );
    assert!(
        report.cfs.iter().any(|n| n == "default"),
        "default-only source still declares the default CF, got {:?}",
        report.cfs
    );
    assert_eq!(
        report.bytes,
        (b"keep".len() + b"v2".len()) as u64,
        "report bytes must match the planted visible pair"
    );
    assert!((1..=6).contains(&report.sst_version));
    assert!(report.ssts_written >= 1);

    let inspect = inspect_format(&dst).unwrap();
    assert_eq!(inspect.kind, DirKind::Pedra);
    assert_eq!(
        inspect.manifest_format, 5,
        "dest must be Pedra MANIFEST v5, got {}",
        inspect.manifest_format
    );
    assert!(
        inspect
            .sst_versions
            .iter()
            .all(|(_, v)| (1..=6).contains(v)),
        "dest SST must be Pedra (v1-v6), got {:?}",
        inspect.sst_versions
    );

    let dest = pedradb_core::db::Db::open(&dst).unwrap();
    assert_eq!(dest.get(b"keep").as_deref(), Some(b"v2".as_ref()));
    assert!(dest.get(b"gone").is_none(), "deleted key must stay gone");
    dest.verify_checksums().unwrap();
    dest.close().unwrap();

    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dst);
}

/// RFC-0186 P1.1+P1.2: named CFs copy into physical Pedra CFs via apply_batch.
#[test]
fn migrate_from_rocks_named_cfs_copy_physical() {
    let src = temp("cf-src");
    let dst = temp("cf-dst");
    {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = rocksdb::DB::open_cf(&opts, &src, ["default", "write", "lock"]).expect("rocks cf");
        db.put(b"keep", b"v1").unwrap();
        db.put(b"gone", b"v0").unwrap();
        db.delete(b"gone").unwrap();
        db.put(b"keep", b"v2").unwrap();
        let write = db.cf_handle("write").expect("write cf");
        db.put_cf(&write, b"w", b"v1").unwrap();
        db.put_cf(&write, b"wgone", b"x").unwrap();
        db.delete_cf(&write, b"wgone").unwrap();
        db.put_cf(&write, b"w", b"v2").unwrap();
        let lock = db.cf_handle("lock").expect("lock cf");
        db.put_cf(&lock, b"l", b"L").unwrap();
        db.flush().unwrap();
        let _ = db.flush_cf(&write);
        let _ = db.flush_cf(&lock);
        drop(db);
    }
    assert_eq!(classify_dir(&src).unwrap(), DirKind::Rocks);

    let report = migrate_from_rocks(&src, &dst).expect("copy named CFs");
    assert!(report.verified);
    // visible: default keep, write w, lock l
    assert_eq!(report.keys, 3, "one visible key per CF");
    let expected_bytes =
        (b"keep".len() + b"v2".len() + b"w".len() + b"v2".len() + b"l".len() + b"L".len()) as u64;
    assert_eq!(report.bytes, expected_bytes);
    assert!(
        report.ssts_written >= 3,
        "one SST per non-empty CF, got {}",
        report.ssts_written
    );
    for family in ["default", "write", "lock"] {
        assert!(
            report.cfs.iter().any(|n| n == family),
            "report must declare physical CF {family}, got {:?}",
            report.cfs
        );
    }

    let inspect = inspect_format(&dst).unwrap();
    assert_eq!(inspect.kind, DirKind::Pedra);
    assert_eq!(inspect.manifest_format, 5);

    // Reopen without set_physical_cfs: MANIFEST v5 SST tags + encoded keys
    // are the durable physical-CF declaration.
    let dest = Db::open(&dst).unwrap();
    assert_eq!(
        dest.get(b"keep").as_deref(),
        Some(b"v2".as_ref()),
        "default stays raw"
    );
    assert!(dest.get(b"gone").is_none());
    let wkey = encode_cf_key("write", b"w", true);
    let lkey = encode_cf_key("lock", b"l", true);
    assert_eq!(dest.get(&wkey).as_deref(), Some(b"v2".as_ref()));
    assert!(dest.get(&encode_cf_key("write", b"wgone", true)).is_none());
    assert_eq!(dest.get(&lkey).as_deref(), Some(b"L".as_ref()));
    dest.verify_checksums().unwrap();

    let meta = dest.live_sst_meta();
    for family in ["default", "write", "lock"] {
        assert!(
            meta.iter().any(|m| m.cf == family),
            "non-empty CF {family} must have its own SST after reopen, got {meta:?}"
        );
    }
    dest.close().unwrap();

    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dst);
}

/// RFC-0186 P1.3: a real merge-operator Rocks source refuses; dest unusable.
#[test]
fn migrate_from_rocks_merge_operator_refuses() {
    let src = temp("merge-src");
    let dst = temp("merge-dst");
    {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.set_merge_operator_associative("rfc0186-merge", |_k, existing, operands| {
            let mut out = existing.map(<[u8]>::to_vec).unwrap_or_default();
            for op in operands {
                out.extend_from_slice(op);
            }
            Some(out)
        });
        let db = rocksdb::DB::open(&opts, &src).expect("rocks merge open");
        db.merge(b"k", b"a").unwrap();
        db.merge(b"k", b"b").unwrap();
        db.flush().unwrap();
        drop(db);
    }
    let err = migrate_from_rocks(&src, &dst).unwrap_err().to_string();
    assert!(err.contains("merge"), "must name merge operands, got {err}");
    if dst.exists() {
        let n = std::fs::read_dir(&dst).map(|i| i.count()).unwrap_or(0);
        assert_eq!(n, 0, "no partial dest after merge refuse");
    }
    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dst);
}

/// RFC-0186 P2.1: src bytes unchanged; checkpoint temp dir gone after Ok.
#[test]
fn migrate_from_rocks_checkpoint_leaves_src_untouched() {
    let src = temp("ckpt-src");
    let dst = temp("ckpt-dst");
    {
        let db = open_rocks(&src);
        db.put(b"k", b"v").unwrap();
        db.flush().unwrap();
    }
    let before = snapshot_dir(&src);
    let parent = src.parent().unwrap().to_path_buf();
    let src_name = src.file_name().unwrap().to_string_lossy().into_owned();
    let prefix = format!(".pedra-rocks-ckpt-{src_name}-");

    let report = migrate_from_rocks(&src, &dst).expect("copy");
    assert!(report.verified);
    assert_eq!(snapshot_dir(&src), before, "src must be byte-identical");

    let leftovers: Vec<_> = std::fs::read_dir(&parent)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.starts_with(&prefix))
                .unwrap_or(false)
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "checkpoint temp must be removed on success: {leftovers:?}"
    );

    let dest = Db::open(&dst).unwrap();
    assert_eq!(dest.get(b"k").as_deref(), Some(b"v".as_ref()));
    dest.close().unwrap();
    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dst);
}

/// RFC-0186 P2.1: injected error after checkpoint still cleans the temp dir.
#[test]
fn migrate_from_rocks_checkpoint_cleaned_on_failure() {
    let src = temp("ckpt-fail-src");
    {
        let db = open_rocks(&src);
        db.put(b"k", b"v").unwrap();
        db.flush().unwrap();
    }
    let parent = src.parent().unwrap().to_path_buf();
    let src_name = src.file_name().unwrap().to_string_lossy().into_owned();
    let prefix = format!(".pedra-rocks-ckpt-{src_name}-");
    let err = pedradb_ops::checkpoint_then_err_for_test(&src)
        .unwrap_err()
        .to_string();
    assert!(err.contains("injected failure"), "{err}");
    let leftovers: Vec<_> = std::fs::read_dir(&parent)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.starts_with(&prefix))
                .unwrap_or(false)
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "checkpoint temp must be removed on failure: {leftovers:?}"
    );
    let _ = std::fs::remove_dir_all(&src);
}
