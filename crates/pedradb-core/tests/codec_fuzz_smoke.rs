//! RFC-0020 P0.5 — bounded codec fuzz smoke for WAL WriteRecord + CHANGELOG.
//!
//! Mutates seed corpora and asserts decode either succeeds or fails closed
//! (no panic). Not a full libFuzzer harness — CI-bounded deterministic smoke.

use pedradb_core::{
    decode_changelog, ChangeEntry, ChangeKind, ChangeLog, ValueType, WriteOp, WriteRecord,
};

fn seed_write_record() -> Vec<u8> {
    let rec = WriteRecord {
        ops: vec![
            WriteOp::put(1, &b"k1"[..], &b"v1"[..]),
            WriteOp::delete(2, &b"k2"[..]),
            WriteOp {
                kind: ValueType::RangeDeletion,
                sequence: 3,
                key: bytes::Bytes::from_static(b"a"),
                value: bytes::Bytes::from_static(b"z"),
            },
        ],
    };
    rec.encode()
}

fn seed_changelog() -> Vec<u8> {
    let mut log = ChangeLog::new();
    log.extend([
        ChangeEntry {
            sequence: 1,
            key: bytes::Bytes::from_static(b"a"),
            kind: ChangeKind::Put,
            value: bytes::Bytes::from_static(b"1"),
        },
        ChangeEntry {
            sequence: 2,
            key: bytes::Bytes::from_static(b"b"),
            kind: ChangeKind::Delete,
            value: bytes::Bytes::new(),
        },
    ]);
    // Encode via store round-trip helpers: use public API by round-tripping encode path.
    // ChangeLog does not expose encode_pub; re-build through store_on would need Env.
    // Use internal path via decode of a known-good file written by store_on.
    use pedradb_core::StdEnv;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let i = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pedra-chlog-fuzz-{n}-{i}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let env = StdEnv;
    log.store_on(&env, &dir).unwrap();
    let path = dir.join(pedradb_core::CHANGELOG_FILE_NAME);
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    bytes
}

fn mutate(seed: &[u8], step: u64) -> Vec<u8> {
    let mut b = seed.to_vec();
    if b.is_empty() {
        return vec![(step & 0xff) as u8];
    }
    let i = (step as usize) % b.len();
    match step % 7 {
        0 => {
            b[i] ^= 0xff;
        }
        1 => {
            b[i] = b[i].wrapping_add(1);
        }
        2 => {
            b.truncate(i.saturating_add(1).min(b.len()));
        }
        3 => {
            b.push((step & 0xff) as u8);
        }
        4 => {
            b.insert(i, 0xAB);
        }
        5 => {
            // Flip CRC-ish tail if long enough
            if b.len() >= 4 {
                let n = b.len();
                b[n - 1] ^= 1;
            }
        }
        _ => {
            b.extend_from_slice(&[0xDE, 0xAD]);
        }
    }
    b
}

/// Drive real `WriteRecord::decode` on mutated corpora — panic = fail.
#[test]
fn codec_fuzz_smoke_write_record() {
    let seed = seed_write_record();
    assert!(
        WriteRecord::decode(&seed).is_ok(),
        "seed corpus must decode"
    );
    let mut ok = 0u64;
    let mut err = 0u64;
    for step in 0..512u64 {
        let m = mutate(&seed, step);
        match std::panic::catch_unwind(|| WriteRecord::decode(&m)) {
            Ok(Ok(_)) => ok += 1,
            Ok(Err(_)) => err += 1,
            Err(_) => panic!(
                "WriteRecord::decode panicked on step={step} len={}",
                m.len()
            ),
        }
    }
    assert!(ok + err == 512);
    // At least some mutations should fail closed.
    assert!(err > 0, "expected some decode errors on mutations");
}

/// Drive real `decode_changelog` on mutated corpora — panic = fail.
#[test]
fn codec_fuzz_smoke_changelog() {
    let seed = seed_changelog();
    assert!(
        decode_changelog(&seed).is_ok(),
        "seed CHANGELOG corpus must decode"
    );
    let mut err = 0u64;
    for step in 0..512u64 {
        let m = mutate(&seed, step);
        match std::panic::catch_unwind(|| decode_changelog(&m)) {
            Ok(Ok(_)) => {}
            Ok(Err(_)) => err += 1,
            Err(_) => panic!("decode_changelog panicked on step={step} len={}", m.len()),
        }
    }
    assert!(err > 0, "expected some changelog decode errors");
}
