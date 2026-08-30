//! RFC-0020 P1.5 — SST + MANIFEST codec fuzz smoke (bounded, no panic).

use bytes::Bytes;
use pedradb_core::manifest::{self, VersionSet};
use pedradb_core::{
    write_sst_entries_on, InternalKey, SequenceNumber, StdEnv, ValueType, WAL_FILE_NAME,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(tag: &str) -> std::path::PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let i = N.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("pedra-fuzz-{tag}-{n}-{i}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn mutate(seed: &[u8], step: u64) -> Vec<u8> {
    let mut b = seed.to_vec();
    if b.is_empty() {
        return vec![(step & 0xff) as u8];
    }
    let i = (step as usize) % b.len();
    match step % 6 {
        0 => b[i] ^= 0xff,
        1 => b[i] = b[i].wrapping_add(3),
        2 => b.truncate((i + 1).min(b.len())),
        3 => b.push(0xCD),
        4 => b.insert(i, 0x11),
        _ => {
            if b.len() >= 4 {
                let n = b.len();
                b[n - 1] ^= 0x55;
            }
        }
    }
    b
}

#[test]
fn codec_fuzz_smoke_manifest() {
    let seed = {
        let mut vs = VersionSet::empty();
        vs.next_file_num = 7;
        vs.sst_file_nums = vec![1, 2, 3];
        vs.sst_levels = vec![0, 0, 1];
        vs.manifest_file_num = 1;
        vs.vlog_use_new = false;
        manifest::encode(&vs)
    };
    assert!(manifest::decode(&seed).is_ok());
    let mut err = 0u64;
    for step in 0..256u64 {
        let m = mutate(&seed, step);
        match std::panic::catch_unwind(|| manifest::decode(&m)) {
            Ok(Ok(_)) => {}
            Ok(Err(_)) => err += 1,
            Err(_) => panic!("manifest::decode panicked step={step}"),
        }
    }
    assert!(err > 0);
}

#[test]
fn codec_fuzz_smoke_sst_open() {
    let dir = temp_dir("sst");
    let env = StdEnv;
    let path = dir.join("000001.sst");
    let entries = vec![
        (
            InternalKey::new(&b"a"[..], 1 as SequenceNumber, ValueType::Value),
            Bytes::from_static(b"1"),
        ),
        (
            InternalKey::new(&b"b"[..], 2, ValueType::Value),
            Bytes::from_static(b"2"),
        ),
    ];
    write_sst_entries_on(&env, &path, &entries).unwrap();
    let seed = std::fs::read(&path).unwrap();
    assert!(!seed.is_empty());

    // Mutated SST files: open must not panic (Err ok).
    for step in 0..64u64 {
        let m = mutate(&seed, step);
        let p = dir.join(format!("mut-{step}.sst"));
        std::fs::write(&p, &m).unwrap();
        let r = std::panic::catch_unwind(|| {
            let _ = pedradb_core::SstTable::open_on(&env, &p);
        });
        assert!(r.is_ok(), "SstTable::open_on panicked step={step}");
    }
    // Also poke WAL name path existence (no open required).
    assert_eq!(WAL_FILE_NAME, "CURRENT.log");
    let _ = std::fs::remove_dir_all(&dir);
}
