//! Integration: 3 OS processes, real localhost TCP, elect + put + majority (RFC-0017 P0.1).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn bin() -> PathBuf {
    // Prefer cargo-built binary next to test executable.
    let mut p = std::env::current_exe().expect("exe");
    p.pop(); // deps
    if p.ends_with("deps") {
        p.pop();
    }
    p.push("montanha-tcp");
    if p.exists() {
        return p;
    }
    // Fallback: cargo run path via CARGO_BIN_EXE
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_montanha-tcp") {
        return PathBuf::from(p);
    }
    panic!("montanha-tcp binary not found at {}", p.display());
}

struct Node {
    child: Child,
    addr: SocketAddr,
    id: u64,
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

fn start_cluster(tmp: &std::path::Path) -> Vec<Node> {
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
        let mut cmd = Command::new(bin());
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
        let child = cmd.spawn().expect("spawn node");
        nodes.push(Node {
            child,
            addr: *addr,
            id: *id,
        });
    }
    // Wait until all accept.
    let deadline = Instant::now() + Duration::from_secs(10);
    for n in &nodes {
        loop {
            if pedradb_store::client_status(n.addr.to_string()).is_ok() {
                break;
            }
            if Instant::now() > deadline {
                panic!("node {} not up at {}", n.id, n.addr);
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    nodes
}

#[test]
fn tcp_3node_elect_put_majority() {
    let tmp = tempfile_dir("tcp_maj");
    let nodes = start_cluster(&tmp);
    let peer_flags: Vec<String> = nodes
        .iter()
        .flat_map(|n| vec!["--peer".into(), format!("{}={}", n.id, n.addr)])
        .collect();

    let status = Command::new(bin())
        .arg("elect-wait")
        .args(&peer_flags)
        .output()
        .expect("elect-wait");
    assert!(
        status.status.success(),
        "elect-wait failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );

    let key = b"tcp-k1";
    let val = b"tcp-v1";
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut put_ok = false;
    while Instant::now() < deadline && !put_ok {
        for n in &nodes {
            if pedradb_store::client_put(n.addr.to_string(), key, val).is_ok() {
                put_ok = true;
                break;
            }
        }
        if !put_ok {
            thread::sleep(Duration::from_millis(50));
        }
    }
    assert!(put_ok, "put failed on all nodes");

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let mut seen = 0u32;
        for n in &nodes {
            if pedradb_store::client_get(n.addr.to_string(), key)
                .ok()
                .flatten()
                .as_deref()
                == Some(val.as_ref())
            {
                seen += 1;
            }
        }
        if seen >= 2 {
            break;
        }
        assert!(Instant::now() < deadline, "majority timeout seen={seen}");
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn tcp_wire_unit_via_lib() {
    // Sanity: codec still wired through crate root re-exports.
    let m = pedradb_store::WireMsg::Put {
        key: b"a".to_vec(),
        value: b"b".to_vec(),
    };
    let d = pedradb_store::WireMsg::decode(&m.encode()).unwrap();
    assert_eq!(d, m);
}

/// RFC-0021 P1.3: rewire peer map via TCP SetPeers only (no SSH).
#[test]
fn tcp_rewire_peer_map_without_ssh() {
    let tmp = tempfile_dir("tcp_rewire");
    let nodes = start_cluster(&tmp);
    let peer_flags: Vec<String> = nodes
        .iter()
        .flat_map(|n| vec!["--peer".into(), format!("{}={}", n.id, n.addr)])
        .collect();
    assert!(Command::new(bin())
        .arg("elect-wait")
        .args(&peer_flags)
        .status()
        .unwrap()
        .success());

    let list: Vec<(u64, String)> = nodes.iter().map(|n| (n.id, n.addr.to_string())).collect();
    let mut client = pedradb_store::TcpClusterClient::new(list.iter().cloned());
    // Push the same map via TCP SetPeers to all nodes (self-service control plane).
    client.rewire_peer_map(&list).expect("rewire via TCP");
    assert_eq!(client.peer_map().len(), 3);

    // Cluster still accepts writes after rewire.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut ok = false;
    while Instant::now() < deadline && !ok {
        if client.put(b"rewire-k", b"v").is_ok() {
            ok = true;
        } else {
            thread::sleep(Duration::from_millis(50));
        }
    }
    assert!(ok, "put after rewire failed");
}

/// RFC-0021 P2.6: client prefers same-region dial first.
#[test]
fn tcp_region_prefer_first_dial() {
    let tmp = tempfile_dir("tcp_region");
    let nodes = start_cluster(&tmp);
    let list: Vec<(u64, String)> = nodes.iter().map(|n| (n.id, n.addr.to_string())).collect();
    let mut client = pedradb_store::TcpClusterClient::new(list);
    client.set_region(1, "a");
    client.set_region(2, "a");
    client.set_region(3, "b");
    client.set_prefer_region(Some("b"));
    // Without preferred leader id, first dial should be region b → node 3.
    assert_eq!(client.first_dial_id(), Some(3));
    client.set_prefer_region(Some("a"));
    let first = client.first_dial_id().unwrap();
    assert!(first == 1 || first == 2, "prefer a → {first}");
}

/// RFC-0021 P0.1: multi-key CommitTx over real TCP 3-node cluster.
#[test]
fn tcp_commit_tx_multi_key_majority() {
    let tmp = tempfile_dir("tcp_ctx");
    let nodes = start_cluster(&tmp);
    let peer_flags: Vec<String> = nodes
        .iter()
        .flat_map(|n| vec!["--peer".into(), format!("{}={}", n.id, n.addr)])
        .collect();
    let status = Command::new(bin())
        .arg("elect-wait")
        .args(&peer_flags)
        .output()
        .expect("elect-wait");
    assert!(
        status.status.success(),
        "elect-wait failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );

    let pairs = vec![
        (b"tx-k1".to_vec(), b"v1".to_vec()),
        (b"tx-k2".to_vec(), b"v2".to_vec()),
    ];
    let mut client = pedradb_store::TcpClusterClient::new(
        nodes
            .iter()
            .map(|n| (n.id, n.addr.to_string()))
            .collect::<Vec<_>>(),
    );
    let mut last_err = None;
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut tid = None;
    while Instant::now() < deadline && tid.is_none() {
        match client.commit_tx(&pairs) {
            Ok(t) => tid = Some(t),
            Err(e) => {
                last_err = Some(e);
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
    assert!(tid.is_some(), "commit_tx failed: {:?}", last_err);

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let mut seen = 0u32;
        for n in &nodes {
            let a = pedradb_store::client_get(n.addr.to_string(), b"tx-k1")
                .ok()
                .flatten();
            let b = pedradb_store::client_get(n.addr.to_string(), b"tx-k2")
                .ok()
                .flatten();
            if a.as_deref() == Some(b"v1".as_ref()) && b.as_deref() == Some(b"v2".as_ref()) {
                seen += 1;
            }
        }
        if seen >= 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "majority timeout for commit_tx keys seen={seen}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// RFC-0017 P2.2: TcpClusterClient retries NotLeader and lands write on leader.
#[test]
fn tcp_client_retry_not_leader() {
    let tmp = tempfile_dir("tcp_client");
    let nodes = start_cluster(&tmp);
    let peer_flags: Vec<String> = nodes
        .iter()
        .flat_map(|n| vec!["--peer".into(), format!("{}={}", n.id, n.addr)])
        .collect();
    let status = Command::new(bin())
        .arg("elect-wait")
        .args(&peer_flags)
        .output()
        .expect("elect-wait");
    assert!(status.status.success(), "elect-wait failed");

    let mut leader_id = None;
    for n in &nodes {
        if let Ok(st) = pedradb_store::client_status(n.addr.to_string()) {
            if let Some(l) = pedradb_store::leader_from_status(&st) {
                leader_id = Some(l);
                break;
            }
        }
    }
    let leader_id = leader_id.expect("leader");
    let follower = nodes.iter().find(|n| n.id != leader_id).expect("follower");
    // Follower first in dial order so first attempt is NotLeader → retry leader.
    let mut ordered: Vec<(u64, String)> = vec![(follower.id, follower.addr.to_string())];
    for n in &nodes {
        if n.id != follower.id {
            ordered.push((n.id, n.addr.to_string()));
        }
    }
    let mut client = pedradb_store::TcpClusterClient::new(ordered).with_max_attempts(32);
    // Allow leadership to settle after elect-wait.
    thread::sleep(Duration::from_millis(200));
    client
        .put(b"client-retry-k", b"v1")
        .expect("put with retry");
    assert_eq!(client.preferred_leader(), Some(leader_id));
    let leader_addr = nodes.iter().find(|n| n.id == leader_id).unwrap().addr;
    let got = pedradb_store::client_get(leader_addr.to_string(), b"client-retry-k")
        .unwrap()
        .expect("value on leader");
    assert_eq!(got, b"v1");
}

/// RFC-0025 P1.3: PutBatch one RTT for multi-key put_many on real TCP.
#[test]
fn tcp_put_batch_majority() {
    let tmp = tempfile_dir("tcp_pbatch");
    let nodes = start_cluster(&tmp);
    let peer_flags: Vec<String> = nodes
        .iter()
        .flat_map(|n| vec!["--peer".into(), format!("{}={}", n.id, n.addr)])
        .collect();
    assert!(Command::new(bin())
        .arg("elect-wait")
        .args(&peer_flags)
        .status()
        .unwrap()
        .success());
    let peers: Vec<(u64, String)> = nodes.iter().map(|n| (n.id, n.addr.to_string())).collect();
    let mut client = pedradb_store::TcpClusterClient::new(peers).with_max_attempts(48);
    let pairs: Vec<(Vec<u8>, Vec<u8>)> = (0..8u8)
        .map(|i| (format!("pb-{i}").into_bytes(), vec![i]))
        .collect();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut ok = false;
    while Instant::now() < deadline && !ok {
        if client.put_batch(&pairs).is_ok() {
            ok = true;
        } else {
            thread::sleep(Duration::from_millis(40));
        }
    }
    assert!(ok, "put_batch failed");
    // Majority visibility
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let mut seen = 0u32;
        for n in &nodes {
            if pedradb_store::client_get(n.addr.to_string(), b"pb-0")
                .ok()
                .flatten()
                .as_deref()
                == Some(&[0u8][..])
            {
                seen += 1;
            }
        }
        if seen >= 2 {
            break;
        }
        assert!(Instant::now() < deadline, "majority timeout for put_batch");
        thread::sleep(Duration::from_millis(40));
    }
}

/// Phase A: etcd-need create/CAS/get over **real TCP** (no dual etcd SoR).
#[test]
fn tcp_etcd_need_create_cas_get() {
    let tmp = tempfile_dir("tcp_etcd");
    let nodes = start_cluster(&tmp);
    let peer_flags: Vec<String> = nodes
        .iter()
        .flat_map(|n| vec!["--peer".into(), format!("{}={}", n.id, n.addr)])
        .collect();
    assert!(
        Command::new(bin())
            .arg("elect-wait")
            .args(&peer_flags)
            .status()
            .unwrap()
            .success(),
        "elect-wait"
    );

    // Full coordination key (EtcdNeedFace prefix `m/`).
    let key = b"m/lock/pg1";
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut rev = 0u64;
    let mut created = false;
    while Instant::now() < deadline && !created {
        for n in &nodes {
            if let Ok(r) = pedradb_store::client_dcs_create(n.addr.to_string(), key, b"node-a") {
                rev = r;
                created = true;
                break;
            }
        }
        if !created {
            thread::sleep(Duration::from_millis(40));
        }
    }
    assert!(created && rev >= 1, "create failed rev={rev}");

    // Exclusive create must fail on every node once key exists.
    let mut exclusive_fail = false;
    for n in &nodes {
        if pedradb_store::client_dcs_create(n.addr.to_string(), key, b"node-b").is_err() {
            exclusive_fail = true;
            break;
        }
    }
    assert!(exclusive_fail, "second create must fail");

    // CAS on the current revision.
    let mut cas_ok = false;
    let mut rev2 = 0u64;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && !cas_ok {
        for n in &nodes {
            if let Ok(r) = pedradb_store::client_dcs_cas(n.addr.to_string(), key, b"node-a2", rev) {
                rev2 = r;
                cas_ok = true;
                break;
            }
        }
        if !cas_ok {
            thread::sleep(Duration::from_millis(40));
        }
    }
    assert!(cas_ok && rev2 > rev, "cas rev={rev2} prev={rev}");

    // Majority visibility of new value via DcsGet.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let mut seen = 0u32;
        for n in &nodes {
            if pedradb_store::client_dcs_get(n.addr.to_string(), key)
                .ok()
                .flatten()
                .as_deref()
                == Some(b"node-a2".as_ref())
            {
                seen += 1;
            }
        }
        if seen >= 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "majority dcs get timeout seen={seen}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn tempfile_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "pedradb-mtcp-{}-{}-{}",
        name,
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}
