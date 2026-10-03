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
    let mut bind: SocketAddr = match "127.0.0.1:17001".parse() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("invalid default bind addr: {e}");
            process::exit(1);
        }
    };
    let mut peers: HashMap<u64, SocketAddr> = HashMap::new();

    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--id" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("missing argument for --id");
                    process::exit(2);
                }
                match args[i].parse::<u64>() {
                    Ok(parsed_id) => id = parsed_id,
                    Err(e) => {
                        eprintln!("invalid --id '{}': {e}", args[i]);
                        process::exit(2);
                    }
                }
            }
            "--data" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("missing argument for --data");
                    process::exit(2);
                }
                data = PathBuf::from(&args[i]);
            }
            "--bind" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("missing argument for --bind");
                    process::exit(2);
                }
                match args[i].parse::<SocketAddr>() {
                    Ok(addr) => bind = addr,
                    Err(e) => {
                        eprintln!("invalid --bind '{}': {e}", args[i]);
                        process::exit(2);
                    }
                }
            }
            "--peer" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("missing argument for --peer");
                    process::exit(2);
                }
                match parse_peer(&args[i]) {
                    Ok((pid, addr)) => {
                        peers.insert(pid, addr);
                    }
                    Err(e) => {
                        eprintln!("invalid --peer '{}': {e}", args[i]);
                        process::exit(2);
                    }
                }
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

fn parse_peer(s: &str) -> Result<(u64, SocketAddr), String> {
    let (a, b) = s
        .split_once('=')
        .ok_or_else(|| format!("peer must be in format id=addr, got '{s}'"))?;
    let pid = a
        .parse::<u64>()
        .map_err(|e| format!("invalid peer id '{a}': {e}"))?;
    let addr = b
        .parse::<SocketAddr>()
        .map_err(|e| format!("invalid peer addr '{b}': {e}"))?;
    Ok((pid, addr))
}
