//! Multi-process smoke for Montanha-Store multi-key TX durability.
//!
//! Usage:
//!   montanha-store-smoke write  <parent_dir>
//!   montanha-store-smoke verify <parent_dir>
//!
//! `write` elects a 3-node / 3-range cluster, runs cross-range `commit_tx`, exits.
//! `verify` reopens the same directories and checks committed keys (second OS process).

#![forbid(unsafe_code)]

use pedradb_store::{meta_key, StoreCluster};

fn main() {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().expect("cmd: write|verify");
    let dir = args.next().expect("parent_dir");
    match cmd.as_str() {
        "write" => {
            let mut c = StoreCluster::open(&dir, 3, 3).expect("open");
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
            let c = StoreCluster::open(&dir, 3, 3).expect("reopen");
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
