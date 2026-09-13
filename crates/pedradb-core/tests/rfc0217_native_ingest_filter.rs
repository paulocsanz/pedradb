//! RFC-0217 P1.2 — native ingest + native compaction filter, on the real
//! `Db` path (no mocks): `Db::ingest_sst_file` (fresh global seqs, direct
//! SST write + install, durable MANIFEST point before Ok) and
//! `Db::compact_filter_families` (decision inside the merge — removed keys
//! never reach an output SST).

use bytes::Bytes;
use pedradb_core::key::InternalKey;
use pedradb_core::merge::CompactFilterDecision;
use pedradb_core::{Db, MemTable, ValueType};

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rfc0217-p12-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build an external Pedra SST at `path` from `(key, seq, value)` triples
/// (seqs are writer-local, like `SstFileWriter`).
fn write_external_sst(path: &std::path::Path, entries: &[(&[u8], u64, &[u8])]) {
    let mut mem = MemTable::new();
    for (k, seq, v) in entries {
        mem.insert(
            InternalKey::new(Bytes::copy_from_slice(*k), *seq, ValueType::Value),
            Bytes::copy_from_slice(*v),
        );
    }
    pedradb_core::sst::write_sst(path, &mem).unwrap();
}

#[test]
fn ingest_sst_visible_and_durable_across_reopen() {
    let dir = temp_dir("ingest-durable");
    let ext = dir.join("external.sst");
    write_external_sst(&ext, &[(b"ing/k1", 1, b"v1"), (b"ing/k2", 1, b"v2")]);
    {
        let mut db = Db::open(&dir).unwrap();
        db.ingest_sst_file(&ext, "").unwrap();
        // Visible immediately (no flush, no put).
        assert_eq!(db.get(b"ing/k1").as_deref(), Some(&b"v1"[..]));
        assert_eq!(db.get(b"ing/k2").as_deref(), Some(&b"v2"[..]));
        // The ingested file is the only copy — the durability point was the
        // MANIFEST publish inside ingest.
        assert!(db.sst_count() >= 1);
    }
    let db = Db::open(&dir).unwrap();
    assert_eq!(
        db.get(b"ing/k1").as_deref(),
        Some(&b"v1"[..]),
        "ingested key must survive reopen (MANIFEST publish is the point)"
    );
    assert_eq!(db.get(b"ing/k2").as_deref(), Some(&b"v2"[..]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ingest_preserves_recency_order_of_versions() {
    let dir = temp_dir("ingest-recency");
    let ext = dir.join("external.sst");
    // Same user key twice: local seq 2 (newest) must stay newest after the
    // global re-sequencing.
    write_external_sst(
        &ext,
        &[(b"ing/k", 1, b"old"), (b"ing/k", 2, b"new")],
    );
    let mut db = Db::open(&dir).unwrap();
    db.ingest_sst_file(&ext, "").unwrap();
    assert_eq!(
        db.get(b"ing/k").as_deref(),
        Some(&b"new"[..]),
        "the newest local version must remain the visible one after re-seq"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ingest_orders_against_writes_before_and_after() {
    let dir = temp_dir("ingest-order");
    let mut db = Db::open(&dir).unwrap();
    db.put(b"ing/k", b"pre").unwrap();
    let ext = dir.join("external.sst");
    write_external_sst(&ext, &[(b"ing/k", 1, b"ingested")]);
    db.ingest_sst_file(&ext, "").unwrap();
    assert_eq!(
        db.get(b"ing/k").as_deref(),
        Some(&b"ingested"[..]),
        "ingested seqs are above every prior write's seq"
    );
    db.put(b"ing/k", b"post").unwrap();
    assert_eq!(
        db.get(b"ing/k").as_deref(),
        Some(&b"post"[..]),
        "writes after ingest get seqs above the ingested range"
    );
    // Reopen: WAL replay (pre/post) must not resurrect over the ingested
    // SST nor lose it — the manifest floor covers the ingested seqs.
    drop(db);
    let db = Db::open(&dir).unwrap();
    assert_eq!(db.get(b"ing/k").as_deref(), Some(&b"post"[..]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ingest_empty_file_is_ok_noop() {
    let dir = temp_dir("ingest-empty");
    let ext = dir.join("external.sst");
    write_external_sst(&ext, &[]);
    let mut db = Db::open(&dir).unwrap();
    db.ingest_sst_file(&ext, "").unwrap();
    assert_eq!(db.get(b"nothing").as_deref(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compact_filter_drops_prefix_keeps_rest_and_is_durable() {
    let dir = temp_dir("filter-prefix");
    {
        let mut db = Db::open(&dir).unwrap();
        for i in 0..5u32 {
            db.put(format!("keep/{i:03}").as_bytes(), b"keepval").unwrap();
            db.put(format!("drop/{i:03}").as_bytes(), b"dropval").unwrap();
        }
        db.flush().unwrap();
        // A mem-resident key must be filtered too (flush first inside the
        // wrapper contract is exercised by ConcurrentDb; here the Db-level
        // call sees SSTs only — put one more after the flush).
        db.put(b"drop/mem", b"dropval").unwrap();
        db.flush().unwrap();
        let mut drops = 0u32;
        let mut decision = |_cf: &str, key: &[u8], _val: &[u8]| {
            if key.starts_with(b"drop/") {
                drops += 1;
                CompactFilterDecision::Remove
            } else {
                CompactFilterDecision::Keep
            }
        };
        db.compact_filter_families(&mut decision).unwrap();
        // The filter saw every drop/ run exactly once (newest decides).
        assert_eq!(drops, 6, "one decision per drop/ user-key run");
        for i in 0..5u32 {
            assert_eq!(
                db.get(format!("keep/{i:03}").as_bytes()).as_deref(),
                Some(&b"keepval"[..])
            );
            assert_eq!(db.get(format!("drop/{i:03}").as_bytes()).as_deref(), None);
        }
        assert_eq!(db.get(b"drop/mem").as_deref(), None);
    }
    let db = Db::open(&dir).unwrap();
    assert_eq!(db.get(b"keep/000").as_deref(), Some(&b"keepval"[..]));
    assert_eq!(db.get(b"drop/000").as_deref(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compact_filter_change_rewrites_value_during_merge() {
    let dir = temp_dir("filter-change");
    {
        let mut db = Db::open(&dir).unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"b", b"2").unwrap();
        db.flush().unwrap();
        let mut decision = |_cf: &str, key: &[u8], _val: &[u8]| {
            if key == b"b" {
                CompactFilterDecision::Change(Bytes::from_static(b"rewritten"))
            } else {
                CompactFilterDecision::Keep
            }
        };
        db.compact_filter_families(&mut decision).unwrap();
        assert_eq!(db.get(b"a").as_deref(), Some(&b"1"[..]));
        assert_eq!(db.get(b"b").as_deref(), Some(&b"rewritten"[..]));
    }
    let db = Db::open(&dir).unwrap();
    assert_eq!(db.get(b"b").as_deref(), Some(&b"rewritten"[..]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compact_filter_does_not_resurrect_tombstoned_keys() {
    let dir = temp_dir("filter-tombstone");
    let mut db = Db::open(&dir).unwrap();
    db.put(b"gone", b"1").unwrap();
    db.flush().unwrap();
    db.delete(b"gone").unwrap();
    db.flush().unwrap();
    // Keep-all filter: the tombstone passes through (no resurrection), and
    // the bottommost whole-keyspace rewrite may GC it — either way `gone`
    // stays invisible.
    let mut decision = |_cf: &str, _key: &[u8], _val: &[u8]| CompactFilterDecision::Keep;
    db.compact_filter_families(&mut decision).unwrap();
    assert_eq!(db.get(b"gone").as_deref(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compact_filter_remove_drops_old_versions_too_no_resurrect() {
    let dir = temp_dir("filter-run");
    let mut db = Db::open(&dir).unwrap();
    db.put(b"victim", b"v1").unwrap();
    db.flush().unwrap();
    db.put(b"victim", b"v2").unwrap();
    db.flush().unwrap();
    // Two versions of `victim` live in different SSTs: the whole-keyspace
    // rewrite covers both files, so Remove must drop the ENTIRE run — an
    // older version may not resurface from a non-input file.
    let mut decision = |_cf: &str, key: &[u8], _val: &[u8]| {
        if key == b"victim" {
            CompactFilterDecision::Remove
        } else {
            CompactFilterDecision::Keep
        }
    };
    db.compact_filter_families(&mut decision).unwrap();
    assert_eq!(db.get(b"victim").as_deref(), None);
    drop(db);
    let db = Db::open(&dir).unwrap();
    assert_eq!(
        db.get(b"victim").as_deref(),
        None,
        "no version of a removed key resurfaces after reopen"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
