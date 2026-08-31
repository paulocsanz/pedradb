//! RFC-0064 P2.2 / L28: 3 OS processes of `montanha-tcp`, seed-derived KV,
//! kill a follower, reopen, get must still see the acked put.
//!
//! ```text
//! cargo run -p pedradb-store --bin cluster_real -- [seed] [--leader-kill] [--leave-joint] [--remove-member]
//! MONTANHA_TCP=/path/to/montanha-tcp  # default: sibling of this binary
//! ```
//!
//! `--leave-joint` (RFC-0118): after elect, call production `client_leave_joint`
//! (no-op Ok when no joint is in flight). Default fingerprint omits `leave=` so
//! existing L28 durability lines stay byte-stable.
//!
//! `--remove-member` (RFC-0121): after the durability triple, call
//! `client_remove_member_joint` then `client_leave_joint`. After the kids
//! die, reopen each Pedra dir and require on-disk C-new-only (`left=`).
//! Fingerprint adds `remove=`/`leave=`/`left=`/`hw=`/`part=` only with this flag.
//! `hw=1` means on-disk high-water still exceeds the live set (RFC-0126).
//! `part=1` means a removed voter is not participating after reopen (RFC-0128).
//! `apply=1` means recover apply closed `commit > applied` (RFC-0130).
//! `napply=1` means recover apply closed the gap on the removed replica (RFC-0131).
//! `trunc=1` means recover truncate persisted on the removed replica (RFC-0132).
//! `odrop=1` means orphan `log_entry_key` rows were deleted (RFC-0133).
//! `abort=1` means leftover 2PC intents were aborted (RFC-0134).
//! `nowms=1` means `now_ms` persisted on the removed replica (RFC-0135).
//! `hist=1` means SI hist persisted on the removed replica (RFC-0136).
//! `fence=1` means abort fence persisted on the removed replica (RFC-0137).
//! `clear=1` means stuck 2PC intents were force-cleared (RFC-0138).
//! `pre=1` means leftover TX preimages were dropped (RFC-0139).
//! `peer=1` means TCP ctor election timeout follows disk C-new (RFC-0140).
//! `lid=1` means HashMap first-key is not identity after leave (RFC-0141).
//! `rdr=1` means LocalApplied reader skips remote `ids.first()` (RFC-0142).
//! `dsc=1` means live uncommitted-log discard on the removed replica (RFC-0143).
//! `pld=1` means no-leader persist-leader is local so next_index repairs (RFC-0144).
//! `std=1` means a planted Leader on the removed replica is stepped down (RFC-0145).
//! `hnt=1` means remaining voter's leader_hint omits the removed replica (RFC-0146).
//! `slot=1` means remaining voter forgets next/match/sent_through of the removed replica (RFC-0147).
//! `dterm=1` means the removed replica rolled a newer term back when the hard-state persist failed on its REAL dir (RFC-0158).
//! `sth=1` means remaining voter oob `remove_member` drops `sent_through` of a remote replica (RFC-0148).
//! `pj=1` means planted committed C-old,new without leave refuses C-old majority (RFC-0068).
//! Default / `--leave-joint` fingerprints add `sth=`/`pj=` (3 still in `ids`). `--remove-member` omits them.
//!
//! Fingerprint is **outcomes** (put/get/kill/restart), not a World
//! `trace_hash` — TCP elect uses wall-tick so the leader id is not
//! seed-stable. The durability oracle is: same seed ⇒ same value
//! survives process kill.

#![forbid(unsafe_code)]

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use pedradb_store::{
    client_get, client_leave_joint, client_put, client_remove_member_joint, client_status,
    client_tick, elect_claim_banner, high_water_at_least, l28_durability_ok, l28_leader_kill_ok,
    l28_tcp_abort_ok, l28_tcp_apply_ok, l28_tcp_clear_ok, l28_tcp_fence_ok, l28_tcp_hist_ok,
    l28_tcp_hw_ok, l28_tcp_leave_ok, l28_tcp_left_ok, l28_tcp_napply_ok,
    l28_tcp_napply_retry_admitted, l28_tcp_nowms_ok,
    l28_tcp_dsc_ok, l28_tcp_dterm_ok, l28_tcp_hnt_ok, l28_tcp_lid_ok, l28_tcp_odrop_ok, l28_tcp_part_ok,
    l28_tcp_peer_ok, l28_tcp_pld_ok, l28_tcp_plant_ok, l28_tcp_pre_ok, l28_tcp_rdr_ok,
    l28_tcp_pj_ok, l28_tcp_slot_ok, l28_tcp_std_ok, l28_tcp_sth_ok, l28_tcp_trunc_ok,
    liveness_admitted, tcp_node_disk_high_water, tcp_node_disk_left_joint, tcp_node_drop_repl_ok,
    tcp_node_drop_st_ok, tcp_node_hint_ok, tcp_node_plant_joint_ok, tcp_node_recover_apply_ok,
    tcp_node_removed_abort_ok, tcp_node_removed_clear_ok, tcp_node_removed_dsc_ok,
    tcp_node_removed_durable_term_ok, tcp_node_removed_fence_ok, tcp_node_removed_hist_ok,
    tcp_node_removed_lid_ok,
    tcp_node_removed_not_participating, tcp_node_removed_now_ms_ok,
    tcp_node_removed_orphan_drop_ok, tcp_node_removed_peer_ok, tcp_node_removed_pld_ok,
    tcp_node_removed_pre_ok, tcp_node_removed_rdr_ok, tcp_node_removed_recover_apply_ok,
    tcp_node_removed_std_ok, tcp_node_removed_truncate_ok,
};

static PORTS: AtomicU16 = AtomicU16::new(0);

fn tcp_bin() -> PathBuf {
    if let Ok(p) = env::var("MONTANHA_TCP") {
        return PathBuf::from(p);
    }
    let mut p = env::current_exe().expect("exe");
    p.set_file_name("montanha-tcp");
    p
}

fn alloc_base_port() -> u16 {
    // RFC-0157 P0.3 campaign seam: pin the per-process port range so K
    // simultaneous clusters cannot collide — the pid-derived default
    // overlaps for consecutive pids (pid%1500 + 3 slots each). Only the
    // parallel campaign sets this; tests keep the historical scheme.
    if let Ok(p) = env::var("L28_BASE_PORT") {
        let base: u16 = p
            .parse()
            .unwrap_or_else(|_| panic!("L28_BASE_PORT must be a u16, got {p:?}"));
        assert!(
            (1024..=65_500).contains(&base),
            "L28_BASE_PORT out of range: {base}"
        );
        return base + PORTS.fetch_add(3, Ordering::Relaxed);
    }
    let pid = (std::process::id() % 1500) as u16;
    let n = PORTS.fetch_add(3, Ordering::Relaxed);
    23000 + pid + n
}

fn cluster_id_hex(seed: u64) -> String {
    format!("{:016x}{:016x}", seed, seed ^ 0xC1D5_7EED_C1D5_7EED)
}

/// Fingerprint kill field with the resolved target id (`node1`, `leader2`).
/// The id is behavior-resolved (seed % 3, or the parsed leader under
/// `--leader-kill`), so kill-target coverage across a campaign's seeds is
/// checkable from the artifacts instead of re-derivable only from source
/// (post-script in findings/2026-08-31-campaign-seed-collapse/README.md).
fn kill_target_field(kind: &str, node: u64) -> String {
    format!("{kind}{node}")
}

fn parse_r1_leader(st: &str) -> Option<u64> {
    for part in st.split_whitespace() {
        if let Some(rest) = part.strip_prefix("r1:leader=") {
            if rest != "-" {
                return rest.parse().ok();
            }
        }
    }
    None
}

fn wait_leaders(addrs: &[String], timeout: Duration) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    let mut last = String::new();
    while Instant::now() < deadline {
        for a in addrs {
            let _ = client_tick(a, 4);
            if let Ok(st) = client_status(a) {
                last = st.clone();
                if st.contains("leader=") && !st.contains("leader=-") {
                    return Ok(st);
                }
            }
        }
        thread::sleep(Duration::from_millis(80));
    }
    Err(format!("elect timeout last={last}"))
}

fn put_any(addrs: &[String], key: &[u8], val: &[u8], timeout: Duration) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        for a in addrs {
            let _ = client_tick(a, 2);
            if client_put(a, key, val).is_ok() {
                return Ok(a.clone());
            }
        }
        thread::sleep(Duration::from_millis(80));
    }
    Err("put timeout".into())
}

fn remove_any(addrs: &[String], node_id: u64, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        for a in addrs {
            let _ = client_tick(a, 2);
            if client_remove_member_joint(a, node_id).is_ok() {
                return true;
            }
        }
        thread::sleep(Duration::from_millis(80));
    }
    false
}

fn status_omits_member(st: &str, node_id: u64) -> bool {
    let Some(after) = st.split("members=").nth(1) else {
        return false;
    };
    let token = after.split_whitespace().next().unwrap_or("");
    let (Some(s), Some(e)) = (token.find('['), token.find(']')) else {
        return false;
    };
    if e <= s {
        return false;
    }
    let inner = &token[s + 1..e];
    !inner.split(',').any(|p| p.trim() == node_id.to_string())
}

fn wait_member_gone(addrs: &[String], node_id: u64, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let mut all_omit = !addrs.is_empty();
        for a in addrs {
            let _ = client_tick(a, 4);
            match client_status(a) {
                Ok(st) if status_omits_member(&st, node_id) => {}
                _ => all_omit = false,
            }
        }
        if all_omit {
            return true;
        }
        thread::sleep(Duration::from_millis(80));
    }
    false
}

fn disk_left_after_remove(parent: &Path, removed: u64) -> bool {
    (1..=3u64).any(|nid| tcp_node_disk_left_joint(&parent.join(format!("n{nid}")), nid, removed))
}

fn disk_high_water_after_remove(parent: &Path) -> u64 {
    (1..=3u64)
        .map(|nid| tcp_node_disk_high_water(&parent.join(format!("n{nid}")), nid))
        .max()
        .unwrap_or(0)
}

fn get_any(addrs: &[String], key: &[u8], want: &[u8], timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        for a in addrs {
            if client_get(a, key).ok().flatten().as_deref() == Some(want) {
                return true;
            }
        }
        thread::sleep(Duration::from_millis(80));
    }
    false
}

struct Kids(Vec<Child>);

impl Drop for Kids {
    fn drop(&mut self) {
        for c in &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn spawn_node(
    bin: &Path,
    id: u64,
    data: &Path,
    bind: &str,
    peers: &[(u64, String)],
    cid: &str,
) -> Child {
    let mut cmd = Command::new(bin);
    cmd.arg("node")
        .arg("--id")
        .arg(id.to_string())
        .arg("--data")
        .arg(data)
        .arg("--bind")
        .arg(bind)
        .arg("--cluster-id")
        .arg(cid)
        .arg("--ranges")
        .arg("1")
        .stdout(Stdio::null())
        .stderr(match std::fs::File::create(data.join("stderr.log")) {
            Ok(f) => Stdio::from(f),
            Err(_) => Stdio::null(),
        });
    for (pid, addr) in peers {
        cmd.arg("--peer").arg(format!("{pid}={addr}"));
    }
    cmd.spawn().unwrap_or_else(|e| panic!("spawn node {id}: {e}"))
}

fn run(seed: u64, kill_leader: bool, do_leave: bool, do_remove: bool) -> String {
    let bin = tcp_bin();
    assert!(bin.exists(), "montanha-tcp missing at {}", bin.display());
    let parent = env::temp_dir().join(format!(
        "pedra-l28-{seed:016x}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&parent);
    std::fs::create_dir_all(&parent).unwrap();
    for i in 1..=3u64 {
        let _ = std::fs::create_dir_all(parent.join(format!("n{i}")));
    }
    let base = alloc_base_port();
    let peers: Vec<(u64, String)> = (1..=3)
        .map(|i| (i, format!("127.0.0.1:{}", base + i as u16 - 1)))
        .collect();
    let cid = cluster_id_hex(seed);
    let mut kids = Kids(
        (1..=3)
            .map(|i| {
                spawn_node(
                    &bin,
                    i,
                    &parent.join(format!("n{i}")),
                    &peers[(i - 1) as usize].1,
                    &peers,
                    &cid,
                )
            })
            .collect(),
    );
    let addrs: Vec<String> = peers.iter().map(|(_, a)| a.clone()).collect();
    let st = wait_leaders(&addrs, Duration::from_secs(45)).expect("elect");
    let mut leave_ok = 0u8;
    if do_leave && !do_remove {
        for a in &addrs {
            if client_leave_joint(a).is_ok() {
                leave_ok = 1;
                break;
            }
        }
    }
    let key = format!("l28-{seed:x}").into_bytes();
    let val = format!("v-{seed:x}").into_bytes();
    put_any(&addrs, &key, &val, Duration::from_secs(20)).expect("put");
    let get_ok = u8::from(get_any(&addrs, &key, &val, Duration::from_secs(15)));
    let st_now = wait_leaders(&addrs, Duration::from_secs(10)).unwrap_or(st);
    let kill_i = if kill_leader {
        parse_r1_leader(&st_now)
            .map(|id| (id.saturating_sub(1) as usize).min(2))
            .unwrap_or((seed % 3) as usize)
    } else {
        (seed % 3) as usize
    };
    let _ = kids.0[kill_i].kill();
    let _ = kids.0[kill_i].wait();
    let survivors: Vec<String> = addrs
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != kill_i)
        .map(|(_, a)| a.clone())
        .collect();
    let _ = wait_leaders(&survivors, Duration::from_secs(30));
    let kill_ok = u8::from(get_any(&survivors, &key, &val, Duration::from_secs(15)));
    let nid = kill_i as u64 + 1;
    kids.0[kill_i] = spawn_node(
        &bin,
        nid,
        &parent.join(format!("n{nid}")),
        &peers[kill_i].1,
        &peers,
        &cid,
    );
    let _ = wait_leaders(&addrs, Duration::from_secs(30));
    let restart_ok = u8::from(get_any(
        &[addrs[kill_i].clone()],
        &key,
        &val,
        Duration::from_secs(20),
    ));
    let mut remove_ok = 0u8;
    let mut left_ok = 0u8;
    let mut hw_ok = 0u8;
    let mut part_ok = 0u8;
    let mut apply_ok = 0u8;
    let mut napply_ok = 0u8;
    let mut trunc_ok = 0u8;
    let mut odrop_ok = 0u8;
    let mut abort_ok = 0u8;
    let mut nowms_ok = 0u8;
    let mut hist_ok = 0u8;
    let mut fence_ok = 0u8;
    let mut clear_ok = 0u8;
    let mut pre_ok = 0u8;
    let mut peer_ok = 0u8;
    let mut lid_ok = 0u8;
    let mut rdr_ok = 0u8;
    let mut dsc_ok = 0u8;
    let mut pld_ok = 0u8;
    let mut std_ok = 0u8;
    let mut hnt_ok = 0u8;
    let mut slot_ok = 0u8;
    let mut dterm_ok = 0u8;
    let mut sth_ok = 0u8;
    let mut pj_ok = 0u8;
    if do_remove {
        let _ = wait_leaders(&addrs, Duration::from_secs(15));
        if remove_any(&addrs, 3, Duration::from_secs(20)) {
            remove_ok = 1;
        }
        // Joint is still C-old∪C-new: n3 must catch the remove AE before leave
        // applies on the leader and drops the replication slot.
        for _ in 0..60 {
            for a in &addrs {
                let _ = client_tick(a, 8);
            }
            thread::sleep(Duration::from_millis(40));
        }
        for a in &addrs {
            let _ = client_tick(a, 4);
            if client_leave_joint(a).is_ok() {
                leave_ok = 1;
                break;
            }
        }
        // Leave is appended as NotCommitted; ticks let it majority-commit
        // and apply so load_range_peer keeps it (uncommitted suffix is dropped).
        // Require **n3** to omit 3 before SIGKILL: {1,2} can commit leave
        // without n3, and killing then leaves disk membership with 3 —
        // every 0131+ removed-replica helper returns false (`napply=0`).
        // Patience is generous (0157 P2.3): nightly waves run on loaded
        // machines; a starved n3 turns into a slow success here, not a
        // 50s dead attempt + full-seed retry that breaks the K-parallel
        // wall-clock factor gate.
        let n3 = &addrs[2];
        let mut n3_left = wait_member_gone(std::slice::from_ref(n3), 3, Duration::from_secs(40));
        if !n3_left {
            for _ in 0..240 {
                for a in &addrs {
                    let _ = client_tick(a, 8);
                }
                if wait_member_gone(std::slice::from_ref(n3), 3, Duration::from_millis(250)) {
                    n3_left = true;
                    break;
                }
            }
        }
        let _ = wait_member_gone(&addrs[..2], 3, Duration::from_secs(5));
        for _ in 0..8 {
            for a in &addrs {
                let _ = client_tick(a, 4);
            }
            thread::sleep(Duration::from_millis(40));
        }
        let _ = n3_left;
        for c in &mut kids.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
        left_ok = u8::from(disk_left_after_remove(&parent, 3));
        // Started with 3 voters; live set after remove is 2. Disk must keep 3.
        let disk_hw = disk_high_water_after_remove(&parent);
        hw_ok = u8::from(high_water_at_least(disk_hw, 2) >= 3);
        part_ok = u8::from(tcp_node_removed_not_participating(
            &parent.join("n1"),
            1,
            &[1, 2, 3],
            3,
        ));
        apply_ok = u8::from(tcp_node_recover_apply_ok(
            &parent.join("n1"),
            1,
            &[1, 2, 3],
        ));
        napply_ok = u8::from(tcp_node_removed_recover_apply_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
        trunc_ok = u8::from(tcp_node_removed_truncate_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
        odrop_ok = u8::from(tcp_node_removed_orphan_drop_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
        abort_ok = u8::from(tcp_node_removed_abort_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
        nowms_ok = u8::from(tcp_node_removed_now_ms_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
        hist_ok = u8::from(tcp_node_removed_hist_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
        fence_ok = u8::from(tcp_node_removed_fence_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
        clear_ok = u8::from(tcp_node_removed_clear_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
        pre_ok = u8::from(tcp_node_removed_pre_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
        peer_ok = u8::from(tcp_node_removed_peer_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
        lid_ok = u8::from(tcp_node_removed_lid_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
        rdr_ok = u8::from(tcp_node_removed_rdr_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
        dsc_ok = u8::from(tcp_node_removed_dsc_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
        pld_ok = u8::from(tcp_node_removed_pld_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
        std_ok = u8::from(tcp_node_removed_std_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
        hnt_ok = u8::from(tcp_node_hint_ok(
            &parent.join("n1"),
            1,
            &[1, 2, 3],
            3,
        ));
        slot_ok = u8::from(tcp_node_drop_repl_ok(
            &parent.join("n1"),
            1,
            &[1, 2, 3],
            3,
        ));
        dterm_ok = u8::from(tcp_node_removed_durable_term_ok(
            &parent.join("n3"),
            3,
            &[1, 2, 3],
        ));
    } else {
        for c in &mut kids.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
        pj_ok = u8::from(tcp_node_plant_joint_ok(
            &parent.join("n1"),
            1,
            &[1, 2, 3],
            4,
        ));
        sth_ok = u8::from(tcp_node_drop_st_ok(
            &parent.join("n1"),
            1,
            &[1, 2, 3],
            3,
        ));
    }
    let _ = std::fs::remove_dir_all(&parent);
    let kind = if kill_leader { "leader" } else { "node" };
    // `kill=leader{n}` still prefix-matches the `kill=leader` consumer in
    // tests/l28_real_tcp.rs.
    let kill_field = kill_target_field(kind, kill_i as u64 + 1);
    if do_remove {
        format!(
            "seed={seed:x} kill={kill_field} put=1 get={get_ok} after={kill_ok} restart={restart_ok} remove={remove_ok} leave={leave_ok} left={left_ok} hw={hw_ok} part={part_ok} apply={apply_ok} napply={napply_ok} trunc={trunc_ok} odrop={odrop_ok} abort={abort_ok} nowms={nowms_ok} hist={hist_ok} fence={fence_ok} clear={clear_ok} pre={pre_ok} peer={peer_ok} lid={lid_ok} rdr={rdr_ok} dsc={dsc_ok} pld={pld_ok} std={std_ok} hnt={hnt_ok} slot={slot_ok} dterm={dterm_ok}"
        )
    } else if do_leave {
        format!(
            "seed={seed:x} kill={kill_field} put=1 get={get_ok} after={kill_ok} restart={restart_ok} leave={leave_ok} sth={sth_ok} pj={pj_ok}"
        )
    } else {
        format!(
            "seed={seed:x} kill={kill_field} put=1 get={get_ok} after={kill_ok} restart={restart_ok} sth={sth_ok} pj={pj_ok}"
        )
    }
}

fn parse_seed(arg: &str) -> Option<u64> {
    let t = arg.trim();
    match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => t.parse().ok().or_else(|| u64::from_str_radix(t, 16).ok()),
    }
}

fn main() {
    let arg = env::args().nth(1);
    let seed: u64 = match arg.as_deref().and_then(parse_seed) {
        Some(s) => s,
        None => {
            // RFC-0157 correction (2026-08-31): the old silent default
            // 0x0064_1E28 collapsed every campaign seed of the form
            // `0x015A_N01` (underscore + non-hex mnemonic) to one world.
            // A bad seed now refuses to run instead of picking a world.
            eprintln!(
                "cluster_real: seed arg {:?} is not decimal or 0x-hex u64 — refusing \
                 (no silent default; see findings/2026-08-31-campaign-seed-collapse)",
                arg.as_deref().unwrap_or("")
            );
            std::process::exit(2);
        }
    };
    let kill_leader = env::args().any(|a| a == "--leader-kill")
        || env::var("L28_KILL").ok().as_deref() == Some("leader");
    let do_leave = env::args().any(|a| a == "--leave-joint")
        || env::var("L28_LEAVE").ok().as_deref() == Some("1");
    let do_remove = env::args().any(|a| a == "--remove-member")
        || env::var("L28_REMOVE").ok().as_deref() == Some("1");
    let line = run(seed, kill_leader, do_leave, do_remove);
    // RFC-0069 P2.2: REAL TCP elect is bounded wall-tick, not eventual-live.
    // Print `live` only if `liveness_admitted` (and then ES is named).
    let live = liveness_admitted(false, false, false);
    let banner = if live {
        elect_claim_banner(true, true, true)
    } else {
        elect_claim_banner(false, false, false)
    };
    println!("cluster_real {line} {banner}");
    let get_ok = line.contains("get=1");
    let after_ok = line.contains("after=1");
    let restart_ok = line.contains("restart=1");
    let ok = if kill_leader {
        l28_leader_kill_ok(get_ok, after_ok, restart_ok)
    } else {
        l28_durability_ok(get_ok, after_ok, restart_ok)
    };
    if !ok {
        eprintln!("L28 durability miss: {line}");
        std::process::exit(1);
    }
    if do_remove {
        let remove_ok = line.contains("remove=1");
        let leave_ok = line.contains("leave=1");
        let left_ok = line.contains("left=1");
        if !l28_tcp_plant_ok(remove_ok, leave_ok) {
            eprintln!("L28 TCP plant miss: {line}");
            std::process::exit(1);
        }
        if !l28_tcp_left_ok(left_ok) {
            eprintln!("L28 TCP on-disk C-new-only miss: {line}");
            std::process::exit(1);
        }
        let hw_ok = line.contains("hw=1");
        if !l28_tcp_hw_ok(hw_ok) {
            eprintln!("L28 TCP on-disk high-water miss: {line}");
            std::process::exit(1);
        }
        let part_ok = line.contains("part=1");
        if !l28_tcp_part_ok(part_ok) {
            eprintln!("L28 TCP participating miss: {line}");
            std::process::exit(1);
        }
        let apply_ok = line.contains(" apply=1");
        if !l28_tcp_apply_ok(apply_ok) {
            eprintln!("L28 TCP recover apply miss: {line}");
            std::process::exit(1);
        }
        let napply_ok = line.contains("napply=1");
        if !l28_tcp_napply_ok(napply_ok) {
            eprintln!("L28 TCP removed-replica recover apply miss: {line}");
            std::process::exit(1);
        }
        if l28_tcp_napply_retry_admitted(1, napply_ok) {
            eprintln!("L28 TCP napply retry is not forall traces: {line}");
            std::process::exit(1);
        }
        let trunc_ok = line.contains("trunc=1");
        if !l28_tcp_trunc_ok(trunc_ok) {
            eprintln!("L28 TCP removed-replica truncate persist miss: {line}");
            std::process::exit(1);
        }
        let odrop_ok = line.contains("odrop=1");
        if !l28_tcp_odrop_ok(odrop_ok) {
            eprintln!("L28 TCP removed-replica orphan-segment drop miss: {line}");
            std::process::exit(1);
        }
        let abort_ok = line.contains("abort=1");
        if !l28_tcp_abort_ok(abort_ok) {
            eprintln!("L28 TCP removed-replica leftover 2PC abort miss: {line}");
            std::process::exit(1);
        }
        let nowms_ok = line.contains("nowms=1");
        if !l28_tcp_nowms_ok(nowms_ok) {
            eprintln!("L28 TCP removed-replica now_ms persist miss: {line}");
            std::process::exit(1);
        }
        let hist_ok = line.contains("hist=1");
        if !l28_tcp_hist_ok(hist_ok) {
            eprintln!("L28 TCP removed-replica SI hist persist miss: {line}");
            std::process::exit(1);
        }
        let fence_ok = line.contains("fence=1");
        if !l28_tcp_fence_ok(fence_ok) {
            eprintln!("L28 TCP removed-replica abort-fence persist miss: {line}");
            std::process::exit(1);
        }
        let clear_ok = line.contains("clear=1");
        if !l28_tcp_clear_ok(clear_ok) {
            eprintln!("L28 TCP removed-replica force-local TX clear miss: {line}");
            std::process::exit(1);
        }
        let pre_ok = line.contains("pre=1");
        if !l28_tcp_pre_ok(pre_ok) {
            eprintln!("L28 TCP removed-replica drop-preimages miss: {line}");
            std::process::exit(1);
        }
        let peer_ok = line.contains("peer=1");
        if !l28_tcp_peer_ok(peer_ok) {
            eprintln!("L28 TCP removed-replica disk-peer timeout miss: {line}");
            std::process::exit(1);
        }
        let lid_ok = line.contains("lid=1");
        if !l28_tcp_lid_ok(lid_ok) {
            eprintln!("L28 TCP removed-replica local-id gate miss: {line}");
            std::process::exit(1);
        }
        let rdr_ok = line.contains("rdr=1");
        if !l28_tcp_rdr_ok(rdr_ok) {
            eprintln!("L28 TCP removed-replica reader-local gate miss: {line}");
            std::process::exit(1);
        }
        let dsc_ok = line.contains("dsc=1");
        if !l28_tcp_dsc_ok(dsc_ok) {
            eprintln!("L28 TCP removed-replica live discard miss: {line}");
            std::process::exit(1);
        }
        let pld_ok = line.contains("pld=1");
        if !l28_tcp_pld_ok(pld_ok) {
            eprintln!("L28 TCP removed-replica persist-leader locality miss: {line}");
            std::process::exit(1);
        }
        let std_ok = line.contains("std=1");
        if !l28_tcp_std_ok(std_ok) {
            eprintln!("L28 TCP removed-replica Leader step-down miss: {line}");
            std::process::exit(1);
        }
        let hnt_ok = line.contains("hnt=1");
        if !l28_tcp_hnt_ok(hnt_ok) {
            eprintln!("L28 TCP remaining-voter leader-hint miss: {line}");
            std::process::exit(1);
        }
        let slot_ok = line.contains("slot=1");
        if !l28_tcp_slot_ok(slot_ok) {
            eprintln!("L28 TCP remaining-voter repl-slot miss: {line}");
            std::process::exit(1);
        }
        let dterm_ok = line.contains("dterm=1");
        if !l28_tcp_dterm_ok(dterm_ok) {
            eprintln!("L28 TCP removed-replica durable-term rollback miss: {line}");
            std::process::exit(1);
        }
    } else if do_leave {
        let leave_ok = line.contains("leave=1");
        if !l28_tcp_leave_ok(leave_ok) {
            eprintln!("L28 TCP leave miss: {line}");
            std::process::exit(1);
        }
        let sth_ok = line.contains("sth=1");
        if !l28_tcp_sth_ok(sth_ok) {
            eprintln!("L28 TCP remaining-voter oob sent_through miss: {line}");
            std::process::exit(1);
        }
        let pj_ok = line.contains("pj=1");
        if !l28_tcp_pj_ok(pj_ok) {
            eprintln!("L28 TCP planted committed-joint-without-leave miss: {line}");
            std::process::exit(1);
        }
    } else {
        let sth_ok = line.contains("sth=1");
        if !l28_tcp_sth_ok(sth_ok) {
            eprintln!("L28 TCP remaining-voter oob sent_through miss: {line}");
            std::process::exit(1);
        }
        let pj_ok = line.contains("pj=1");
        if !l28_tcp_pj_ok(pj_ok) {
            eprintln!("L28 TCP planted committed-joint-without-leave miss: {line}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod seed_parse_tests {
    use super::parse_seed;

    #[test]
    fn decimal_and_plain_hex_parse() {
        assert_eq!(parse_seed("671"), Some(671));
        assert_eq!(parse_seed(" 42 "), Some(42));
        assert_eq!(parse_seed("0x15b001"), Some(0x15b001));
        assert_eq!(parse_seed("0X15B001"), Some(0x15b001));
    }

    #[test]
    fn mnemonic_prefixes_are_refused_not_defaulted() {
        // 2026-08-31: every RFC-0157 campaign seed of this shape silently
        // collapsed to the 0x641e28 default (113 fingerprint rows); the
        // parser must refuse them so the world choice is never silent.
        assert_eq!(parse_seed("0x0157_C01"), None);
        assert_eq!(parse_seed("0x015A_N01"), None);
        assert_eq!(parse_seed("0x015B_N01"), None);
        assert_eq!(parse_seed(""), None);
    }
}

#[cfg(test)]
mod kill_target_tests {
    use super::kill_target_field;

    #[test]
    fn kill_target_echoes_resolved_node_id() {
        assert_eq!(kill_target_field("node", 1), "node1");
        assert_eq!(kill_target_field("node", 3), "node3");
        assert_eq!(kill_target_field("leader", 2), "leader2");
    }

    #[test]
    fn kill_target_keeps_prefix_consumers_matching() {
        // tests/l28_real_tcp.rs greps `kill=leader` as a substring; the
        // echoed id must not break that (or the campaign's `kill=node`
        // readability).
        assert!(format!("kill={}", kill_target_field("leader", 3)).contains("kill=leader"));
        assert!(format!("kill={}", kill_target_field("node", 2)).contains("kill=node"));
    }
}
