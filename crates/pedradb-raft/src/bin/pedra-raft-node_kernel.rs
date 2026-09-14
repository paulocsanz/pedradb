//! Network Raft node process (RFC-0012 P0).
//!
//! Usage:
//!   pedra-raft-node --id 1 --data /tmp/n1 --bind 127.0.0.1:17001 \
//!     --peer 1=127.0.0.1:17001 --peer 2=127.0.0.1:17002 --peer 3=127.0.0.1:17003

use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process;

use pedradb_raft::net::NetworkNode;

fn main() {
    let mut id = 1u64;
    let mut data = PathBuf::from("./raft-data");
    let mut bind: SocketAddr = "127.0.0.1:17001".parse().unwrap();
    let mut peers: HashMap<u64, SocketAddr> = HashMap::new();

    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--id" => {
                i += 1;
                id = args[i].parse().expect("id");
            }
            "--data" => {
                i += 1;
                data = PathBuf::from(&args[i]);
            }
            "--bind" => {
                i += 1;
                bind = args[i].parse().expect("bind");
            }
            "--peer" => {
                i += 1;
                let (pid, addr) = parse_peer(&args[i]);
                peers.insert(pid, addr);
            }
            other => {
                eprintln!("unknown arg {other}");
                process::exit(2);
            }
        }
        i += 1;
    }
    if peers.is_empty() {
        peers.insert(id, bind);
    }

    eprintln!(
        "pedra-raft-node id={id} bind={bind} data={}",
        data.display()
    );
    let node = NetworkNode::open(id, data, bind, peers).unwrap_or_else(|e| {
        eprintln!("open failed: {e}");
        process::exit(1);
    });
    if let Err(e) = node.serve() {
        eprintln!("serve failed: {e}");
        process::exit(1);
    }
}

fn parse_peer(s: &str) -> (u64, SocketAddr) {
    let (a, b) = s.split_once('=').expect("peer id=addr");
    (a.parse().expect("peer id"), b.parse().expect("peer addr"))
}
