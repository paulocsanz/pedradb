//! Alias-swap proof: this consumer only knows `use rocksdb::…`.
//! Building and running it demonstrates the rust-rocksdb→pedradb substitution
//! mechanism (see docs/rocksdb-compat.md — lab foundation, not drop-in TiKV).

#![forbid(unsafe_code)]

use rocksdb::{
    backup::{BackupEngine, BackupEngineOptions, RestoreOptions},
    checkpoint::Checkpoint,
    Direction, Env, IteratorMode, Options, WriteBatch, DB,
};

fn main() {
    let dir = std::env::temp_dir().join(format!("rdbcompat-alias-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut opts = Options::new();
    opts.create_if_missing(true);
    let db = DB::open_cf(&opts, &dir, &["raft"]).expect("open");

    let raft = db.cf_handle("raft").expect("raft cf");
    let mut wb = WriteBatch::new();
    wb.put(b"kv/1", b"v1");
    wb.put_cf(&raft, b"log/1", b"term1");
    db.write(&wb).expect("batch");

    let mut wb = WriteBatch::new();
    wb.delete(b"kv/1");
    db.write(&wb).expect("delete batch");

    assert_eq!(db.get(b"kv/1").unwrap(), None);
    assert_eq!(
        db.get_cf(&raft, b"log/1").unwrap().as_deref(),
        Some(&b"term1"[..])
    );

    db.put(b"kv/2", b"v2").unwrap();
    let it = db.iterator(IteratorMode::Start).unwrap();
    assert_eq!(it.key(), b"kv/2");
    let it = db
        .iterator(IteratorMode::From(b"kv/1", Direction::Forward))
        .unwrap();
    assert_eq!(it.key(), b"kv/2");

    let snap = db.snapshot();
    db.put(b"kv/3", b"v3").unwrap();
    assert_eq!(snap.get(b"kv/3").unwrap(), None);

    let ckpt = std::env::temp_dir().join(format!("rdbcompat-alias-ckpt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ckpt);
    Checkpoint::new(&db)
        .expect("checkpoint obj")
        .create_checkpoint(&ckpt)
        .expect("checkpoint");
    let opened = DB::open_cf(&opts, &ckpt, &["raft"]).expect("open checkpoint");
    assert_eq!(opened.get(b"kv/2").unwrap().as_deref(), Some(&b"v2"[..]));
    drop(opened);

    let backup = std::env::temp_dir().join(format!("rdbcompat-alias-bak-{}", std::process::id()));
    let restore = std::env::temp_dir().join(format!("rdbcompat-alias-rst-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&backup);
    let _ = std::fs::remove_dir_all(&restore);
    let env = Env::new().unwrap();
    let bopts = BackupEngineOptions::new(&backup).unwrap();
    let mut eng = BackupEngine::open(&bopts, &env).unwrap();
    eng.create_new_backup_flush(&db, true).unwrap();
    let info = eng.get_backup_info();
    assert!(!info.is_empty());
    eng.verify_backup(info[0].backup_id).unwrap();
    let ropts = RestoreOptions::default();
    eng.restore_from_latest_backup(&restore, &restore, &ropts)
        .unwrap();
    let restored = DB::open_cf(&opts, &restore, &["raft"]).unwrap();
    assert_eq!(restored.get(b"kv/2").unwrap().as_deref(), Some(&b"v2"[..]));
    drop(restored);

    println!(
        "alias-smoke ok: rocksdb-named consumer running on pedradb ({})",
        dir.display()
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&ckpt);
    let _ = std::fs::remove_dir_all(&backup);
    let _ = std::fs::remove_dir_all(&restore);
}
