//! RFC-0050 P1.2: a World trial opens the `pedradb-index` and `pedradb-journal`
//! layer canaries under `FailingEnv` — the lab drives the product layers on
//! the same Env-injectable surface as the cluster nodes (no `StdEnv` island).

use pedradb_core::{Db, OpenOptions};
use pedradb_sim::FailingEnv;
use pedradb_world::{World, WorldConfig};

fn scratch(tag: &str) -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir().join(format!("pedra-world-canary-{tag}-{n}"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn world_trial_opens_index_and_journal_canaries_under_failing_env() {
    let parent = scratch("p12");

    // Seeded cluster trial completes with zero safety violations first.
    let trace = World::new(
        0xCA11_0002,
        WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.join("world"),
            exchange_rounds: 32,
            ..Default::default()
        },
    )
    .run()
    .unwrap();
    assert_eq!(trace.silent_wrong, 0, "{trace:?}");
    assert_eq!(trace.dual_leader_fail_open, 0);

    // Journal canary (RFC-0020 W4): open under FailingEnv with per-op delay —
    // every read/write goes through the injected Env. catch_up must pin the
    // watermark at the durable sequence (never ahead of last_sequence).
    let jdir = parent.join("canary-journal");
    let jenv = FailingEnv::passing();
    jenv.set_delay_per_op(1);
    let mut jdb = Db::open_with_env(&jdir, OpenOptions::default(), jenv).unwrap();
    let seq = pedradb_journal::append(&mut jdb, b"canary/j", b"1").unwrap();
    let mut consumer = pedradb_journal::JournalConsumer::new();
    let batch = consumer.catch_up(&jdb);
    assert_eq!(batch.len(), 1);
    assert_eq!(consumer.pin, seq);
    assert!(batch[0].sequence <= jdb.last_sequence());

    // Index canary (RFC-0020 W3): multi-key row + both secondary indexes on a
    // FailingEnv Db; the row must be fully indexed (all-or-nothing), never
    // half-indexed.
    let idir = parent.join("canary-index");
    let ienv = FailingEnv::passing();
    ienv.set_delay_per_op(1);
    let mut idb = Db::open_with_env(&idir, OpenOptions::default(), ienv).unwrap();
    pedradb_index::put_row_with_indexes(&mut idb, b"r1", b"payload", b"name", b"e@x").unwrap();
    assert!(pedradb_index::row_fully_indexed(&idb, b"r1", b"name", b"e@x"));
    assert!(!pedradb_index::row_half_indexed(&idb, b"r1", b"name", b"e@x"));

    let _ = std::fs::remove_dir_all(&parent);
}
