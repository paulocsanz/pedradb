//! Same-schedule differential: rocksdb-compat (Pedra, G1 default) vs real RocksDB.
//!
//! Not a performance claim. Catches silent API/behavior drift: put/get/delete,
//! overwrite, empty values, interned identical payloads, WriteBatch atomicity,
//! snapshots, iterators, reopen. `cargo test -p rocksdb-parity-bench --features real`.

#![cfg(feature = "real")]

use rocksdb_compat as compat;
use std::collections::BTreeMap;
use std::path::Path;

fn xorshift(rng: &mut u64) -> u64 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    *rng
}

fn tmp(tag: &str) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let d = tempfile::tempdir().expect("tmp");
    let c = d.path().join(format!("{tag}-compat"));
    let r = d.path().join(format!("{tag}-rocks"));
    std::fs::create_dir_all(&c).unwrap();
    std::fs::create_dir_all(&r).unwrap();
    (d, c, r)
}

fn open_compat(path: &Path) -> compat::DB {
    let mut opts = compat::Options::new();
    opts.create_if_missing(true);
    compat::DB::open(&opts, path).expect("compat open")
}

fn open_rocks(path: &Path) -> rocksdb::DB {
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    rocksdb::DB::open(&opts, path).expect("rocks open")
}

fn scan_compat(db: &compat::DB) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut it = db
        .iterator(compat::IteratorMode::Start)
        .expect("compat iter");
    let mut out = Vec::new();
    while it.valid() {
        out.push((it.key().to_vec(), it.value().to_vec()));
        it.next();
        if out.len() > 10_000 {
            break;
        }
    }
    out
}

fn scan_rocks(db: &rocksdb::DB) -> Vec<(Vec<u8>, Vec<u8>)> {
    db.iterator(rocksdb::IteratorMode::Start)
        .map_while(Result::ok)
        .map(|(k, v)| (k.as_ref().to_vec(), v.as_ref().to_vec()))
        .collect()
}

fn map_of(pairs: &[(Vec<u8>, Vec<u8>)]) -> BTreeMap<Vec<u8>, Vec<u8>> {
    pairs.iter().cloned().collect()
}

#[test]
fn put_get_delete_overwrite_empty_match() {
    let (_keep, cp, rp) = tmp("basic");
    let c = open_compat(&cp);
    let r = open_rocks(&rp);
    for (k, v) in [(&b"a"[..], &b"1"[..]), (b"b", b""), (b"c", b"xxxxxxxx")] {
        c.put(k, v).unwrap();
        r.put(k, v).unwrap();
        assert_eq!(c.get(k).unwrap(), r.get(k).unwrap(), "get {k:?}");
    }
    c.put(b"a", b"2").unwrap();
    r.put(b"a", b"2").unwrap();
    assert_eq!(c.get(b"a").unwrap().as_deref(), Some(&b"2"[..]));
    assert_eq!(c.get(b"a").unwrap(), r.get(b"a").unwrap());
    c.delete(b"b").unwrap();
    r.delete(b"b").unwrap();
    assert_eq!(c.get(b"b").unwrap(), None);
    assert_eq!(c.get(b"b").unwrap(), r.get(b"b").unwrap());
    assert_eq!(map_of(&scan_compat(&c)), map_of(&scan_rocks(&r)));
}

#[test]
fn interned_same_payload_survives_reopen_on_both() {
    let (_keep, cp, rp) = tmp("intern");
    let payload = vec![b'k'; 1024];
    {
        let c = open_compat(&cp);
        let r = open_rocks(&rp);
        for i in 0..40u8 {
            let k = [b'q', i];
            c.put(&k, &payload).unwrap();
            r.put(&k, &payload).unwrap();
        }
        c.flush().unwrap();
        r.flush().unwrap();
        assert_eq!(map_of(&scan_compat(&c)), map_of(&scan_rocks(&r)));
        drop(c);
        drop(r);
    }
    let c = open_compat(&cp);
    let r = open_rocks(&rp);
    for i in 0..40u8 {
        let k = [b'q', i];
        assert_eq!(
            c.get(&k).unwrap().as_deref(),
            Some(payload.as_slice()),
            "compat lost q/{i}"
        );
        assert_eq!(c.get(&k).unwrap(), r.get(&k).unwrap(), "reopen q/{i}");
    }
}

#[test]
fn write_batch_is_atomic_on_both() {
    let (_keep, cp, rp) = tmp("batch");
    let c = open_compat(&cp);
    let r = open_rocks(&rp);
    let mut cb = compat::WriteBatch::new();
    cb.put(b"x", b"1");
    cb.put(b"y", b"2");
    cb.delete(b"missing");
    c.write(&cb).unwrap();
    let mut rb = rocksdb::WriteBatch::default();
    rb.put(b"x", b"1");
    rb.put(b"y", b"2");
    rb.delete(b"missing");
    r.write(rb).unwrap();
    assert_eq!(c.get(b"x").unwrap(), r.get(b"x").unwrap());
    assert_eq!(c.get(b"y").unwrap(), r.get(b"y").unwrap());
    assert_eq!(c.get(b"missing").unwrap(), None);
}

#[test]
fn snapshot_hides_later_puts_on_both() {
    let (_keep, cp, rp) = tmp("snap");
    let c = open_compat(&cp);
    let r = open_rocks(&rp);
    c.put(b"k", b"old").unwrap();
    r.put(b"k", b"old").unwrap();
    let cs = c.snapshot();
    let rs = r.snapshot();
    c.put(b"k", b"new").unwrap();
    r.put(b"k", b"new").unwrap();
    assert_eq!(cs.get(b"k").unwrap().as_deref(), Some(&b"old"[..]));
    assert_eq!(
        rs.get(b"k").unwrap().as_deref().map(|b| b.as_ref()),
        Some(&b"old"[..])
    );
    assert_eq!(c.get(b"k").unwrap().as_deref(), Some(&b"new"[..]));
    assert_eq!(r.get(b"k").unwrap().as_deref(), Some(&b"new"[..]));
}

#[test]
fn iterator_from_matches_across_engines() {
    let (_keep, cp, rp) = tmp("iter");
    let c = open_compat(&cp);
    let r = open_rocks(&rp);
    for i in 0..80usize {
        let k = format!("k{i:04}").into_bytes();
        let v = [i as u8];
        c.put(&k, v).unwrap();
        r.put(&k, v).unwrap();
    }
    let mid = b"k0040";
    let mut cit = c
        .iterator(compat::IteratorMode::From(mid, compat::Direction::Forward))
        .unwrap();
    let cgot: Vec<Vec<u8>> = {
        let mut v = Vec::new();
        while cit.valid() {
            v.push(cit.key().to_vec());
            cit.next();
        }
        v
    };
    let rgot: Vec<Vec<u8>> = r
        .iterator(rocksdb::IteratorMode::From(
            mid,
            rocksdb::Direction::Forward,
        ))
        .map_while(Result::ok)
        .map(|(k, _)| k.as_ref().to_vec())
        .collect();
    assert_eq!(cgot, rgot, "forward-from k0040");
    assert_eq!(cgot.first().map(Vec::as_slice), Some(mid.as_ref()));
}

/// Random campaign: after every mutating op, point-get the working set on both.
#[test]
fn random_campaign_point_gets_match() {
    let (_keep, cp, rp) = tmp("camp");
    let c = open_compat(&cp);
    let r = open_rocks(&rp);
    let mut rng = 0xC0FF_EE00_u64;
    let mut live: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    for _ in 0..256 {
        let op = xorshift(&mut rng) % 8;
        let k = format!("k{:02}", xorshift(&mut rng) % 40).into_bytes();
        match op {
            0..=3 => {
                let v = format!("v{}", xorshift(&mut rng) % 200).into_bytes();
                c.put(&k, &v).unwrap();
                r.put(&k, &v).unwrap();
                live.insert(k, v);
            }
            4 => {
                c.delete(&k).unwrap();
                r.delete(&k).unwrap();
                live.remove(&k);
            }
            5 => {
                let v = vec![b'p'; 256];
                let k2 = format!("p{:02}", xorshift(&mut rng) % 16).into_bytes();
                c.put(&k2, &v).unwrap();
                r.put(&k2, &v).unwrap();
                live.insert(k2, v);
            }
            _ => {
                let mut cb = compat::WriteBatch::new();
                let mut rb = rocksdb::WriteBatch::default();
                let k2 = format!("b{:02}", xorshift(&mut rng) % 8).into_bytes();
                let v = b"batch".to_vec();
                cb.put(&k2, &v);
                rb.put(&k2, &v);
                cb.delete(b"nope");
                rb.delete(b"nope");
                c.write(&cb).unwrap();
                r.write(rb).unwrap();
                live.insert(k2, v);
                live.remove(&b"nope"[..]);
            }
        }
        for (qk, qv) in &live {
            assert_eq!(
                c.get(qk).unwrap().as_deref(),
                Some(qv.as_slice()),
                "compat drift {qk:?}"
            );
            assert_eq!(
                r.get(qk).unwrap().as_deref(),
                Some(qv.as_slice()),
                "rocks drift {qk:?}"
            );
        }
        for i in 0..40u64 {
            let qk = format!("k{i:02}").into_bytes();
            assert_eq!(
                c.get(&qk).unwrap(),
                r.get(&qk).unwrap(),
                "point mismatch {qk:?}"
            );
        }
    }
    assert_eq!(map_of(&scan_compat(&c)), map_of(&scan_rocks(&r)));
}
