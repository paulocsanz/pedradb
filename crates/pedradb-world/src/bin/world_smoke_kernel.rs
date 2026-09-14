//! Smoke: run one seed and print trace_hash (P1 world).
//!
//! RFC-0079 P1.2: `--claim-tcg` is refused unless `tcg_guest_admitted`.
//! Native World does not SSH and does not invent a guest.

use pedradb_world::{allow_claim_tcg_flag, fdb_class_campaign, temp_parent, World};

fn parse_seed(t: &str) -> Option<u64> {
    let t = t.trim();
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u64::from_str_radix(h, 16).ok()
    } else {
        t.parse().ok()
    }
}

fn main() {
    let mut seed = 1u64;
    let mut claim_tcg = false;
    for a in std::env::args().skip(1) {
        if a == "--claim-tcg" {
            claim_tcg = true;
            continue;
        }
        if let Some(s) = parse_seed(&a) {
            seed = s;
        }
    }
    let parent = temp_parent("cli");
    let cfg = fdb_class_campaign(parent.clone());
    match World::new(seed, cfg).run() {
        Ok(t) => {
            println!(
                "seed={seed} hash={:016x} puts_ok={} puts_err={} gets_ok={} dcs_ok={} silent_wrong={} events={} net_sent={} dropped={} rpc={} disk_arms={} trip={} t={} tcg_guest={}",
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
                t.logical_now,
                u8::from(t.claim_tcg_guest()),
            );
            if !allow_claim_tcg_flag(claim_tcg, t.claim_tcg_guest()) {
                eprintln!(
                    "world_smoke: --claim-tcg refused (tcg_guest_admitted=false; native World is not a TCG guest)"
                );
                let _ = std::fs::remove_dir_all(&parent);
                std::process::exit(2);
            }
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
