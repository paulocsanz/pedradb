//! The external-reviewer escape bug: `ingest_sst_file` (the native
//! `ingest_external_file` path) stored ingested values VERBATIM while every
//! read strips a leading `0x01` inline-escape marker — a raw value starting
//! with `0x01` came back one byte short (silent corruption for exactly the
//! "move data between databases" workload this API exists for). The put /
//! tx paths pre-escape (`escape_inline_value`); ingest must too.

use bytes::Bytes;
use pedradb_core::key::{InternalKey, ValueType};
use pedradb_core::{Db, OpenOptions};

#[test]
fn ingest_round_trips_values_starting_with_escape_marker() {
    let base = std::env::temp_dir().join(format!("ingest-esc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let db_dir = base.join("db");
    let mut db = Db::open_with(&db_dir, OpenOptions::default()).unwrap();

    // A source SST holding RAW values, the way an external producer
    // (write_sst_entries / another engine export) delivers them.
    let src = base.join("src-raw.sst");
    let meta: Bytes = Bytes::from(vec![
        1u8, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    let entries = vec![
        (
            InternalKey::new(Bytes::from_static(b"m"), 10, ValueType::Value),
            meta.clone(),
        ),
        (
            InternalKey::new(Bytes::from_static(b"esc2"), 9, ValueType::Value),
            Bytes::from_static(b"\x01\x01double"),
        ),
        (
            InternalKey::new(Bytes::from_static(b"plain"), 8, ValueType::Value),
            Bytes::from_static(b"plain"),
        ),
    ];
    let table = pedradb_core::sst::write_sst_entries(&src, &entries).unwrap();
    drop(table);

    db.ingest_sst_file(&src, "").unwrap();

    assert_eq!(
        db.get(b"m").expect("ingested meta key"),
        meta,
        "ingest path mangled the 0x01-leading value (verbatim store, read strips the escape)"
    );
    assert_eq!(
        db.get(b"esc2").expect("ingested esc2 key"),
        Bytes::from_static(b"\x01\x01double"),
        "double-marker value lost an escape byte"
    );
    assert_eq!(
        db.get(b"plain").expect("ingested plain key"),
        Bytes::from_static(b"plain"),
        "plain value corrupted by ingest"
    );
    drop(db);
    let _ = std::fs::remove_dir_all(&base);
}
