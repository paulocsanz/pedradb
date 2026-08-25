//! Smoke: run one seed and print trace_hash (P1 world).

use pedradb_world::{temp_parent, World, WorldConfig};

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|s| {
            let t = s.trim();
            if let Some(h) = t
                .strip_prefix("0x")
                .or_else(|| t.strip_prefix("0X"))
            {
                u64::from_str_radix(h, 16).ok()
            } else {
                t.parse().ok()
            }
        })
        .unwrap_or(1);
    let parent = temp_parent("cli");
    let cfg = WorldConfig {
        n_nodes: 3,
        n_ranges: 1,
        schedule_steps: 16,
        parent: parent.clone(),
        ..Default::default()
    };
    match World::new(seed, cfg).run() {
        Ok(t) => {
            println!(
                "seed={seed} hash={:016x} puts_ok={} puts_err={} gets_ok={} dcs_ok={} silent_wrong={} events={} net_sent={} dropped={} rpc={} disk_arms={} trip={} t={}",
                t.trace_hash,
                t.puts_ok,
                t.puts_err,
                t.gets_ok,
                t.dcs_ok,
                t.silent_wrong,
                t.events.len(),
                t.net_sent,
                t.net_dropped,
                t.rpc_applied,
                t.disk_arms,
                t.disk_tripped_nodes,
                t.logical_now
            );
            let _ = std::fs::remove_dir_all(&parent);
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("world failed: {e}");
            let _ = std::fs::remove_dir_all(&parent);
            std::process::exit(1);
        }
    }
}
