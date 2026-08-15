//! Multi-host Montanha-Store node over TCP (RFC-0017 P0.1).
//!
//! # Node
//! ```text
//! montanha-tcp node --id 1 --data /data --bind 0.0.0.0:9701 \
//!   --peer 1=127.0.0.1:9701 --peer 2=127.0.0.1:9702 --peer 3=127.0.0.1:9703 \
//!   [--health 0.0.0.0:9780]
//! ```
//!
//! HTTP health (proxy / LB, soviet-style role checks):
//! - `GET /ready`   → 200 if worker answers status
//! - `GET /leader`  → 200 if this node is range-1 leader, else 503
//! - `GET /follower`→ 200 if ready and not leader, else 503
//! - `GET /status`  → 200 + status_text body
//!
//! # Client
//! ```text
//! montanha-tcp put  --addr 127.0.0.1:9701 --key hello --value world
//! montanha-tcp get  --addr 127.0.0.1:9701 --key hello
//! montanha-tcp status --addr 127.0.0.1:9701
//! montanha-tcp smoke --peer 1=... --peer 2=... --peer 3=...
//! ```
//!
//! The store cluster is **single-threaded** (SeedRng is `!Send`). Accept threads
//! only forward wire messages to the worker via `mpsc`.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::env;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use pedradb_store::{
    client_get, client_put, client_set_peers, client_status, client_tick, read_frame,
    resolve_host_port, write_frame, StoreCluster, StoreError, WireMsg,
};

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!(
            "usage: montanha-tcp <node|put|get|status|tick|smoke|elect-wait|set-peers|proxy> [flags]\n\
             node: --id N --data DIR --bind ADDR --peer id=addr... [--ranges N] [--health ADDR]\n\
             put:  --addr HOST:PORT --key K --value V [--peer id=addr...]\n\
             get:  --addr HOST:PORT --key K\n\
             status/tick: --addr HOST:PORT [--n N]\n\
             smoke: --peer id=addr... (elect + put + get on real TCP cluster)\n\
             proxy: --listen ADDR --mode write|read|any --member host:dataPort[@healthPort]...\n\
             health HTTP: GET /ready /leader /follower /status on --health (default bind_port+79)"
        );
        process::exit(2);
    }
    let cmd = args.remove(0);
    match cmd.as_str() {
        "node" => cmd_node(&args),
        "put" => cmd_put(&args),
        "get" => cmd_get(&args),
        "status" => cmd_status(&args),
        "tick" => cmd_tick(&args),
        "smoke" => cmd_smoke(&args),
        "elect-wait" => cmd_elect_wait(&args),
        "set-peers" => cmd_set_peers(&args),
        "proxy" => cmd_proxy(&args),
        other => {
            eprintln!("unknown command {other}");
            process::exit(2);
        }
    }
}

/// Peer target as `host:port` string (resolved on each dial — supports DNS).
fn parse_peer(s: &str) -> (u64, String) {
    let (a, b) = s.split_once('=').unwrap_or_else(|| {
        eprintln!("peer must be id=host:port, got {s}");
        process::exit(2);
    });
    let id = a.parse().unwrap_or_else(|_| {
        eprintln!("bad peer id {a}");
        process::exit(2);
    });
    // Accept literal SocketAddr or hostname:port.
    if b.parse::<SocketAddr>().is_err() && !b.contains(':') {
        eprintln!("bad peer addr {b} (want host:port)");
        process::exit(2);
    }
    (id, b.to_string())
}

fn flag_val(args: &[String], name: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            return args.get(i + 1).cloned();
        }
        i += 1;
    }
    None
}

fn flag_peers(args: &[String]) -> HashMap<u64, String> {
    let mut peers = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--peer" {
            if let Some(v) = args.get(i + 1) {
                let (id, addr) = parse_peer(v);
                peers.insert(id, addr);
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    peers
}

/// Work item for the single-threaded raft worker.
enum Work {
    /// Inbound peer RPC (fire-and-forget reply on same conn is handled by accept).
    Peer {
        from: u64,
        to: u64,
        body: Vec<u8>,
        resp: SyncSender<Result<(), String>>,
    },
    Put {
        key: Vec<u8>,
        value: Vec<u8>,
        resp: SyncSender<Result<(), String>>,
    },
    PutBatch {
        pairs: Vec<(Vec<u8>, Vec<u8>)>,
        resp: SyncSender<Result<(), String>>,
    },
    Get {
        key: Vec<u8>,
        resp: SyncSender<Result<Option<Vec<u8>>, String>>,
    },
    Tick {
        resp: SyncSender<Result<(), String>>,
    },
    Status {
        resp: SyncSender<Result<String, String>>,
    },
    ClusterJson {
        resp: SyncSender<Result<String, String>>,
    },
    SetPeers {
        peers: Vec<(u64, String)>,
        resp: SyncSender<Result<(), String>>,
    },
    CommitTx {
        pairs: Vec<(Vec<u8>, Vec<u8>)>,
        resp: SyncSender<Result<u64, String>>,
    },
    DcsCreate {
        key: Vec<u8>,
        value: Vec<u8>,
        resp: SyncSender<Result<u64, String>>,
    },
    DcsCas {
        key: Vec<u8>,
        value: Vec<u8>,
        expected_rev: u64,
        resp: SyncSender<Result<u64, String>>,
    },
    DcsGet {
        key: Vec<u8>,
        resp: SyncSender<Result<Option<Vec<u8>>, String>>,
    },
}

fn cmd_node(args: &[String]) {
    let id: u64 = flag_val(args, "--id")
        .expect("--id")
        .parse()
        .expect("id");
    let data = PathBuf::from(flag_val(args, "--data").unwrap_or_else(|| format!("./mtcp-{id}")));
    let bind: SocketAddr = flag_val(args, "--bind")
        .unwrap_or_else(|| format!("0.0.0.0:{}", 9700 + id))
        .parse()
        .expect("bind");
    let n_ranges: u64 = flag_val(args, "--ranges")
        .unwrap_or_else(|| "1".into())
        .parse()
        .expect("ranges");
    let health_bind: SocketAddr = flag_val(args, "--health")
        .or_else(|| env::var("HEALTH_BIND").ok())
        .unwrap_or_else(|| {
            let p = bind.port().saturating_add(79); // 9701 → 9780
            format!("0.0.0.0:{p}")
        })
        .parse()
        .expect("health bind");
    let mut peers = flag_peers(args);
    let bind_hp = bind.to_string();
    if peers.is_empty() {
        peers.insert(id, bind_hp.clone());
    }
    if !peers.contains_key(&id) {
        peers.insert(id, bind_hp);
    }
    let mut member_ids: Vec<u64> = peers.keys().copied().collect();
    member_ids.sort_unstable();

    std::fs::create_dir_all(&data).ok();
    let cluster = StoreCluster::open_single_node(&data, id, &member_ids, n_ranges)
        .unwrap_or_else(|e| {
            eprintln!("open_single_node: {e}");
            process::exit(1);
        });

    let (tx, rx) = mpsc::sync_channel::<Work>(256);

    // Accept thread: only I/O + channel (cluster stays on main worker).
    {
        let tx = tx.clone();
        thread::spawn(move || accept_loop(bind, tx));
    }
    // HTTP health for LB / RoleAware proxy (soviet-style /leader /ready).
    {
        let tx = tx.clone();
        thread::spawn(move || health_http_loop(health_bind, id, tx));
    }

    eprintln!(
        "montanha-tcp node id={id} bind={bind} health={health_bind} data={} members={member_ids:?}",
        data.display()
    );

    worker_loop(id, cluster, peers, rx);
}

/// Minimal HTTP/1.0 health server — no hyper dep.
fn health_http_loop(bind: SocketAddr, self_id: u64, tx: SyncSender<Work>) {
    let listener = match TcpListener::bind(bind) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("health bind {bind}: {e}");
            return;
        }
    };
    eprintln!("health http listening on {bind}");
    for conn in listener.incoming() {
        let Ok(mut stream) = conn else { continue };
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .ok();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .ok();
        let mut buf = [0u8; 1024];
        let n = match std::io::Read::read(&mut stream, &mut buf) {
            Ok(0) | Err(_) => continue,
            Ok(n) => n,
        };
        let req = String::from_utf8_lossy(&buf[..n]);
        let path = req
            .lines()
            .next()
            .and_then(|line| {
                // GET /leader HTTP/1.1
                let mut parts = line.split_whitespace();
                let _method = parts.next()?;
                parts.next()
            })
            .unwrap_or("/");
        let path = path.split('?').next().unwrap_or(path);

        let (code, body) = match path {
            "/ready" | "/health" | "/liveness" | "/readiness" => match query_status(&tx) {
                Ok(st) => (200, format!("ready local={self_id} {st}\n")),
                Err(e) => (503, format!("not ready: {e}\n")),
            },
            "/leader" | "/primary" => match query_status(&tx) {
                Ok(st) => {
                    if status_says_leader(&st, self_id) {
                        (200, format!("leader local={self_id} {st}\n"))
                    } else {
                        (503, format!("not leader local={self_id} {st}\n"))
                    }
                }
                Err(e) => (503, format!("error: {e}\n")),
            },
            "/follower" | "/replica" => match query_status(&tx) {
                Ok(st) => {
                    if status_says_leader(&st, self_id) {
                        (503, format!("is leader local={self_id} {st}\n"))
                    } else if st.contains("local=") {
                        (200, format!("follower local={self_id} {st}\n"))
                    } else {
                        (503, format!("unknown {st}\n"))
                    }
                }
                Err(e) => (503, format!("error: {e}\n")),
            },
            "/status" => match query_status(&tx) {
                Ok(st) => (200, format!("{st}\n")),
                Err(e) => (503, format!("error: {e}\n")),
            },
            // RFC-0021 P0.5 — machine-readable cluster status
            "/v1/cluster" | "/cluster" => match query_cluster_json(&tx) {
                Ok(js) => (200, format!("{js}\n")),
                Err(e) => (503, format!("{{\"error\":{e:?}}}\n")),
            },
            _ => (
                404,
                "paths: /ready /leader /follower /status /v1/cluster\n".to_string(),
            ),
        };
        let reason = match code {
            200 => "OK",
            503 => "Service Unavailable",
            404 => "Not Found",
            _ => "Error",
        };
        let resp = format!(
            "HTTP/1.0 {code} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = std::io::Write::write_all(&mut stream, resp.as_bytes());
    }
}

fn query_status(tx: &SyncSender<Work>) -> Result<String, String> {
    let (rtx, rrx) = mpsc::sync_channel(1);
    tx.send(Work::Status { resp: rtx })
        .map_err(|_| "worker gone".to_string())?;
    rrx.recv_timeout(Duration::from_secs(2))
        .map_err(|_| "status timeout".to_string())?
}

fn query_cluster_json(tx: &SyncSender<Work>) -> Result<String, String> {
    let (rtx, rrx) = mpsc::sync_channel(1);
    tx.send(Work::ClusterJson { resp: rtx })
        .map_err(|_| "worker gone".to_string())?;
    rrx.recv_timeout(Duration::from_secs(2))
        .map_err(|_| "cluster json timeout".to_string())?
}

/// Parse `r1:leader=N` (or any `r*:leader=N`) from status_text.
fn status_says_leader(status: &str, self_id: u64) -> bool {
    for part in status.split_whitespace() {
        if let Some(rest) = part.strip_prefix("r") {
            if let Some((_, lead)) = rest.split_once(":leader=") {
                if lead == "-" {
                    continue;
                }
                if lead.parse::<u64>().ok() == Some(self_id) {
                    return true;
                }
            }
        }
    }
    false
}

fn accept_loop(bind: SocketAddr, tx: SyncSender<Work>) {
    let listener = TcpListener::bind(bind).unwrap_or_else(|e| {
        eprintln!("bind {bind}: {e}");
        process::exit(1);
    });
    // Cap concurrent conn handlers — caixote TCP proxies + peer flood can
    // otherwise exhaust RLIMIT_NOFILE ("Too many open files").
    let inflight = Arc::new(AtomicUsize::new(0));
    const MAX_INFLIGHT: usize = 48;
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let cur = inflight.load(Ordering::Relaxed);
                if cur >= MAX_INFLIGHT {
                    drop(stream);
                    continue;
                }
                inflight.fetch_add(1, Ordering::Relaxed);
                let tx = tx.clone();
                let inflight_c = Arc::clone(&inflight);
                thread::spawn(move || {
                    let _guard = InflightGuard(inflight_c);
                    if let Err(e) = handle_conn(tx, stream) {
                        let msg = e.to_string();
                        if !msg.contains("tcp read magic")
                            && !msg.contains("Connection reset")
                            && !msg.contains("Broken pipe")
                            && !msg.contains("Resource temporarily unavailable")
                        {
                            eprintln!("conn: {msg}");
                        }
                    }
                });
            }
            Err(e) => {
                // EMFILE / ENFILE — brief backoff.
                eprintln!("accept: {e}");
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// Decrement inflight counter on drop (no external crate).
struct InflightGuard(Arc<AtomicUsize>);
impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

fn handle_conn(tx: SyncSender<Work>, mut stream: TcpStream) -> Result<(), StoreError> {
    // Short first-frame timeout: TCP health probes that never send MTCP must not
    // pin FDs for 30s (EMFILE under proxy churn).
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .ok();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .ok();
    stream.set_nodelay(true).ok();
    let msg = read_frame(&mut stream)?;
    match msg {
        WireMsg::Peer { from, to, body } => {
            // Enqueue for worker; do not block the accept thread on a round-trip
            // ack (senders are fire-and-forget). Dropping the response channel is fine.
            let (rtx, _rrx) = mpsc::sync_channel(1);
            tx.send(Work::Peer {
                from,
                to,
                body,
                resp: rtx,
            })
            .map_err(|_| StoreError::Msg("worker dead".into()))?;
            // Best-effort ack if the client still reads (optional).
            let _ = write_frame(&mut stream, &WireMsg::RespOk);
        }
        WireMsg::Put { key, value } => {
            let (rtx, rrx) = mpsc::sync_channel(1);
            tx.send(Work::Put {
                key,
                value,
                resp: rtx,
            })
            .map_err(|_| StoreError::Msg("worker dead".into()))?;
            // Puts may wait for majority AE/RV round-trips.
            let r = rrx
                .recv_timeout(Duration::from_secs(20))
                .map_err(|_| StoreError::Msg("put timeout".into()))?;
            match r {
                Ok(()) => write_frame(&mut stream, &WireMsg::RespOk)?,
                Err(m) => write_frame(
                    &mut stream,
                    &WireMsg::RespErr { message: m },
                )?,
            }
        }
        WireMsg::PutBatch { pairs } => {
            let (rtx, rrx) = mpsc::sync_channel(1);
            tx.send(Work::PutBatch { pairs, resp: rtx })
                .map_err(|_| StoreError::Msg("worker dead".into()))?;
            let r = rrx
                .recv_timeout(Duration::from_secs(25))
                .map_err(|_| StoreError::Msg("put_batch timeout".into()))?;
            match r {
                Ok(()) => write_frame(&mut stream, &WireMsg::RespOk)?,
                Err(m) => write_frame(&mut stream, &WireMsg::RespErr { message: m })?,
            }
        }
        WireMsg::Get { key } => {
            let (rtx, rrx) = mpsc::sync_channel(1);
            tx.send(Work::Get { key, resp: rtx })
                .map_err(|_| StoreError::Msg("worker dead".into()))?;
            let r = rrx
                .recv_timeout(Duration::from_secs(10))
                .map_err(|_| StoreError::Msg("get timeout".into()))?;
            match r {
                Ok(v) => write_frame(&mut stream, &WireMsg::RespValue { value: v })?,
                Err(m) => write_frame(
                    &mut stream,
                    &WireMsg::RespErr { message: m },
                )?,
            }
        }
        WireMsg::Tick => {
            let (rtx, rrx) = mpsc::sync_channel(1);
            tx.send(Work::Tick { resp: rtx })
                .map_err(|_| StoreError::Msg("worker dead".into()))?;
            let r = rrx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| StoreError::Msg("tick timeout".into()))?;
            match r {
                Ok(()) => write_frame(&mut stream, &WireMsg::RespOk)?,
                Err(m) => write_frame(
                    &mut stream,
                    &WireMsg::RespErr { message: m },
                )?,
            }
        }
        WireMsg::Status => {
            let (rtx, rrx) = mpsc::sync_channel(1);
            tx.send(Work::Status { resp: rtx })
                .map_err(|_| StoreError::Msg("worker dead".into()))?;
            let r = rrx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| StoreError::Msg("status timeout".into()))?;
            match r {
                Ok(text) => write_frame(&mut stream, &WireMsg::StatusResp { text })?,
                Err(m) => write_frame(
                    &mut stream,
                    &WireMsg::RespErr { message: m },
                )?,
            }
        }
        WireMsg::SetPeers { peers } => {
            let (rtx, rrx) = mpsc::sync_channel(1);
            tx.send(Work::SetPeers { peers, resp: rtx })
                .map_err(|_| StoreError::Msg("worker dead".into()))?;
            let r = rrx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| StoreError::Msg("setpeers timeout".into()))?;
            match r {
                Ok(()) => write_frame(&mut stream, &WireMsg::RespOk)?,
                Err(m) => write_frame(
                    &mut stream,
                    &WireMsg::RespErr { message: m },
                )?,
            }
        }
        WireMsg::CommitTx { pairs } => {
            let (rtx, rrx) = mpsc::sync_channel(1);
            tx.send(Work::CommitTx { pairs, resp: rtx })
                .map_err(|_| StoreError::Msg("worker dead".into()))?;
            let r = rrx
                .recv_timeout(Duration::from_secs(25))
                .map_err(|_| StoreError::Msg("commit_tx timeout".into()))?;
            match r {
                Ok(txn_id) => write_frame(&mut stream, &WireMsg::RespTxn { txn_id })?,
                Err(m) => write_frame(
                    &mut stream,
                    &WireMsg::RespErr { message: m },
                )?,
            }
        }
        WireMsg::DcsCreate { key, value } => {
            let (rtx, rrx) = mpsc::sync_channel(1);
            tx.send(Work::DcsCreate {
                key,
                value,
                resp: rtx,
            })
            .map_err(|_| StoreError::Msg("worker dead".into()))?;
            let r = rrx
                .recv_timeout(Duration::from_secs(25))
                .map_err(|_| StoreError::Msg("dcs_create timeout".into()))?;
            match r {
                Ok(rev) => write_frame(&mut stream, &WireMsg::RespRev { rev })?,
                Err(m) => write_frame(&mut stream, &WireMsg::RespErr { message: m })?,
            }
        }
        WireMsg::DcsCas {
            key,
            value,
            expected_rev,
        } => {
            let (rtx, rrx) = mpsc::sync_channel(1);
            tx.send(Work::DcsCas {
                key,
                value,
                expected_rev,
                resp: rtx,
            })
            .map_err(|_| StoreError::Msg("worker dead".into()))?;
            let r = rrx
                .recv_timeout(Duration::from_secs(25))
                .map_err(|_| StoreError::Msg("dcs_cas timeout".into()))?;
            match r {
                Ok(rev) => write_frame(&mut stream, &WireMsg::RespRev { rev })?,
                Err(m) => write_frame(&mut stream, &WireMsg::RespErr { message: m })?,
            }
        }
        WireMsg::DcsGet { key } => {
            let (rtx, rrx) = mpsc::sync_channel(1);
            tx.send(Work::DcsGet { key, resp: rtx })
                .map_err(|_| StoreError::Msg("worker dead".into()))?;
            let r = rrx
                .recv_timeout(Duration::from_secs(10))
                .map_err(|_| StoreError::Msg("dcs_get timeout".into()))?;
            match r {
                Ok(v) => write_frame(&mut stream, &WireMsg::RespValue { value: v })?,
                Err(m) => write_frame(&mut stream, &WireMsg::RespErr { message: m })?,
            }
        }
        other => {
            write_frame(
                &mut stream,
                &WireMsg::RespErr {
                    message: format!("unexpected client msg {other:?}"),
                },
            )?;
        }
    }
    Ok(())
}

fn worker_loop(
    id: u64,
    mut cluster: StoreCluster,
    mut peers: HashMap<u64, String>,
    rx: Receiver<Work>,
) {
    // 50ms ticks: enough for elect/HB; lower dial rate over public TCP.
    let tick_every = Duration::from_millis(50);
    let mut last_tick = Instant::now();
    loop {
        let timeout = tick_every.saturating_sub(last_tick.elapsed());
        match rx.recv_timeout(timeout) {
            Ok(Work::Peer {
                from,
                to,
                body,
                resp,
            }) => {
                let r = cluster
                    .handle_inbound(from, to, &body)
                    .map_err(|e| e.to_string());
                flush_outbound(id, &mut cluster, &peers);
                let _ = resp.send(r);
            }
            Ok(Work::Put { key, value, resp }) => {
                // RFC-0025 P1.1: coalesce queued Puts into put_many (one Raft batch
                // per range) so concurrent TCP clients share fsync/majority cost.
                let mut batch: Vec<(Vec<u8>, Vec<u8>)> = vec![(key, value)];
                let mut resps = vec![resp];
                let mut deferred: Option<Work> = None;
                while batch.len() < 64 {
                    match rx.try_recv() {
                        Ok(Work::Put {
                            key: k,
                            value: v,
                            resp: r,
                        }) => {
                            batch.push((k, v));
                            resps.push(r);
                        }
                        Ok(other) => {
                            deferred = Some(other);
                            break;
                        }
                        Err(_) => break,
                    }
                }
                let r = put_many_until_committed(id, &mut cluster, &peers, &batch, &rx);
                let msg = r.map_err(|e| e.to_string());
                for resp in resps {
                    let _ = resp.send(msg.clone());
                }
                if let Some(w) = deferred {
                    service_nested(id, &mut cluster, &peers, w);
                }
            }
            Ok(Work::PutBatch { pairs, resp }) => {
                let r = put_many_until_committed(id, &mut cluster, &peers, &pairs, &rx);
                let _ = resp.send(r.map_err(|e| e.to_string()));
            }
            Ok(Work::Get { key, resp }) => {
                let r = cluster
                    .get(&key)
                    .map(|o| o.map(|b| b.to_vec()))
                    .map_err(|e| e.to_string());
                let _ = resp.send(r);
            }
            Ok(Work::Tick { resp }) => {
                let r = cluster.tick().map_err(|e| e.to_string());
                flush_outbound(id, &mut cluster, &peers);
                let _ = resp.send(r);
            }
            Ok(Work::Status { resp }) => {
                let _ = resp.send(Ok(cluster.status_text()));
            }
            Ok(Work::ClusterJson { resp }) => {
                let _ = resp.send(Ok(cluster.cluster_status_json()));
            }
            Ok(Work::CommitTx { pairs, resp }) => {
                let r = commit_tx_drive(id, &mut cluster, &peers, &pairs, &rx);
                let _ = resp.send(r.map_err(|e| e.to_string()));
            }
            Ok(Work::DcsCreate { key, value, resp }) => {
                let r = dcs_mutate_drive(
                    id,
                    &mut cluster,
                    &peers,
                    &rx,
                    DcsMutate::Create { key, value },
                );
                let _ = resp.send(r.map_err(|e| e.to_string()));
            }
            Ok(Work::DcsCas {
                key,
                value,
                expected_rev,
                resp,
            }) => {
                let r = dcs_mutate_drive(
                    id,
                    &mut cluster,
                    &peers,
                    &rx,
                    DcsMutate::Cas {
                        key,
                        value,
                        expected_rev,
                    },
                );
                let _ = resp.send(r.map_err(|e| e.to_string()));
            }
            Ok(Work::DcsGet { key, resp }) => {
                let r = cluster
                    .dcs_get_on(id, &key)
                    .map(|o| o.map(|kv| kv.value))
                    .map_err(|e| e.to_string());
                let _ = resp.send(r);
            }
            Ok(Work::SetPeers {
                peers: new_peers,
                resp,
            }) => {
                peers.clear();
                for (pid, hp) in new_peers {
                    peers.insert(pid, hp);
                }
                eprintln!("set-peers ok map={peers:?}");
                let _ = resp.send(Ok(()));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // periodic tick
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if last_tick.elapsed() >= tick_every {
            if let Err(e) = cluster.tick() {
                eprintln!("tick err: {e}");
            }
            flush_outbound(id, &mut cluster, &peers);
            last_tick = Instant::now();
        }
    }
}

fn flush_outbound(
    self_id: u64,
    cluster: &mut StoreCluster,
    peers: &HashMap<u64, String>,
) {
    let batch = cluster.drain_outbound();
    for (from, to, bytes) in batch {
        if to == self_id {
            let _ = cluster.handle_inbound(from, to, &bytes);
            continue;
        }
        let Some(addr) = peers.get(&to) else {
            eprintln!("no peer addr for node {to}");
            continue;
        };
        if let Err(e) = send_peer(addr, from, to, &bytes) {
            eprintln!("send peer {from}->{to} @{addr}: {e}");
        }
    }
}

/// Drive put / put_many to majority (RFC-0025 P1.1 coalesce path).
fn put_many_until_committed(
    self_id: u64,
    cluster: &mut StoreCluster,
    peers: &HashMap<u64, String>,
    pairs: &[(Vec<u8>, Vec<u8>)],
    rx: &Receiver<Work>,
) -> Result<(), StoreError> {
    if pairs.is_empty() {
        return Ok(());
    }
    if pairs.len() == 1 {
        match cluster.put(&pairs[0].0, &pairs[0].1) {
            Ok(()) => {
                flush_outbound(self_id, cluster, peers);
                return Ok(());
            }
            Err(StoreError::NotCommitted {
                range_id, index, ..
            }) => {
                return finish_not_committed(
                    self_id, cluster, peers, rx, range_id, index,
                );
            }
            Err(e) => return Err(e),
        }
    }
    match cluster.put_many(pairs.iter().map(|(k, v)| (k.as_slice(), v.as_slice()))) {
        Ok(()) => {
            flush_outbound(self_id, cluster, peers);
            Ok(())
        }
        Err(StoreError::NotCommitted {
            range_id, index, ..
        }) => finish_not_committed(self_id, cluster, peers, rx, range_id, index),
        Err(e) => Err(e),
    }
}

fn finish_not_committed(
    self_id: u64,
    cluster: &mut StoreCluster,
    peers: &HashMap<u64, String>,
    rx: &Receiver<Work>,
    range_id: u64,
    index: u64,
) -> Result<(), StoreError> {
    flush_outbound(self_id, cluster, peers);
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        while let Ok(w) = rx.try_recv() {
            service_nested(self_id, cluster, peers, w);
        }
        if let Ok(true) = cluster.finish_queued_propose(range_id, index, false) {
            flush_outbound(self_id, cluster, peers);
            return Ok(());
        }
        let _ = cluster.tick();
        flush_outbound(self_id, cluster, peers);
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(w) => service_nested(self_id, cluster, peers, w),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let commit = cluster.commit_index(self_id, range_id);
    let _ = cluster.finish_queued_propose(range_id, index, true);
    Err(StoreError::NotCommitted {
        range_id,
        index,
        commit,
    })
}

/// Drive multi-key commit while servicing peer RPCs.
///
/// Single-range (typical mesh `RANGES=1`): [`StoreCluster::put_batch`] — one raft
/// entry, no durable intents (avoids multi-host stuck-intent Conflict on retry).
/// Cross-range: [`StoreCluster::commit_tx`] 2PC.
fn commit_tx_drive(
    self_id: u64,
    cluster: &mut StoreCluster,
    peers: &HashMap<u64, String>,
    pairs: &[(Vec<u8>, Vec<u8>)],
    rx: &Receiver<Work>,
) -> Result<u64, StoreError> {
    for _ in 0..32 {
        match rx.try_recv() {
            Ok(w) => service_nested(self_id, cluster, peers, w),
            Err(_) => break,
        }
    }
    let _ = cluster.tick();
    flush_outbound(self_id, cluster, peers);

    let batch = pairs
        .iter()
        .map(|(k, v)| (k.as_slice(), v.as_slice()))
        .collect::<Vec<_>>();

    match cluster.put_batch(batch.iter().copied()) {
        Ok(()) => {
            flush_outbound(self_id, cluster, peers);
            pump_ae(self_id, cluster, peers, rx, 40);
            // Synthetic id: put_batch has no txn_id; 1 means "committed batch".
            Ok(1)
        }
        Err(StoreError::NotCommitted {
            range_id, index, ..
        }) => {
            flush_outbound(self_id, cluster, peers);
            let deadline = Instant::now() + Duration::from_secs(15);
            while Instant::now() < deadline {
                while let Ok(w) = rx.try_recv() {
                    service_nested(self_id, cluster, peers, w);
                }
                if let Ok(true) = cluster.finish_queued_propose(range_id, index, false) {
                    flush_outbound(self_id, cluster, peers);
                    pump_ae(self_id, cluster, peers, rx, 20);
                    return Ok(1);
                }
                let _ = cluster.tick();
                flush_outbound(self_id, cluster, peers);
                match rx.recv_timeout(Duration::from_millis(10)) {
                    Ok(w) => service_nested(self_id, cluster, peers, w),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            let commit = cluster.commit_index(self_id, range_id);
            let _ = cluster.finish_queued_propose(range_id, index, true);
            Err(StoreError::NotCommitted {
                range_id,
                index,
                commit,
            })
        }
        Err(StoreError::CrossRange { .. }) => {
            // Multi-range: full 2PC path.
            match cluster.commit_tx(batch.iter().copied()) {
                Ok(tid) => {
                    flush_outbound(self_id, cluster, peers);
                    pump_ae(self_id, cluster, peers, rx, 40);
                    Ok(tid)
                }
                Err(e) => {
                    flush_outbound(self_id, cluster, peers);
                    Err(e)
                }
            }
        }
        Err(e) => Err(e),
    }
}

fn pump_ae(
    self_id: u64,
    cluster: &mut StoreCluster,
    peers: &HashMap<u64, String>,
    rx: &Receiver<Work>,
    n: usize,
) {
    for _ in 0..n {
        while let Ok(w) = rx.try_recv() {
            service_nested(self_id, cluster, peers, w);
        }
        let _ = cluster.tick();
        flush_outbound(self_id, cluster, peers);
        thread::sleep(Duration::from_millis(5));
    }
}

fn service_nested(
    self_id: u64,
    cluster: &mut StoreCluster,
    peers: &HashMap<u64, String>,
    w: Work,
) {
    match w {
        Work::Peer {
            from,
            to,
            body,
            resp,
        } => {
            let r = cluster
                .handle_inbound(from, to, &body)
                .map_err(|e| e.to_string());
            flush_outbound(self_id, cluster, peers);
            let _ = resp.send(r);
        }
        Work::Tick { resp } => {
            let r = cluster.tick().map_err(|e| e.to_string());
            flush_outbound(self_id, cluster, peers);
            let _ = resp.send(r);
        }
        Work::Status { resp } => {
            let _ = resp.send(Ok(cluster.status_text()));
        }
        Work::ClusterJson { resp } => {
            let _ = resp.send(Ok(cluster.cluster_status_json()));
        }
        Work::Get { key, resp } => {
            let r = cluster
                .get(&key)
                .map(|o| o.map(|b| b.to_vec()))
                .map_err(|e| e.to_string());
            let _ = resp.send(r);
        }
        Work::Put { key: _, value: _, resp } => {
            // Nested put while another put waits: refuse to avoid re-entrancy mess.
            let _ = resp.send(Err("busy: put in progress".into()));
        }
        Work::PutBatch { pairs: _, resp } => {
            let _ = resp.send(Err("busy: put_batch in progress".into()));
        }
        Work::CommitTx { pairs: _, resp } => {
            let _ = resp.send(Err("busy: commit_tx in progress".into()));
        }
        Work::DcsCreate { key: _, value: _, resp } => {
            let _ = resp.send(Err("busy: dcs mutate in progress".into()));
        }
        Work::DcsCas {
            key: _,
            value: _,
            expected_rev: _,
            resp,
        } => {
            let _ = resp.send(Err("busy: dcs mutate in progress".into()));
        }
        Work::DcsGet { key, resp } => {
            let r = cluster
                .dcs_get_on(self_id, &key)
                .map(|o| o.map(|kv| kv.value))
                .map_err(|e| e.to_string());
            let _ = resp.send(r);
        }
        Work::SetPeers { peers: _, resp } => {
            let _ = resp.send(Err("busy: cannot set-peers nested".into()));
        }
    }
}

enum DcsMutate {
    Create { key: Vec<u8>, value: Vec<u8> },
    Cas {
        key: Vec<u8>,
        value: Vec<u8>,
        expected_rev: u64,
    },
}

/// Drive DCS create/CAS while pumping peer AE (same nested pattern as put).
fn dcs_mutate_drive(
    self_id: u64,
    cluster: &mut StoreCluster,
    peers: &HashMap<u64, String>,
    rx: &Receiver<Work>,
    op: DcsMutate,
) -> Result<u64, StoreError> {
    for _ in 0..16 {
        match rx.try_recv() {
            Ok(w) => service_nested(self_id, cluster, peers, w),
            Err(_) => break,
        }
    }
    let _ = cluster.tick();
    flush_outbound(self_id, cluster, peers);

    let dcs_key = match &op {
        DcsMutate::Create { key, .. } | DcsMutate::Cas { key, .. } => key.clone(),
    };
    let result = match op {
        DcsMutate::Create { key, value } => cluster.dcs_create(&key, &value),
        DcsMutate::Cas {
            key,
            value,
            expected_rev,
        } => cluster.dcs_cas(&key, &value, expected_rev),
    };
    match result {
        Ok(rev) => {
            flush_outbound(self_id, cluster, peers);
            pump_ae(self_id, cluster, peers, rx, 40);
            Ok(rev)
        }
        Err(StoreError::NotCommitted {
            range_id, index, ..
        }) => {
            flush_outbound(self_id, cluster, peers);
            let deadline = Instant::now() + Duration::from_secs(15);
            while Instant::now() < deadline {
                while let Ok(w) = rx.try_recv() {
                    service_nested(self_id, cluster, peers, w);
                }
                if let Ok(true) = cluster.finish_queued_propose(range_id, index, false) {
                    flush_outbound(self_id, cluster, peers);
                    pump_ae(self_id, cluster, peers, rx, 20);
                    if let Ok(Some(kv)) = cluster.dcs_get_on(self_id, &dcs_key) {
                        return Ok(kv.mod_revision);
                    }
                    return Ok(1);
                }
                let _ = cluster.tick();
                flush_outbound(self_id, cluster, peers);
                match rx.recv_timeout(Duration::from_millis(10)) {
                    Ok(w) => service_nested(self_id, cluster, peers, w),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            let commit = cluster.commit_index(self_id, range_id);
            let _ = cluster.finish_queued_propose(range_id, index, true);
            Err(StoreError::NotCommitted {
                range_id,
                index,
                commit,
            })
        }
        Err(e) => {
            flush_outbound(self_id, cluster, peers);
            Err(e)
        }
    }
}

fn send_peer(host_port: &str, from: u64, to: u64, body: &[u8]) -> Result<(), StoreError> {
    // Fire-and-forget: do **not** wait for an application ack. Waiting while the
    // local worker is mid-flush deadlocks when the peer flush dials us back
    // (both sides blocked in send_peer). Delivery is confirmed by the peer's
    // accept thread reading the frame; TCP write success is enough for lab/P0.1.
    let addr = resolve_host_port(host_port)?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(400))
        .map_err(|e| StoreError::Msg(format!("dial {host_port} ({addr}): {e}")))?;
    s.set_write_timeout(Some(Duration::from_millis(800))).ok();
    s.set_nodelay(true).ok();
    write_frame(
        &mut s,
        &WireMsg::Peer {
            from,
            to,
            body: body.to_vec(),
        },
    )?;
    // Full close — avoid half-open FDs piling under proxy/NAT.
    drop(s);
    Ok(())
}

fn cmd_put(args: &[String]) {
    let key = flag_val(args, "--key").expect("--key");
    let value = flag_val(args, "--value").expect("--value");
    let mut addrs: Vec<String> = Vec::new();
    if let Some(a) = flag_val(args, "--addr") {
        addrs.push(a);
    }
    for a in flag_peers(args).into_values() {
        if !addrs.contains(&a) {
            addrs.push(a);
        }
    }
    if addrs.is_empty() {
        eprintln!("need --addr or --peer");
        process::exit(2);
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last_err = String::new();
    while Instant::now() < deadline {
        for addr in &addrs {
            match client_put(addr, key.as_bytes(), value.as_bytes()) {
                Ok(()) => {
                    println!("put ok via {addr}");
                    return;
                }
                Err(e) => last_err = format!("{addr}: {e}"),
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    eprintln!("put failed: {last_err}");
    process::exit(1);
}

fn cmd_get(args: &[String]) {
    let addr = flag_val(args, "--addr").expect("--addr");
    let key = flag_val(args, "--key").expect("--key");
    match client_get(&addr, key.as_bytes()) {
        Ok(Some(v)) => println!("{}", String::from_utf8_lossy(&v)),
        Ok(None) => {
            println!("(missing)");
            process::exit(3);
        }
        Err(e) => {
            eprintln!("get: {e}");
            process::exit(1);
        }
    }
}

fn cmd_status(args: &[String]) {
    let addr = flag_val(args, "--addr").expect("--addr");
    match client_status(&addr) {
        Ok(t) => println!("{t}"),
        Err(e) => {
            eprintln!("status: {e}");
            process::exit(1);
        }
    }
}

fn cmd_tick(args: &[String]) {
    let addr = flag_val(args, "--addr").expect("--addr");
    let n: u32 = flag_val(args, "--n")
        .unwrap_or_else(|| "1".into())
        .parse()
        .expect("n");
    match client_tick(&addr, n) {
        Ok(()) => println!("tick ok n={n}"),
        Err(e) => {
            eprintln!("tick: {e}");
            process::exit(1);
        }
    }
}

fn cmd_set_peers(args: &[String]) {
    // Apply the same peer map to every --peer / --addr target (mesh rewire).
    let peers_map = flag_peers(args);
    if peers_map.is_empty() {
        eprintln!("set-peers needs --peer id=host:port ...");
        process::exit(2);
    }
    let list: Vec<(u64, String)> = {
        let mut v: Vec<_> = peers_map.iter().map(|(&id, hp)| (id, hp.clone())).collect();
        v.sort_by_key(|(id, _)| *id);
        v
    };
    let mut targets: Vec<String> = peers_map.values().cloned().collect();
    if let Some(a) = flag_val(args, "--addr") {
        if !targets.contains(&a) {
            targets.push(a);
        }
    }
    let mut ok = 0u32;
    for t in &targets {
        match client_set_peers(t, &list) {
            Ok(()) => {
                println!("set-peers ok via {t}");
                ok += 1;
            }
            Err(e) => eprintln!("set-peers {t}: {e}"),
        }
    }
    if ok == 0 {
        process::exit(1);
    }
}

fn cmd_elect_wait(args: &[String]) {
    let peers = flag_peers(args);
    if peers.is_empty() {
        eprintln!("need --peer");
        process::exit(2);
    }
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        for (&id, addr) in &peers {
            if let Ok(st) = client_status(addr) {
                for part in st.split_whitespace() {
                    if let Some(rest) = part.strip_prefix("r") {
                        if let Some((_, lead)) = rest.split_once(":leader=") {
                            if lead != "-" {
                                println!("elected leader={lead} (from node {id}) status={st}");
                                return;
                            }
                        }
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(150));
    }
    eprintln!("elect-wait timeout");
    process::exit(1);
}

fn cmd_smoke(args: &[String]) {
    let peers = flag_peers(args);
    if peers.len() < 3 {
        eprintln!("smoke needs >=3 --peer id=addr");
        process::exit(2);
    }
    let peer_args: Vec<String> = peers
        .iter()
        .flat_map(|(id, addr)| vec!["--peer".into(), format!("{id}={addr}")])
        .collect();
    cmd_elect_wait(&peer_args);

    let key = b"mtcp-smoke";
    let val = b"hello-linux";
    let addrs: Vec<String> = peers.values().cloned().collect();
    let mut put_ok = false;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !put_ok {
        for addr in &addrs {
            if client_put(addr, key, val).is_ok() {
                println!("smoke put ok via {addr}");
                put_ok = true;
                break;
            }
        }
        if !put_ok {
            thread::sleep(Duration::from_millis(150));
        }
    }
    if !put_ok {
        eprintln!("smoke put failed on all peers");
        process::exit(1);
    }

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut seen = 0u32;
        for addr in &addrs {
            if client_get(addr, key).ok().flatten().as_deref() == Some(val.as_ref()) {
                seen += 1;
            }
        }
        if seen >= 2 {
            println!("smoke ok majority={seen}/{}", addrs.len());
            return;
        }
        if Instant::now() >= deadline {
            eprintln!("smoke majority timeout seen={seen}");
            process::exit(1);
        }
        thread::sleep(Duration::from_millis(150));
    }
}

// ── RoleAware L4 proxy (soviet-shaped /leader) ──────────────────────────────

#[derive(Clone, Debug)]
struct ProxyMember {
    /// MTCP data plane `host:port`.
    data: String,
    /// HTTP health base `host:port` (GET /leader /follower /ready).
    health: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProxyMode {
    /// Writes: only backends advertising GET /leader → 200. Fail closed.
    Write,
    /// Reads: any GET /ready → 200 (leader + followers).
    Read,
    /// Any ready backend (smoke / liveness).
    Any,
}

/// `--member host:dataPort[@healthPort]` — health defaults to dataPort+79.
fn parse_member(s: &str) -> ProxyMember {
    let (data_hp, health_override) = match s.split_once('@') {
        Some((d, h)) => (d, Some(h)),
        None => (s, None),
    };
    let (host, data_port) = data_hp.rsplit_once(':').unwrap_or_else(|| {
        eprintln!("member must be host:port[@healthPort], got {s}");
        process::exit(2);
    });
    let data_port: u16 = data_port.parse().unwrap_or_else(|_| {
        eprintln!("bad data port in {s}");
        process::exit(2);
    });
    let health_port: u16 = if let Some(h) = health_override {
        if let Some((hh, hp)) = h.rsplit_once(':') {
            // allow host:healthPort override of host
            let _ = hh;
            hp.parse().unwrap_or_else(|_| {
                eprintln!("bad health port in {s}");
                process::exit(2);
            })
        } else {
            h.parse().unwrap_or_else(|_| {
                eprintln!("bad health port in {s}");
                process::exit(2);
            })
        }
    } else {
        data_port.saturating_add(79)
    };
    ProxyMember {
        data: format!("{host}:{data_port}"),
        health: format!("{host}:{health_port}"),
    }
}

fn flag_members(args: &[String]) -> Vec<ProxyMember> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--member" {
            if let Some(v) = args.get(i + 1) {
                out.push(parse_member(v));
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Minimal HTTP/1.0 GET — returns status code (0 on transport error).
fn http_get_status(health_hp: &str, path: &str) -> u16 {
    let Ok(addr) = resolve_host_port(health_hp) else {
        return 0;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(400)) else {
        return 0;
    };
    stream
        .set_read_timeout(Some(Duration::from_millis(400)))
        .ok();
    stream
        .set_write_timeout(Some(Duration::from_millis(400)))
        .ok();
    let host = health_hp.split(':').next().unwrap_or(health_hp);
    let req = format!(
        "GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(req.as_bytes()).is_err() {
        return 0;
    }
    let mut buf = [0u8; 128];
    let n = stream.read(&mut buf).unwrap_or(0);
    if n < 12 {
        return 0;
    }
    // HTTP/1.x 200 ...
    let line = String::from_utf8_lossy(&buf[..n]);
    let code = line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    code
}

fn pick_backend(mode: ProxyMode, members: &[ProxyMember]) -> Option<String> {
    match mode {
        ProxyMode::Write => {
            for m in members {
                if http_get_status(&m.health, "/leader") == 200 {
                    return Some(m.data.clone());
                }
            }
            None
        }
        ProxyMode::Read | ProxyMode::Any => {
            // Prefer followers for Read when available; fall back to any ready.
            if mode == ProxyMode::Read {
                for m in members {
                    if http_get_status(&m.health, "/follower") == 200 {
                        return Some(m.data.clone());
                    }
                }
            }
            for m in members {
                if http_get_status(&m.health, "/ready") == 200 {
                    return Some(m.data.clone());
                }
            }
            None
        }
    }
}

fn pipe_bidirectional(a: TcpStream, b: TcpStream) {
    let (mut a_r, mut a_w) = {
        let ar = a.try_clone().ok();
        let aw = a;
        match ar {
            Some(r) => (r, aw),
            None => return,
        }
    };
    let (mut b_r, mut b_w) = {
        let br = b.try_clone().ok();
        let bw = b;
        match br {
            Some(r) => (r, bw),
            None => return,
        }
    };
    let t = thread::spawn(move || {
        let _ = std::io::copy(&mut a_r, &mut b_w);
        let _ = b_w.shutdown(Shutdown::Write);
    });
    let _ = std::io::copy(&mut b_r, &mut a_w);
    let _ = a_w.shutdown(Shutdown::Write);
    let _ = t.join();
}

/// RoleAware L4 TCP proxy: write→only /leader=200; read→follower|ready; fail closed.
///
/// ```text
/// montanha-tcp proxy --listen 0.0.0.0:9600 --mode write \
///   --member 10.0.0.112:9701 --member 10.0.0.109:9701 --member 10.0.0.111:9701
/// ```
fn cmd_proxy(args: &[String]) {
    let listen: SocketAddr = flag_val(args, "--listen")
        .or_else(|| env::var("PROXY_LISTEN").ok())
        .unwrap_or_else(|| "0.0.0.0:9600".into())
        .parse()
        .expect("--listen host:port");
    let mode = match flag_val(args, "--mode")
        .or_else(|| env::var("PROXY_MODE").ok())
        .unwrap_or_else(|| "write".into())
        .to_ascii_lowercase()
        .as_str()
    {
        "write" | "primary" | "leader" => ProxyMode::Write,
        "read" | "replica" | "follower" => ProxyMode::Read,
        "any" | "ready" => ProxyMode::Any,
        other => {
            eprintln!("--mode must be write|read|any, got {other}");
            process::exit(2);
        }
    };
    let mut members = flag_members(args);
    if members.is_empty() {
        // Also accept --peer id=host:port (reuse mesh map; health = dataPort+79).
        for (_, hp) in flag_peers(args) {
            members.push(parse_member(&hp));
        }
    }
    if members.is_empty() {
        eprintln!("proxy needs --member host:port[@healthPort] ... (or --peer id=host:port)");
        process::exit(2);
    }

    let cache: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let members_bg = members.clone();
    let cache_bg = cache.clone();
    let mode_bg = mode;
    thread::spawn(move || {
        let mut last = String::new();
        loop {
            let pick = pick_backend(mode_bg, &members_bg);
            if let Ok(mut g) = cache_bg.lock() {
                *g = pick.clone();
            }
            if let Some(ref p) = pick {
                if p != &last {
                    eprintln!("proxy mode={mode_bg:?} backend={p}");
                    last = p.clone();
                }
            } else if !last.is_empty() {
                eprintln!("proxy mode={mode_bg:?} backend=NONE (fail closed)");
                last.clear();
            }
            thread::sleep(Duration::from_millis(300));
        }
    });

    let listener = TcpListener::bind(listen).unwrap_or_else(|e| {
        eprintln!("proxy listen {listen}: {e}");
        process::exit(1);
    });
    eprintln!(
        "proxy listening {listen} mode={mode:?} members={}",
        members.len()
    );
    // Warm cache once before accepting.
    if let Ok(mut g) = cache.lock() {
        *g = pick_backend(mode, &members);
    }

    let active = Arc::new(AtomicUsize::new(0));
    for conn in listener.incoming() {
        let Ok(client) = conn else { continue };
        let backend = cache
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .or_else(|| pick_backend(mode, &members));
        let Some(backend_hp) = backend else {
            // Fail closed: accept then RST/close — no silent random routing.
            let _ = client.shutdown(Shutdown::Both);
            continue;
        };
        let active_c = active.clone();
        active_c.fetch_add(1, Ordering::Relaxed);
        thread::spawn(move || {
            let _guard = scopeguard_dec(active_c);
            let Ok(addr) = resolve_host_port(&backend_hp) else {
                return;
            };
            let Ok(upstream) =
                TcpStream::connect_timeout(&addr, Duration::from_secs(2))
            else {
                return;
            };
            let _ = client.set_nodelay(true);
            let _ = upstream.set_nodelay(true);
            pipe_bidirectional(client, upstream);
        });
    }
}

/// Decrement active counter when the proxy connection ends (no external crate).
fn scopeguard_dec(active: Arc<AtomicUsize>) -> impl Drop {
    struct Dec(Arc<AtomicUsize>);
    impl Drop for Dec {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::Relaxed);
        }
    }
    Dec(active)
}
