//! Multi-process smoke for Montanha-Store (RFC-0017 + universe A–E).
//!
//! Usage:
//!   montanha-store-smoke write|verify|multiwrite|partition|canaries|dcs-layer|etcd-need <parent_dir>
//!
//! - write/verify: cross-range commit_tx durability across OS processes
//! - multiwrite: multi-range multiwrite + fast RO
//! - partition: elect + put + minority partition NotCommitted + heal + majority
//! - canaries: lease exclusive + index batch + journal seq (multi-node), then exit for reopen verify
//! - dcs-layer / dcs-layer-verify: RFC-0017 P2.3 freeze — DCS + index batch **only** via
//!   StoreCluster (no standalone pedradb_dcs path)

#![forbid(unsafe_code)]

use pedradb_store::{meta_key, StoreCluster};

fn main() {
    let mut args = std::env::args().skip(1);
    let cmd = args
        .next()
        .expect("cmd: write|verify|multiwrite|partition|canaries|canaries-verify|dcs-layer|dcs-layer-verify|etcd-need|etcd-need-verify");
    let dir = args.next().expect("parent_dir");
    match cmd.as_str() {
        "write" => {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 3).expect("open");
            c.elect_all(120).expect("elect");
            // Keys guaranteed in different ranges by split (start-of-range style).
            let keys: Vec<Vec<u8>> = c
                .range_metas()
                .iter()
                .map(|r| {
                    if r.start.is_empty() {
                        vec![0x00, b't']
                    } else {
                        let mut k = r.start.clone();
                        k.push(b't');
                        k
                    }
                })
                .collect();
            assert!(keys.len() >= 2);
            c.commit_tx([
                (keys[0].as_slice(), b"mp-a".as_slice()),
                (keys[1].as_slice(), b"mp-b".as_slice()),
            ])
            .expect("commit_tx");
            // Stamp for verify without re-deriving split.
            std::fs::write(
                std::path::Path::new(&dir).join("smoke-keys.txt"),
                format!(
                    "{}\n{}\n",
                    hex::encode_upper_loose(&keys[0]),
                    hex::encode_upper_loose(&keys[1])
                ),
            )
            .ok();
            // Also write plain markers under meta for easy verify if hex fails.
            let _ = c.put(meta_key(b"smoke/k0"), &keys[0]);
            let _ = c.put(meta_key(b"smoke/k1"), &keys[1]);
            let _ = c.put(meta_key(b"smoke/ok"), b"1");
            println!("write ok");
        }
        "verify" => {
            let c = StoreCluster::open_lab_direct(&dir, 3, 3).expect("reopen");
            // Prefer stored key bytes from first process.
            let k0 = c
                .get_on(1, &meta_key(b"smoke/k0"))
                .expect("get")
                .expect("k0 meta");
            let k1 = c
                .get_on(1, &meta_key(b"smoke/k1"))
                .expect("get")
                .expect("k1 meta");
            let v0 = c.get_on(1, &k0).expect("get").expect("val0");
            let v1 = c.get_on(1, &k1).expect("get").expect("val1");
            assert_eq!(v0.as_ref(), b"mp-a");
            assert_eq!(v1.as_ref(), b"mp-b");
            // Majority visibility after reopen elect is not required for all peers
            // if they already applied before exit; check at least node 1 + one more.
            let mut seen = 0u32;
            for nid in 1..=3u64 {
                if c.get_on(nid, &k0).ok().flatten().as_deref() == Some(b"mp-a".as_ref()) {
                    seen += 1;
                }
            }
            assert!(seen >= 2, "expected majority still hold mp-a, seen={seen}");
            println!("verify ok");
        }
        "multiwrite" => {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 4).expect("open");
            c.elect_all(140).expect("elect");
            let keys: Vec<Vec<u8>> = c
                .range_metas()
                .iter()
                .map(|r| {
                    if r.start.is_empty() {
                        vec![0x00, b'm']
                    } else {
                        let mut k = r.start.clone();
                        k.push(b'm');
                        k
                    }
                })
                .collect();
            let mut ranges = 0u32;
            for (i, k) in keys.iter().enumerate() {
                let rid = c.put_routed(k, [b'X', i as u8]).expect("put_routed");
                let _ = rid;
                ranges += 1;
                assert_eq!(
                    c.get_strong(k).unwrap().as_deref(),
                    Some([b'X', i as u8].as_slice())
                );
                assert_eq!(
                    c.get_fast_replica(k).unwrap().as_deref(),
                    Some([b'X', i as u8].as_slice())
                );
            }
            assert!(ranges >= 3, "need multi-range multiwrite");
            println!("multiwrite ok ranges={ranges}");
        }
        "partition" => {
            // A: elect + put + minority partition + heal (3 nodes, one process; durable dirs).
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).expect("open");
            c.elect_all(120).expect("elect");
            c.put(b"p-ok", b"before").expect("put before partition");
            assert!(c.count_applied_eq(b"p-ok", b"before") >= 2);
            let rid = c.locate(b"p-ok").unwrap();
            let leader = c.range_leader(rid).expect("leader");
            let ids: Vec<u64> = c.node_ids().to_vec();
            for &nid in &ids {
                if nid != leader {
                    c.set_participating(nid, false).unwrap();
                }
            }
            let err = c.put(b"p-block", b"should-fail");
            assert!(
                matches!(err, Err(pedradb_store::StoreError::NotCommitted { .. })),
                "minority must NotCommitted: {err:?}"
            );
            for &nid in &ids {
                c.set_participating(nid, true).unwrap();
            }
            for _ in 0..40 {
                c.tick().unwrap();
            }
            c.put(b"p-after", b"healed").expect("put after heal");
            assert!(c.count_applied_eq(b"p-after", b"healed") >= 2);
            let _ = c.put(meta_key(b"partition/ok"), b"1");
            println!("partition ok leader={leader}");
        }
        "canaries" => {
            // B+C: lease + index + journal pins on multi-node, durable for canaries-verify.
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).expect("open");
            c.elect_all(120).expect("elect");
            // B: lease IF NOT EXISTS
            c.dcs_create(b"lease/canary", b"holder-a")
                .expect("lease acquire");
            assert!(c.dcs_create(b"lease/canary", b"holder-b").is_err());
            // C: index batch W3
            c.put_batch([
                (b"row/7".as_slice(), b"body".as_slice()),
                (b"idx/name/x".as_slice(), b"7".as_slice()),
                (b"idx/email/y".as_slice(), b"7".as_slice()),
            ])
            .expect("index batch");
            // C: journal-like seq W4
            for i in 0..8u8 {
                c.put([b'j', i], [b'v', i]).expect("journal put");
            }
            let _ = c.put(meta_key(b"canaries/ok"), b"1");
            println!("canaries ok");
        }
        "canaries-verify" => {
            // Second OS process: reopen, no double-hold, full index, journal keys.
            let c = StoreCluster::open_lab_direct(&dir, 3, 1).expect("reopen");
            let lease = c
                .dcs_get_on(1, b"lease/canary")
                .expect("dcs")
                .expect("lease present");
            assert_eq!(lease.value.as_slice(), b"holder-a");
            // Still exclusive after reopen.
            // (Cannot dcs_create again without mut — check majority still holder-a only.)
            let n = (1..=3u64)
                .filter(|&nid| {
                    c.dcs_get_on(nid, b"lease/canary")
                        .ok()
                        .flatten()
                        .is_some_and(|kv| kv.value == b"holder-a")
                })
                .count();
            assert!(n >= 2, "lease majority after reopen n={n}");
            assert_eq!(
                c.get_on(1, b"row/7").unwrap().as_deref(),
                Some(b"body".as_ref())
            );
            assert_eq!(
                c.get_on(1, b"idx/name/x").unwrap().as_deref(),
                Some(b"7".as_ref())
            );
            assert_eq!(
                c.get_on(1, b"idx/email/y").unwrap().as_deref(),
                Some(b"7".as_ref())
            );
            for i in 0..8u8 {
                assert_eq!(
                    c.get_on(1, &[b'j', i]).unwrap().as_deref(),
                    Some([b'v', i].as_slice())
                );
            }
            println!("canaries-verify ok");
        }
        // RFC-0017 P2.3: DCS layer + one app-shaped batch **only** on multi-process StoreCluster.
        "dcs-layer" => {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).expect("open");
            c.elect_all(120).expect("elect");
            // DCS layer (not a second consensus product).
            let rev = c
                .dcs_create(b"dcs/layer/lock", b"owner-1")
                .expect("dcs_create");
            assert!(rev >= 1);
            assert!(
                c.dcs_create(b"dcs/layer/lock", b"owner-2").is_err(),
                "exclusive create"
            );
            let rev2 = c
                .dcs_cas(b"dcs/layer/lock", b"owner-1b", rev)
                .expect("dcs_cas");
            assert!(rev2 > rev);
            // One app layer: index-style multi-key batch on the same store.
            c.put_batch([
                (b"app/row/1".as_slice(), b"body".as_slice()),
                (b"app/idx/1".as_slice(), b"1".as_slice()),
            ])
            .expect("app batch");
            let n_lease = (1..=3u64)
                .filter(|&nid| {
                    c.dcs_get_on(nid, b"dcs/layer/lock")
                        .ok()
                        .flatten()
                        .is_some_and(|kv| kv.value == b"owner-1b")
                })
                .count();
            assert!(n_lease >= 2, "dcs majority n={n_lease}");
            let _ = c.put(meta_key(b"dcs-layer/ok"), b"1");
            let _ = c.put(meta_key(b"dcs-layer/rev"), rev2.to_string().as_bytes());
            println!("dcs-layer ok rev={rev2}");
        }
        "dcs-layer-verify" => {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).expect("reopen");
            c.elect_all(80).expect("elect");
            let kv = c
                .dcs_get_on(1, b"dcs/layer/lock")
                .expect("dcs")
                .expect("lock present");
            assert_eq!(kv.value.as_slice(), b"owner-1b");
            let n = (1..=3u64)
                .filter(|&nid| {
                    c.dcs_get_on(nid, b"dcs/layer/lock")
                        .ok()
                        .flatten()
                        .is_some_and(|kv| kv.value == b"owner-1b")
                })
                .count();
            assert!(n >= 2, "dcs majority after reopen n={n}");
            // Wrong CAS must fail (layer still exclusive).
            assert!(
                c.dcs_cas(b"dcs/layer/lock", b"hijack", 1).is_err(),
                "stale cas must fail"
            );
            assert_eq!(
                c.get_on(1, b"app/row/1").unwrap().as_deref(),
                Some(b"body".as_ref())
            );
            assert_eq!(
                c.get_on(1, b"app/idx/1").unwrap().as_deref(),
                Some(b"1".as_ref())
            );
            println!("dcs-layer-verify ok");
        }
        // RFC-0022 P0.4: etcd-need face only via StoreCluster multiproc (no external etcd).
        "etcd-need" => {
            use pedradb_store::EtcdNeedFace;
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).expect("open");
            c.elect_all(120).expect("elect");
            let (_id, rx) = c.watch_prefix(EtcdNeedFace::PREFIX);
            let rev = EtcdNeedFace::create(&mut c, b"lock/pg1", b"node-a").expect("create");
            assert!(rev >= 1);
            assert!(
                EtcdNeedFace::create(&mut c, b"lock/pg1", b"node-b").is_err(),
                "exclusive"
            );
            let rev2 = EtcdNeedFace::cas(&mut c, b"lock/pg1", b"node-a2", rev).expect("cas");
            assert!(rev2 > rev);
            let kv = EtcdNeedFace::get(&c, b"lock/pg1")
                .expect("get")
                .expect("kv");
            assert_eq!(kv.value.as_slice(), b"node-a2");
            let mut watched = 0u32;
            while rx.try_recv().is_ok() {
                watched += 1;
            }
            assert!(watched >= 1, "watch after majority, watched={watched}");
            // Snapshot TX concurrent path still works on same store (no dual SoR).
            let mut tx = c.begin();
            tx.set(b"app/cfg", b"1").expect("set");
            tx.commit(&mut c).expect("snapshot commit");
            let _ = c.put(meta_key(b"etcd-need/ok"), b"1");
            let _ = c.put(meta_key(b"etcd-need/rev"), rev2.to_string().as_bytes());
            println!("etcd-need ok rev={rev2} watched={watched}");
        }
        "etcd-need-verify" => {
            use pedradb_store::EtcdNeedFace;
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).expect("reopen");
            c.elect_all(80).expect("elect");
            let kv = EtcdNeedFace::get(&c, b"lock/pg1")
                .expect("get")
                .expect("lock present after reopen");
            assert_eq!(kv.value.as_slice(), b"node-a2");
            let n = (1..=3u64)
                .filter(|&nid| {
                    EtcdNeedFace::get_on(&c, nid, b"lock/pg1")
                        .ok()
                        .flatten()
                        .is_some_and(|kv| kv.value == b"node-a2")
                })
                .count();
            assert!(n >= 2, "etcd-need majority after reopen n={n}");
            assert_eq!(
                c.get_on(1, b"app/cfg").unwrap().as_deref(),
                Some(b"1".as_ref())
            );
            println!("etcd-need-verify ok");
        }
        other => panic!("unknown cmd {other}"),
    }
}

/// Minimal hex without extra deps.
mod hex {
    pub fn encode_upper_loose(bytes: &[u8]) -> String {
        const H: &[u8; 16] = b"0123456789ABCDEF";
        let mut s = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            s.push(H[(b >> 4) as usize] as char);
            s.push(H[(b & 0xf) as usize] as char);
        }
        s
    }
}
