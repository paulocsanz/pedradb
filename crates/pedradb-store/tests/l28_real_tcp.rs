//! RFC-0064 P2.2 / L28: REAL 3-process TCP cluster, same seed twice.
//!
//! Outcome fingerprint (put/get/kill/restart) must match. Leader identity
//! is not in the hash (wall-tick elect). The durability oracle is the
//! seed-derived value surviving SIGKILL of one node + reopen.
//!
//! Production `cluster_real` / `montanha-tcp` open is `RpcMode::Queued`
//! (RFC-0067). Direct RPC is lab-only.

#![cfg(unix)]

use pedradb_store::{
    l28_durability_ok, l28_leader_kill_ok, l28_leader_kill_ok_as_is, l28_tcp_apply_ok,
    l28_tcp_apply_ok_as_is, l28_tcp_hw_ok, l28_tcp_hw_ok_as_is, l28_tcp_leave_ok,
    l28_tcp_leave_ok_as_is, l28_tcp_left_ok, l28_tcp_left_ok_as_is, l28_tcp_napply_ok,
    l28_tcp_napply_ok_as_is, l28_tcp_napply_retry_admitted, l28_tcp_napply_retry_admitted_as_is,
    l28_tcp_part_ok, l28_tcp_part_ok_as_is, l28_tcp_plant_ok,
    l28_tcp_abort_ok, l28_tcp_abort_ok_as_is, l28_tcp_clear_ok, l28_tcp_clear_ok_as_is,
    l28_tcp_lid_ok, l28_tcp_lid_ok_as_is, l28_tcp_peer_ok, l28_tcp_peer_ok_as_is,
    l28_tcp_dsc_ok, l28_tcp_dsc_ok_as_is, l28_tcp_pld_ok, l28_tcp_pld_ok_as_is,
    l28_tcp_pre_ok, l28_tcp_pre_ok_as_is, l28_tcp_rdr_ok, l28_tcp_rdr_ok_as_is,
    l28_tcp_hnt_ok, l28_tcp_hnt_ok_as_is, l28_tcp_slot_ok, l28_tcp_slot_ok_as_is,
    l28_tcp_pj_ok, l28_tcp_pj_ok_as_is, l28_tcp_std_ok, l28_tcp_std_ok_as_is, l28_tcp_sth_ok,
    l28_tcp_sth_ok_as_is,
    l28_tcp_fence_ok, l28_tcp_fence_ok_as_is,
    l28_tcp_hist_ok, l28_tcp_hist_ok_as_is, l28_tcp_nowms_ok, l28_tcp_nowms_ok_as_is,
    l28_tcp_odrop_ok, l28_tcp_odrop_ok_as_is,
    l28_tcp_plant_ok_as_is, l28_tcp_trunc_ok, l28_tcp_trunc_ok_as_is, world_seed_l28_ok,
    world_seed_l28_ok_as_is,
};
use std::process::Command;
use std::sync::Mutex;

/// One 3-process TCP cluster at a time. Default `cargo test` threads share
/// loopback ports and SIGKILL leftovers; parallel `cluster_real` is
/// `restart=0` / `napply=0` (campaign flake, not a kernel miss).
static CLUSTER_REAL: Mutex<()> = Mutex::new(());

fn parse_l28(line: &str) -> (bool, bool, bool) {
    (
        line.contains("get=1"),
        line.contains("after=1"),
        line.contains("restart=1"),
    )
}

fn run_once(seed: u64, extra: &[&str]) -> Result<String, String> {
    let real = env!("CARGO_BIN_EXE_cluster_real");
    let tcp = env!("CARGO_BIN_EXE_montanha-tcp");
    let mut cmd = Command::new(real);
    cmd.env("MONTANHA_TCP", tcp)
        .arg(format!("0x{seed:x}"));
    for a in extra {
        cmd.arg(a);
    }
    let out = cmd.output().expect("spawn cluster_real");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let line = stdout
        .lines()
        .find(|l| l.starts_with("cluster_real "))
        .unwrap_or(stdout.trim())
        .to_string();
    if out.status.success() {
        Ok(line)
    } else {
        Err(format!(
            "status={:?} stdout={line} stderr={stderr}",
            out.status
        ))
    }
}

fn run_counted(seed: u64, extra: &[&str]) -> (String, u64) {
    let _gate = CLUSTER_REAL
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // Wall-tick elect + n3 leave catch-up is a campaign. One SIGKILL of n3
    // before leave lands is `napply=0`; retry the same seed, not a kernel skip.
    let mut last = String::new();
    for attempt in 1..=3 {
        match run_once(seed, extra) {
            Ok(line) => return (line, attempt),
            Err(e) => {
                last = e;
                eprintln!("cluster_real attempt {attempt}/3 failed: {last}");
            }
        }
    }
    panic!("cluster_real failed after 3 attempts: {last}");
}

fn run(seed: u64, extra: &[&str]) -> String {
    run_counted(seed, extra).0
}

#[test]
fn l28_real_tcp_seed_replay() {
    let seed = 0x0064_1E28_u64;
    let a = run(seed, &[]);
    let b = run(seed, &[]);
    assert_eq!(a, b, "REAL TCP cluster fingerprint must replay");
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss: {a}"
    );
    // RFC-0072 P1.2: World-clean (silent_wrong=0, same seed as World) is
    // not L28 unless cluster_real durability holds. AS-IS would skip TCP.
    assert!(
        world_seed_l28_ok(0, l28_durability_ok(get_ok, after_ok, restart_ok)),
        "World seed {seed:#x} must pass cluster_real: {a}"
    );
    assert!(
        world_seed_l28_ok_as_is(0, false),
        "AS-IS dente: World-clean without TCP would pass"
    );
    assert!(!world_seed_l28_ok(0, false));
    eprintln!("{a}");
}

/// Kill the **leader** (not a random member). Remaining majority must
/// re-elect and keep the acked value; reopen of the old leader must too.
#[test]
fn l28_real_tcp_leader_kill() {
    let seed = 0x0064_1E29_u64;
    let a = run(seed, &["--leader-kill"]);
    let b = run(seed, &["--leader-kill"]);
    assert_eq!(a, b, "leader-kill fingerprint must replay");
    assert!(a.contains("kill=leader"), "{a}");
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_leader_kill_ok(get_ok, after_ok, restart_ok),
        "L28 leader-kill kernel miss: {a}"
    );
    assert!(
        l28_leader_kill_ok_as_is(true, false, false),
        "AS-IS dente: ignore leader-kill"
    );
    eprintln!("{a}");
}

/// RFC-0118 P1.2: `cluster_real --leave-joint` fingerprint replays.
/// Leave is invoked after elect (no-op Ok when no joint is in flight).
/// Durability still L28. Committed-joint-then-leave is RFC-0066 P2.2 /
/// 0118 P1.1 (TCP add/remove wire is RFC-0119 P1.2).
#[test]
fn l28_real_tcp_leave_joint_replay() {
    let seed = 0x0118_1E28_u64;
    let a = run(seed, &["--leave-joint"]);
    let b = run(seed, &["--leave-joint"]);
    assert_eq!(a, b, "leave-joint fingerprint must replay");
    assert!(a.contains("leave=1"), "TCP leave must fire: {a}");
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --leave-joint: {a}"
    );
    assert!(
        l28_tcp_leave_ok(a.contains("leave=1")),
        "TCP leave kernel miss: {a}"
    );
    assert!(
        l28_tcp_leave_ok_as_is(false),
        "AS-IS dente: skip TCP leave"
    );
    eprintln!("{a}");
}

/// RFC-0121 P1.2 / 0066 P2.2: `cluster_real --remove-member` plants a
/// joint remove then leave; after process death the recovered log (or
/// compacted C-new membership) is C-new-only. Durability still L28.
#[test]
fn l28_real_tcp_remove_member_left_on_disk() {
    let seed = 0x0121_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "remove-member fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(a.contains("leave=1"), "TCP leave after plant must fire: {a}");
    assert!(a.contains("left=1"), "on-disk C-new-only must hold: {a}");
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_plant_ok(a.contains("remove=1"), a.contains("leave=1")),
        "TCP plant kernel miss: {a}"
    );
    assert!(
        l28_tcp_left_ok(a.contains("left=1")),
        "on-disk leave kernel miss: {a}"
    );
    assert!(
        l28_tcp_plant_ok_as_is(false, false),
        "AS-IS dente: skip TCP plant"
    );
    assert!(
        l28_tcp_left_ok_as_is(false),
        "AS-IS dente: skip on-disk C-new-only"
    );
    eprintln!("{a}");
}

/// RFC-0126 P1.2: after `--remove-member` plant, process death, disk
/// high-water still exceeds the live set (3 after shrink to 2).
#[test]
fn l28_real_tcp_high_water_after_remove() {
    let seed = 0x0126_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "high-water fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(a.contains("hw=1"), "on-disk high-water must hold: {a}");
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_hw_ok(a.contains("hw=1")),
        "on-disk high-water kernel miss: {a}"
    );
    assert!(
        l28_tcp_hw_ok_as_is(false),
        "AS-IS dente: skip on-disk high-water"
    );
    eprintln!("{a}");
}

/// RFC-0128 P1.2: after `--remove-member` plant, process death, TCP ctor
/// with stale CLI must not count the removed voter as participating.
#[test]
fn l28_real_tcp_participating_after_remove() {
    let seed = 0x0128_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "participating fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(a.contains("part=1"), "removed voter must not participate: {a}");
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_part_ok(a.contains("part=1")),
        "TCP participating kernel miss: {a}"
    );
    assert!(
        l28_tcp_part_ok_as_is(false),
        "AS-IS dente: skip TCP participating"
    );
    eprintln!("{a}");
}

/// RFC-0130 P1.2: after `--remove-member` plant, process death, TCP ctor
/// must recover-apply a planted committed-unapplied prefix.
#[test]
fn l28_real_tcp_recover_apply() {
    let seed = 0x0130_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "recover-apply fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains(" apply=1"),
        "recover apply must close the gap: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_apply_ok(a.contains(" apply=1")),
        "TCP recover-apply kernel miss: {a}"
    );
    assert!(
        l28_tcp_apply_ok_as_is(false),
        "AS-IS dente: skip TCP recover apply"
    );
    eprintln!("{a}");
}

/// RFC-0131 P1.2: after `--remove-member` plant, process death, TCP ctor
/// on the removed replica must recover-apply. 0130 voter apply is **not**
/// this tooth.
#[test]
fn l28_real_tcp_removed_recover_apply() {
    let seed = 0x0131_1E28_u64;
    let (a, attempts) = run_counted(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "removed-replica recover-apply fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("napply=1"),
        "removed replica recover apply must close the gap: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    let napply_ok = a.contains("napply=1");
    assert!(
        l28_tcp_napply_ok(napply_ok),
        "TCP removed-replica recover-apply kernel miss: {a}"
    );
    assert!(
        l28_tcp_napply_ok_as_is(false),
        "AS-IS dente: skip TCP removed-replica recover apply"
    );
    assert!(
        !l28_tcp_napply_retry_admitted(attempts, napply_ok),
        "retry-success is not forall TCP: attempts={attempts} napply_ok={napply_ok} {a}"
    );
    assert!(
        l28_tcp_napply_retry_admitted_as_is(1, true),
        "AS-IS dente: one successful napply would skip retry as forall"
    );
    eprintln!("{a}");
}

/// RFC-0156 P0.4 (R-swarm-real): multi-seed campaign on the shipped REAL
/// TCP path. Three fresh seeds, each requiring `napply=1`, the
/// `l28_durability` / `l28_tcp_napply` kernels, and the retry-refusal
/// on that seed's own attempt count. A 3-seed campaign is evidence, not
/// ∀ TCP — the residual row stays.
#[test]
fn l28_real_tcp_removed_campaign_seeds() {
    for seed in [0x0156_1E28_u64, 0x0157_1E28_u64, 0x0158_1E28_u64] {
        let (a, attempts) = run_counted(seed, &["--remove-member"]);
        assert!(a.contains("remove=1"), "seed {seed:#x}: remove plant must fire: {a}");
        assert!(
            a.contains("napply=1"),
            "seed {seed:#x}: removed replica recover apply must close the gap: {a}"
        );
        let (get_ok, after_ok, restart_ok) = parse_l28(&a);
        assert!(
            l28_durability_ok(get_ok, after_ok, restart_ok),
            "seed {seed:#x}: L28 kernel miss under --remove-member: {a}"
        );
        let napply_ok = a.contains("napply=1");
        assert!(
            l28_tcp_napply_ok(napply_ok),
            "seed {seed:#x}: TCP removed-replica recover-apply kernel miss: {a}"
        );
        assert!(
            !l28_tcp_napply_retry_admitted(attempts, napply_ok),
            "seed {seed:#x}: campaign success is not forall TCP: attempts={attempts} napply_ok={napply_ok} {a}"
        );
        eprintln!("campaign seed={seed:#x} attempts={attempts}: {a}");
    }
}

/// RFC-0132 P1.2: after `--remove-member` plant, process death, TCP ctor
/// on the removed replica must persist truncated log. 0131 apply is **not**
/// this tooth.
#[test]
fn l28_real_tcp_removed_truncate() {
    let seed = 0x0132_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "removed-replica truncate fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("trunc=1"),
        "removed replica truncate persist must fire: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_trunc_ok(a.contains("trunc=1")),
        "TCP removed-replica truncate kernel miss: {a}"
    );
    assert!(
        l28_tcp_trunc_ok_as_is(false),
        "AS-IS dente: skip TCP removed-replica truncate persist"
    );
    eprintln!("{a}");
}

/// RFC-0133 P1.2: after `--remove-member` plant, process death, TCP ctor
/// on the removed replica must drop orphan `log_entry_key`. 0132 log_hi
/// cap is **not** this tooth.
#[test]
fn l28_real_tcp_removed_orphan_drop() {
    let seed = 0x0133_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "removed-replica orphan-drop fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("odrop=1"),
        "removed replica orphan-segment drop must fire: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_odrop_ok(a.contains("odrop=1")),
        "TCP removed-replica orphan-drop kernel miss: {a}"
    );
    assert!(
        l28_tcp_odrop_ok_as_is(false),
        "AS-IS dente: skip TCP orphan-segment drop"
    );
    eprintln!("{a}");
}

/// RFC-0134 P1.2: after `--remove-member` plant, process death, TCP ctor
/// on the removed replica must abort leftover 2PC. 0133 orphan drop is
/// **not** this tooth.
#[test]
fn l28_real_tcp_removed_abort() {
    let seed = 0x0134_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "removed-replica abort fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("abort=1"),
        "removed replica leftover 2PC abort must fire: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_abort_ok(a.contains("abort=1")),
        "TCP removed-replica abort kernel miss: {a}"
    );
    assert!(
        l28_tcp_abort_ok_as_is(false),
        "AS-IS dente: skip TCP leftover 2PC abort"
    );
    eprintln!("{a}");
}

/// RFC-0135 P1.2: after `--remove-member` plant, process death, TCP ctor
/// on the removed replica must persist `now_ms`. 0134 abort is **not**
/// this tooth.
#[test]
fn l28_real_tcp_removed_now_ms() {
    let seed = 0x0135_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "removed-replica now_ms fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("nowms=1"),
        "removed replica now_ms persist must fire: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_nowms_ok(a.contains("nowms=1")),
        "TCP removed-replica now_ms kernel miss: {a}"
    );
    assert!(
        l28_tcp_nowms_ok_as_is(false),
        "AS-IS dente: skip TCP removed-replica now_ms persist"
    );
    eprintln!("{a}");
}

/// RFC-0136 P1.2: after `--remove-member` plant, process death, TCP ctor
/// on the removed replica must persist SI hist. 0135 now_ms is **not**
/// this tooth.
#[test]
fn l28_real_tcp_removed_hist() {
    let seed = 0x0136_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "removed-replica SI hist fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("hist=1"),
        "removed replica SI hist persist must fire: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_hist_ok(a.contains("hist=1")),
        "TCP removed-replica SI hist kernel miss: {a}"
    );
    assert!(
        l28_tcp_hist_ok_as_is(false),
        "AS-IS dente: skip TCP removed-replica SI hist persist"
    );
    eprintln!("{a}");
}

/// RFC-0137 P1.2: after `--remove-member` plant, process death, TCP ctor
/// on the removed replica must persist abort fence. 0136 SI hist is **not**
/// this tooth.
#[test]
fn l28_real_tcp_removed_fence() {
    let seed = 0x0137_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "removed-replica abort-fence fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("fence=1"),
        "removed replica abort-fence persist must fire: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_fence_ok(a.contains("fence=1")),
        "TCP removed-replica abort-fence kernel miss: {a}"
    );
    assert!(
        l28_tcp_fence_ok_as_is(false),
        "AS-IS dente: skip TCP removed-replica abort-fence persist"
    );
    eprintln!("{a}");
}

/// RFC-0138 P1.2: after `--remove-member` plant, process death, TCP ctor
/// on the removed replica must force-clear stuck intents. 0137 abort fence
/// is **not** this tooth.
#[test]
fn l28_real_tcp_removed_clear() {
    let seed = 0x0138_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "removed-replica force-clear fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("clear=1"),
        "removed replica force-local TX clear must fire: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_clear_ok(a.contains("clear=1")),
        "TCP removed-replica force-clear kernel miss: {a}"
    );
    assert!(
        l28_tcp_clear_ok_as_is(false),
        "AS-IS dente: skip TCP removed-replica force-local TX clear"
    );
    eprintln!("{a}");
}

/// RFC-0139 P1.2: after `--remove-member` plant, process death, TCP ctor
/// on the removed replica must drop leftover TX preimages. 0138 force-clear
/// is **not** this tooth.
#[test]
fn l28_real_tcp_removed_pre() {
    let seed = 0x0139_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "removed-replica drop-preimages fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("pre=1"),
        "removed replica drop-preimages must fire: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_pre_ok(a.contains("pre=1")),
        "TCP removed-replica drop-preimages kernel miss: {a}"
    );
    assert!(
        l28_tcp_pre_ok_as_is(false),
        "AS-IS dente: skip TCP removed-replica drop-preimages"
    );
    eprintln!("{a}");
}

/// RFC-0140 P1.2: after `--remove-member` plant, process death, TCP ctor
/// on the removed replica must load RangePeer from disk C-new (election
/// timeout ≠ stale CLI). 0139 drop-preimages is **not** this tooth.
#[test]
fn l28_real_tcp_removed_peer() {
    let seed = 0x0140_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "removed-replica disk-peer fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("peer=1"),
        "removed replica disk-peer timeout must fire: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_peer_ok(a.contains("peer=1")),
        "TCP removed-replica disk-peer kernel miss: {a}"
    );
    assert!(
        l28_tcp_peer_ok_as_is(false),
        "AS-IS dente: skip TCP disk-peer election timeout"
    );
    eprintln!("{a}");
}

/// RFC-0141 P1.2: after `--remove-member` plant, process death, TCP ctor
/// on the removed replica must not treat HashMap first-key as identity.
/// 0140 timeout peek is **not** this tooth.
#[test]
fn l28_real_tcp_removed_lid() {
    let seed = 0x0141_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "removed-replica local-id fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("lid=1"),
        "removed replica local-id gate must fire: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_lid_ok(a.contains("lid=1")),
        "TCP removed-replica local-id kernel miss: {a}"
    );
    assert!(
        l28_tcp_lid_ok_as_is(false),
        "AS-IS dente: skip TCP removed-replica local-id gate"
    );
    eprintln!("{a}");
}

/// RFC-0142 P1.2: after `--remove-member` plant, process death, TCP ctor
/// on the removed replica must not pick remote `ids.first()` as a
/// LocalApplied reader (`empty`, not `bad node`). 0141 local-id is
/// **not** this tooth.
#[test]
fn l28_real_tcp_removed_rdr() {
    let seed = 0x0142_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "removed-replica reader-local fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("rdr=1"),
        "removed replica reader-local gate must fire: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_rdr_ok(a.contains("rdr=1")),
        "TCP removed-replica reader-local kernel miss: {a}"
    );
    assert!(
        l28_tcp_rdr_ok_as_is(false),
        "AS-IS dente: skip TCP removed-replica reader-local gate"
    );
    eprintln!("{a}");
}

/// RFC-0143 P1.2: after `--remove-member` plant, process death, TCP ctor
/// on the removed replica must live-discard an uncommitted suffix. 0132
/// recover truncate is **not** this tooth.
#[test]
fn l28_real_tcp_removed_dsc() {
    let seed = 0x0143_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "removed-replica live-discard fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("dsc=1"),
        "removed replica live discard must fire: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_dsc_ok(a.contains("dsc=1")),
        "TCP removed-replica live-discard kernel miss: {a}"
    );
    assert!(
        l28_tcp_dsc_ok_as_is(false),
        "AS-IS dente: skip TCP removed-replica live discard"
    );
    eprintln!("{a}");
}

/// RFC-0144 P1.2: after `--remove-member` plant, process death, TCP ctor
/// on the removed replica must pick a local persist-leader on no-leader
/// abort (`next_index` repair). 0143 live discard is **not** this tooth.
#[test]
fn l28_real_tcp_removed_pld() {
    let seed = 0x0144_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(
        a, b,
        "removed-replica persist-leader fingerprint must replay"
    );
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("pld=1"),
        "removed replica persist-leader locality must fire: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_pld_ok(a.contains("pld=1")),
        "TCP removed-replica persist-leader kernel miss: {a}"
    );
    assert!(
        l28_tcp_pld_ok_as_is(false),
        "AS-IS dente: skip TCP persist-leader locality"
    );
    eprintln!("{a}");
}

/// RFC-0145 P1.2: after `--remove-member` plant, process death, TCP ctor
/// on the removed replica must step a planted Leader down on C-new
/// re-install. 0144 persist-leader is **not** this tooth.
#[test]
fn l28_real_tcp_removed_std() {
    let seed = 0x0145_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "removed-replica step-down fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("std=1"),
        "removed replica Leader step-down must fire: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_std_ok(a.contains("std=1")),
        "TCP removed-replica step-down kernel miss: {a}"
    );
    assert!(
        l28_tcp_std_ok_as_is(false),
        "AS-IS dente: skip TCP removed-replica Leader step-down"
    );
    eprintln!("{a}");
}

/// RFC-0146 P1.2: after `--remove-member` plant, process death, TCP ctor
/// of a remaining voter must not route `leader_hint` to the removed
/// replica. 0145 step-down is **not** this tooth.
#[test]
fn l28_real_tcp_hint() {
    let seed = 0x0146_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "remaining-voter leader-hint fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("hnt=1"),
        "remaining voter leader-hint must omit removed: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_hnt_ok(a.contains("hnt=1")),
        "TCP remaining-voter leader-hint kernel miss: {a}"
    );
    assert!(
        l28_tcp_hnt_ok_as_is(false),
        "AS-IS dente: skip TCP leader-hint membership filter"
    );
    eprintln!("{a}");
}

/// RFC-0147 P1.2: after `--remove-member` plant, process death, TCP ctor
/// of a remaining voter must forget next/match/sent_through of the
/// removed replica. 0146 hint is **not** this tooth.
#[test]
fn l28_real_tcp_drop_repl() {
    let seed = 0x0147_1E28_u64;
    let a = run(seed, &["--remove-member"]);
    let b = run(seed, &["--remove-member"]);
    assert_eq!(a, b, "remaining-voter repl-slot fingerprint must replay");
    assert!(a.contains("remove=1"), "TCP remove plant must fire: {a}");
    assert!(
        a.contains("slot=1"),
        "remaining voter must drop removed repl slots: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss under --remove-member: {a}"
    );
    assert!(
        l28_tcp_slot_ok(a.contains("slot=1")),
        "TCP remaining-voter repl-slot kernel miss: {a}"
    );
    assert!(
        l28_tcp_slot_ok_as_is(false),
        "AS-IS dente: skip TCP remaining-voter repl-slot drop"
    );
    eprintln!("{a}");
}

/// RFC-0148 P1.2: after REAL TCP process death, TCP ctor of a remaining
/// 3-node voter must forget `sent_through` of a remote replica on oob
/// `remove_member`. 0147 joint slot drop is **not** this tooth.
#[test]
fn l28_real_tcp_drop_st() {
    let seed = 0x0148_1E28_u64;
    let a = run(seed, &[]);
    let b = run(seed, &[]);
    assert_eq!(a, b, "remaining-voter oob sent_through fingerprint must replay");
    assert!(
        a.contains("sth=1"),
        "remaining voter oob remove must drop remote sent_through: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss: {a}"
    );
    assert!(
        l28_tcp_sth_ok(a.contains("sth=1")),
        "TCP remaining-voter oob sent_through kernel miss: {a}"
    );
    assert!(
        l28_tcp_sth_ok_as_is(false),
        "AS-IS dente: skip TCP remaining-voter sent_through drop"
    );
    eprintln!("{a}");
}

/// RFC-0068 P2.2: after REAL TCP process death, TCP ctor of a 3-node
/// voter with a planted committed C-old,new (no leave) must refuse a
/// C-old majority elect. 0148 oob sent_through is **not** this tooth.
#[test]
fn l28_real_tcp_plant_joint() {
    let seed = 0x0068_1E28_u64;
    let a = run(seed, &[]);
    let b = run(seed, &[]);
    assert_eq!(a, b, "planted committed-joint fingerprint must replay");
    assert!(
        a.contains("pj=1"),
        "planted committed joint must refuse C-old majority: {a}"
    );
    let (get_ok, after_ok, restart_ok) = parse_l28(&a);
    assert!(
        l28_durability_ok(get_ok, after_ok, restart_ok),
        "L28 kernel miss: {a}"
    );
    assert!(
        l28_tcp_pj_ok(a.contains("pj=1")),
        "TCP planted committed-joint kernel miss: {a}"
    );
    assert!(
        l28_tcp_pj_ok_as_is(false),
        "AS-IS dente: skip TCP planted committed-joint-without-leave"
    );
    eprintln!("{a}");
}
