//! Mini-bindingtester soak as CI gates (FDB-layer substitution confidence).
//!
//! E1-style: in-process model-checked random ops + multi-key + WW.
//! E2-style: multi-thread TCP clients with partitioned keys + majority verify.
//!
//! Not full FoundationDB bindingtester.

use pedradb_store::{
    client_get, client_status, client_tick, StoreCluster, StoreOpenOptions, TcpClusterClient,
};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Lab flag parity with scale-gate / fdb-bench / montanha-tcp.
fn write_backpressure_enabled() -> bool {
    std::env::var("MONTANHA_WRITE_BACKPRESSURE").ok().as_deref() == Some("1")
}

fn store_opts() -> StoreOpenOptions {
    let mut opts = StoreOpenOptions::default();
    if write_backpressure_enabled() {
        opts = opts.with_write_backpressure();
    }
    opts
}

fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let i = N.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("montanha-minib-{tag}-{n}-{i}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn xorshift(rng: &mut u64) -> u64 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    *rng
}

/// CI gate: random soak must not silent-wrong vs model; WW exactly-one-winner.
#[test]
fn mini_bt_inprocess_model_checked_soak() {
    let dir = temp_dir("e1");
    let mut c = StoreCluster::open_with_options(&dir, 3, 1, store_opts()).unwrap();
    c.elect_all(100).unwrap();

    let mut model: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    let mut rng = 0x000F_DBC1_u64;
    // Keep CI under ~1–2 min wall on laptop debug builds.
    let ops = 80usize;
    let mut mismatches = 0u64;
    let mut multi_ok = 0u64;
    let mut ww_ok = 0u64;

    for _ in 0..ops {
        let op = xorshift(&mut rng) % 7;
        let k = format!("mb/{}", xorshift(&mut rng) % 24).into_bytes();
        match op {
            0 | 1 => {
                let v = format!("v{}", xorshift(&mut rng) % 500).into_bytes();
                let mut tr = c.begin();
                tr.set(&k, &v).unwrap();
                if tr.commit(&mut c).is_ok() {
                    model.insert(k, v);
                }
            }
            2 => {
                let mut tr = c.begin();
                tr.clear(&k).unwrap();
                if tr.commit(&mut c).is_ok() {
                    model.remove(&k);
                }
            }
            3 => {
                let mut tr = c.begin();
                let got = tr.get(&c, &k).unwrap();
                let exp = model.get(&k).cloned();
                if got != exp {
                    mismatches += 1;
                }
            }
            4 => {
                let k2 = format!("mb/{}", xorshift(&mut rng) % 24).into_bytes();
                let v1 = format!("m{}", xorshift(&mut rng) % 500).into_bytes();
                let v2 = format!("m{}", xorshift(&mut rng) % 500).into_bytes();
                let mut tr = c.begin();
                tr.set(&k, &v1).unwrap();
                tr.set(&k2, &v2).unwrap();
                if tr.commit(&mut c).is_ok() {
                    model.insert(k, v1);
                    model.insert(k2, v2);
                    multi_ok += 1;
                }
            }
            5 => {
                let mut t1 = c.begin();
                let mut t2 = c.begin();
                let _ = t1.get(&c, &k);
                let _ = t2.get(&c, &k);
                let va = format!("a{}", xorshift(&mut rng) % 50).into_bytes();
                let vb = format!("b{}", xorshift(&mut rng) % 50).into_bytes();
                t1.set(&k, &va).unwrap();
                t2.set(&k, &vb).unwrap();
                let r1 = t1.commit(&mut c);
                let r2 = t2.commit(&mut c);
                let wins = r1.is_ok() as u8 + r2.is_ok() as u8;
                assert_eq!(wins, 1, "WW must elect exactly one winner");
                ww_ok += 1;
                if r1.is_ok() {
                    model.insert(k, va);
                } else {
                    model.insert(k, vb);
                }
            }
            _ => {
                let mut tr = c.begin();
                let pairs = tr.get_range(&c, b"mb/", b"mb0").unwrap();
                for (pk, pv) in &pairs {
                    if let Some(ev) = model.get(pk) {
                        if ev != pv {
                            mismatches += 1;
                        }
                    }
                }
                for (mk, mv) in &model {
                    if mk.starts_with(b"mb/") && !pairs.iter().any(|(k, v)| k == mk && v == mv) {
                        mismatches += 1;
                    }
                }
            }
        }
    }

    assert_eq!(mismatches, 0, "silent-wrong vs model");
    assert!(multi_ok >= 5, "expected multi-key commits, got {multi_ok}");
    assert!(ww_ok >= 5, "expected WW pairs, got {ww_ok}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── TCP multi-client (E2) ─────────────────────────────────────────────────

fn montanha_tcp_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_montanha-tcp") {
        return PathBuf::from(p);
    }
    panic!("CARGO_BIN_EXE_montanha-tcp not set");
}

struct TcpNode {
    child: Child,
    addr: SocketAddr,
    id: u64,
}

impl Drop for TcpNode {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start_tcp_cluster(bin: &Path, tmp: &Path) -> Vec<TcpNode> {
    let ports: Vec<u16> = (0..3).map(|_| free_port()).collect();
    let peers: Vec<(u64, SocketAddr)> = ports
        .iter()
        .enumerate()
        .map(|(i, &p)| ((i as u64) + 1, format!("127.0.0.1:{p}").parse().unwrap()))
        .collect();
    let peer_flags: Vec<String> = peers
        .iter()
        .flat_map(|(id, a)| vec!["--peer".into(), format!("{id}={a}")])
        .collect();
    let mut nodes = Vec::new();
    for (id, addr) in &peers {
        let data = tmp.join(format!("n{id}"));
        std::fs::create_dir_all(&data).unwrap();
        let mut cmd = Command::new(bin);
        cmd.arg("node")
            .arg("--id")
            .arg(id.to_string())
            .arg("--data")
            .arg(&data)
            .arg("--bind")
            .arg(addr.to_string())
            .arg("--ranges")
            .arg("1")
            .args(&peer_flags)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Explicit flag (env also works via montanha-tcp); keep soak independent of inherit quirks.
        if write_backpressure_enabled() {
            cmd.arg("--write-backpressure");
        }
        let child = cmd.spawn().expect("spawn montanha-tcp");
        nodes.push(TcpNode {
            child,
            addr: *addr,
            id: *id,
        });
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    for n in &nodes {
        loop {
            if client_status(n.addr.to_string()).is_ok() {
                break;
            }
            assert!(Instant::now() < deadline, "node {} not up", n.id);
            thread::sleep(Duration::from_millis(40));
        }
    }
    let peer_flags: Vec<String> = nodes
        .iter()
        .flat_map(|n| vec!["--peer".into(), format!("{}={}", n.id, n.addr)])
        .collect();
    let _ = Command::new(bin)
        .arg("elect-wait")
        .args(&peer_flags)
        .status();
    for _ in 0..60 {
        for n in &nodes {
            let _ = client_tick(n.addr.to_string(), 2);
        }
        thread::sleep(Duration::from_millis(15));
    }
    nodes
}

/// CI gate: concurrent TCP writers (partitioned keys) + majority visibility.
#[test]
fn mini_bt_tcp_multiclient_majority_verify() {
    let bin = montanha_tcp_bin();
    let dir = temp_dir("e2");
    let nodes = start_tcp_cluster(&bin, &dir);
    let peers: Vec<(u64, String)> = nodes.iter().map(|n| (n.id, n.addr.to_string())).collect();
    let addrs: Vec<String> = nodes.iter().map(|n| n.addr.to_string()).collect();
    let n_threads = 3usize;
    let per = 6usize;
    let peers_a = Arc::new(peers);
    let mut handles = Vec::new();

    for tid in 0..n_threads {
        let peers = (*peers_a).clone();
        handles.push(thread::spawn(move || {
            let mut cli = TcpClusterClient::new(peers).with_max_attempts(64);
            let mut local: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
            let mut rng = 0xC0FFEE_u64 ^ (tid as u64).wrapping_mul(0x9E37);
            for i in 0..per {
                let slot = xorshift(&mut rng) % 12;
                let k = format!("e2/{tid}/{slot}").into_bytes();
                let op = xorshift(&mut rng) % 2;
                if op == 0 {
                    let v = format!("t{tid}-{i}").into_bytes();
                    for _ in 0..12 {
                        if cli.put(&k, &v).is_ok() {
                            local.insert(k.clone(), v.clone());
                            break;
                        }
                        thread::sleep(Duration::from_millis(20));
                    }
                } else {
                    let k2 = format!("e2/{tid}/{}", (slot + 1) % 12).into_bytes();
                    let v1 = format!("a{i}").into_bytes();
                    let v2 = format!("b{i}").into_bytes();
                    let pairs = vec![(k.clone(), v1.clone()), (k2.clone(), v2.clone())];
                    for _ in 0..14 {
                        if cli.commit_tx(&pairs).is_ok() {
                            local.insert(k.clone(), v1.clone());
                            local.insert(k2, v2);
                            break;
                        }
                        thread::sleep(Duration::from_millis(25));
                    }
                }
            }
            local
        }));
    }

    let mut merged = BTreeMap::new();
    for h in handles {
        for (k, v) in h.join().unwrap() {
            merged.insert(k, v);
        }
    }
    assert!(!merged.is_empty(), "writers produced no keys");

    let mut missing = 0u64;
    for (k, v) in &merged {
        let mut seen = 0u32;
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline && seen < 2 {
            seen = 0;
            for a in &addrs {
                if client_get(a, k).ok().flatten().as_deref() == Some(v.as_slice()) {
                    seen += 1;
                }
            }
            if seen < 2 {
                thread::sleep(Duration::from_millis(40));
            }
        }
        if seen < 2 {
            missing += 1;
        }
    }
    assert_eq!(
        missing,
        0,
        "majority visibility failed for {missing} keys (model {})",
        merged.len()
    );
    drop(nodes);
    let _ = std::fs::remove_dir_all(&dir);
}
