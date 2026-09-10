//! RFC-0187 P0.3 — exhaustive crash-injection gate (blocking, self-verifying).
//!
//! Enumerates every crash point of a fixed workload through the real
//! fault seam (`FailingEnvArc`), one injection per op index, and checks
//! the fail-closed recovery oracle after each:
//!
//! 1. **count**: the total fallible-op count T (and the sync-class count
//!    S, printed for the P0.4 barrier tie) is MEASURED by binary search
//!    over `tripped()` on the same `FailingEnvArc` that injects — zero
//!    drift with the gated op set by construction. Injections loop the
//!    full `0..T` and each must actually FIRE (`tripped()`), so a shrunk
//!    or drifting op stream goes red.
//! 2. **crash oracle** (per injected index): abrupt death at the first
//!    failing op (`mem::forget` — no Drop flush), reopen with `StdEnv`
//!    must be `Ok` (the fault returned an error; nothing was torn), and
//!    — acked writes survive (TX committed Ok ⇒ both members present,
//!    correct value); — every TX is all-or-nothing (present or absent,
//!    never half); — every present value is correct (no silent-wrong).
//!    Unacked keys may be present or absent (group-commit may have
//!    durably grouped them before the fault).
//! 3. **CRC fail-closed** (corruption phase): byte flips at fixed
//!    offsets of a fully-acked WAL copy must reopen refused or Ok with
//!    correct values only — a flipped frame may never surface as wrong
//!    data (CRC fail-closed, not fail-silent).
//!
//! `--selftest` proves oracle redness without touching production code:
//! acked-lost, half-TX, wrong-value must each be caught; healthy early
//! and late crash states must pass (an always-red oracle is also a bug).
//!
//! Piso: this proves recovery under injected op failure + process death
//! at every op INDEX of this workload — it is not a theorem over torn
//! sector writes (TCG nightly, P2.2) nor ∀π (R-pct / R-glue stand).

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use pedradb_core::{ConcurrentDb, OpenOptions, WAL_FILE_NAME};
use pedradb_sim::{FailingEnvArc, FaultKind};

static PROBE_N: AtomicU64 = AtomicU64::new(0);

const KEYS_TX1: [&str; 2] = ["crash/a1", "crash/b1"];
const KEY_SINGLE: &str = "crash/s1";
const KEYS_TX2: [&str; 2] = ["crash/c1", "crash/d1"];
const ALL_KEYS: [&str; 5] = ["crash/a1", "crash/b1", "crash/s1", "crash/c1", "crash/d1"];

/// Workload progress when the op stream died. `acked_*` = the commit
/// returned Ok BEFORE the fault (those MUST survive any later crash).
#[derive(Default, Debug)]
struct Outcomes {
    acked_tx1: bool,
    acked_single: bool,
    acked_tx2: bool,
    crashed: bool,
    detail: String,
}

/// Drive the fixed workload sequentially; stop at the first failing op
/// (the injected fault = the crash point). Panics are caught by the
/// caller; a panic loses the partial ack state and is treated as
/// "nothing acked" (conservative for the acked-survives rule).
fn drive_workload(db: &ConcurrentDb<FailingEnvArc>) -> Outcomes {
    let mut o = Outcomes::default();
    let mut tx = db.begin_occ();
    if let Err(e) = tx.put(KEYS_TX1[0].as_bytes(), b"1") {
        o.crashed = true;
        o.detail = format!("tx1.put: {e}");
        return o;
    }
    if let Err(e) = tx.put(KEYS_TX1[1].as_bytes(), b"1") {
        o.crashed = true;
        o.detail = format!("tx1.put2: {e}");
        return o;
    }
    match tx.commit() {
        Ok(()) => o.acked_tx1 = true,
        Err(e) => {
            o.crashed = true;
            o.detail = format!("tx1.commit: {e}");
            return o;
        }
    }
    match db.put(KEY_SINGLE.as_bytes(), b"1") {
        Ok(()) => o.acked_single = true,
        Err(e) => {
            o.crashed = true;
            o.detail = format!("single.put: {e}");
            return o;
        }
    }
    let mut tx = db.begin_occ();
    if let Err(e) = tx.put(KEYS_TX2[0].as_bytes(), b"1") {
        o.crashed = true;
        o.detail = format!("tx2.put: {e}");
        return o;
    }
    if let Err(e) = tx.put(KEYS_TX2[1].as_bytes(), b"1") {
        o.crashed = true;
        o.detail = format!("tx2.put2: {e}");
        return o;
    }
    match tx.commit() {
        Ok(()) => o.acked_tx2 = true,
        Err(e) => {
            o.crashed = true;
            o.detail = format!("tx2.commit: {e}");
        }
    }
    o
}

fn fresh_dir(base: &Path, tag: &str) -> PathBuf {
    let dir = base.join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// One probe run: fresh dir, open passing, arm AFTER open, drive the
/// workload (stopping at the fault), report whether the fault FIRED.
fn probe(base: &Path, after: u64, kind: FaultKind) -> bool {
    let dir = fresh_dir(base, &format!("probe-{}", PROBE_N.fetch_add(1, Ordering::Relaxed)));
    let env = FailingEnvArc::passing();
    let opts = OpenOptions { sync: true, ..OpenOptions::default() };
    let db = ConcurrentDb::open_with_env(&dir, opts, env.clone()).expect("probe open");
    db.set_write_group_catchup_window(std::time::Duration::ZERO);
    env.arm_with_kind(after, true, kind);
    let _ = catch_unwind(AssertUnwindSafe(|| drive_workload(&db)));
    let tripped = env.tripped();
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
    tripped
}

/// Total gated ops for this workload+kind, measured with the SAME
/// `FailingEnvArc` that the injections use (binary search over
/// `tripped()`): T = 1 + max{ n : workload has more than n gated ops }.
fn count_ops(base: &Path, kind: FaultKind) -> Result<usize, String> {
    if !probe(base, 0, kind) {
        return Err("workload performs no gated ops — seam bypassed".into());
    }
    let mut hi: u64 = 1;
    while probe(base, hi, kind) {
        hi = hi.checked_mul(2).ok_or("probe bound overflow")?;
        if hi > 8192 {
            return Err(format!("workload exceeds {hi} gated ops — not a bounded gate scenario"));
        }
    }
    let mut lo: u64 = 0; // probe(0) is true (checked above)
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if probe(base, mid, kind) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok((lo + 1) as usize)
}

/// Pure crash oracle. `observed` = (key, Some(value) | None) for every
/// `ALL_KEYS` entry after reopen. Checks, in order: acked-survives,
/// TX all-or-nothing, every-present-value-correct.
fn check_crash_state(
    observed: &[(&str, Option<&str>)],
    o: &Outcomes,
) -> Result<(), String> {
    let present = |k: &str| {
        observed.iter().find(|(ok, _)| *ok == k).is_some_and(|(_, v)| v.is_some())
    };
    let value = |k: &str| observed.iter().find(|(ok, _)| *ok == k).and_then(|(_, v)| *v);

    if o.acked_tx1 && !(present(KEYS_TX1[0]) && present(KEYS_TX1[1])) {
        return Err(format!(
            "acked write lost: tx1 ({} present, {} present)",
            KEYS_TX1[0],
            present(KEYS_TX1[0])
        ));
    }
    if o.acked_single && !present(KEY_SINGLE) {
        return Err("acked write lost: single".into());
    }
    if o.acked_tx2 && !(present(KEYS_TX2[0]) && present(KEYS_TX2[1])) {
        return Err("acked write lost: tx2".into());
    }
    if present(KEYS_TX1[0]) != present(KEYS_TX1[1]) {
        return Err(format!("tx1 half-present ({} only)", if present(KEYS_TX1[0]) { KEYS_TX1[0] } else { KEYS_TX1[1] }));
    }
    if present(KEYS_TX2[0]) != present(KEYS_TX2[1]) {
        return Err(format!("tx2 half-present ({} only)", if present(KEYS_TX2[0]) { KEYS_TX2[0] } else { KEYS_TX2[1] }));
    }
    for k in ALL_KEYS {
        if present(k) && value(k) != Some("1") {
            return Err(format!("silent-wrong: {k} = {:?}", value(k)));
        }
    }
    Ok(())
}

/// Pure corruption oracle (CRC fail-closed): after a byte flip anywhere
/// in the WAL, reopen may refuse or serve — but a served key must carry
/// the correct value, and a TX must never surface half.
fn check_corrupt_state(observed: &[(&str, Option<&str>)]) -> Result<(), String> {
    let present = |k: &str| {
        observed.iter().find(|(ok, _)| *ok == k).is_some_and(|(_, v)| v.is_some())
    };
    let value = |k: &str| observed.iter().find(|(ok, _)| *ok == k).and_then(|(_, v)| *v);
    for pair in [KEYS_TX1, KEYS_TX2] {
        if present(pair[0]) != present(pair[1]) {
            return Err(format!("half TX after corruption ({} only)", if present(pair[0]) { pair[0] } else { pair[1] }));
        }
    }
    for k in ALL_KEYS {
        if present(k) && value(k) != Some("1") {
            return Err(format!("silent-wrong after corruption: {k} = {:?}", value(k)));
        }
    }
    Ok(())
}

fn read_state(dir: &Path) -> Result<Vec<(&'static str, Option<String>)>, String> {
    let re = ConcurrentDb::open_with_env(dir, OpenOptions::default(), pedradb_core::StdEnv)
        .map_err(|e| format!("reopen failed: {e}"))?;
    let mut out = Vec::new();
    for k in ALL_KEYS {
        out.push((k, re.get(k.as_bytes()).map(|v| String::from_utf8_lossy(&v).into_owned())));
    }
    Ok(out)
}

/// One injected crash: op `i` fails, workload stops, abrupt death
/// (`mem::forget` — Drop never flushes), reopen + oracle.
fn crash_run(base: &Path, i: u64) -> Result<(), String> {
    let dir = fresh_dir(base, &format!("crash-{i}"));
    let env = FailingEnvArc::passing();
    let opts = OpenOptions { sync: true, ..OpenOptions::default() };
    let db = ConcurrentDb::open_with_env(&dir, opts, env.clone())
        .map_err(|e| format!("open: {e}"))?;
    db.set_write_group_catchup_window(std::time::Duration::ZERO);
    // Alternating errno classes for coverage; both are non-sync ops in
    // the same gated stream, so the index space is identical.
    let kind = if i % 2 == 0 { FaultKind::IoError } else { FaultKind::ShortWrite };
    env.arm_with_kind(i, true, kind);
    let outcomes = catch_unwind(AssertUnwindSafe(|| drive_workload(&db))).unwrap_or_default();
    if !env.tripped() {
        return Err(format!(
            "injection {i}: fault never fired — op stream shorter than counted (drift)"
        ));
    }
    // Abrupt death: skip Drop so no flush path runs after the crash.
    std::mem::forget(db);
    let observed = read_state(&dir)?;
    let static_observed: Vec<(&str, Option<&str>)> =
        observed.iter().map(|(k, v)| (*k, v.as_deref())).collect();
    check_crash_state(&static_observed, &outcomes)?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// Corruption phase: flip one byte of a fully-acked WAL at `off`, reopen,
/// fail-closed oracle. `Ok(true)` = reopened; `Ok(false)` = refused.
fn corrupt_run(base: &Path, ref_dir: &Path, off: u64) -> Result<bool, String> {
    let dir = fresh_dir(base, &format!("corrupt-{off}"));
    copy_dir(ref_dir, &dir)?;
    let wal = dir.join(WAL_FILE_NAME);
    let len = std::fs::metadata(&wal).map_err(|e| format!("wal stat: {e}"))?.len();
    if off >= len {
        return Ok(true); // offset beyond this WAL — nothing to flip
    }
    let mut bytes = std::fs::read(&wal).map_err(|e| format!("wal read: {e}"))?;
    bytes[off as usize] ^= 0xFF;
    std::fs::write(&wal, &bytes).map_err(|e| format!("wal write: {e}"))?;
    match read_state(&dir) {
        Ok(observed) => {
            let static_observed: Vec<(&str, Option<&str>)> =
                observed.iter().map(|(k, v)| (*k, v.as_deref())).collect();
            check_corrupt_state(&static_observed)?;
            Ok(true)
        }
        // Refused reopen is fail-closed and allowed under corruption.
        Err(_) => Ok(false),
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("mkdir: {e}"))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("readdir: {e}"))? {
        let entry = entry.map_err(|e| format!("dirent: {e}"))?;
        std::fs::copy(entry.path(), to.join(entry.file_name()))
            .map_err(|e| format!("copy: {e}"))?;
    }
    Ok(())
}

/// Reference pass: passing env, whole workload acks. Returns the dir
/// (kept for the corruption phase).
fn reference_pass(base: &Path) -> Result<(PathBuf, usize), String> {
    let dir = fresh_dir(base, "reference");
    let env = FailingEnvArc::passing();
    let opts = OpenOptions { sync: true, ..OpenOptions::default() };
    let db = ConcurrentDb::open_with_env(&dir, opts, env.clone())
        .map_err(|e| format!("reference open: {e}"))?;
    db.set_write_group_catchup_window(std::time::Duration::ZERO);
    let o = drive_workload(&db);
    if o.crashed || !o.acked_tx1 || !o.acked_single || !o.acked_tx2 {
        return Err(format!("reference workload did not fully ack: {o:?}"));
    }
    let syncs = {
        // Count sync-class ops of the reference pass by re-measuring with
        // the sync-only kind — printed for the P0.4 barrier tie.
        count_ops(base, FaultKind::SyncFail)?
    };
    drop(db);
    Ok((dir, syncs))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--selftest") {
        std::process::exit(selftest());
    }

    // Keep gate logs clean: panics are outcomes (caught), not noise.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let base = std::env::temp_dir().join(format!("pedra-gate-crash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();

    let (ref_dir, sync_ops) = match reference_pass(&base) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("GATE crash: FAIL — {e}");
            eprintln!("GATE crash: RED");
            std::panic::set_hook(prev_hook);
            std::process::exit(1);
        }
    };
    let t_total = match count_ops(&base, FaultKind::IoError) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("GATE crash: FAIL — count: {e}");
            eprintln!("GATE crash: RED");
            std::panic::set_hook(prev_hook);
            std::process::exit(1);
        }
    };
    println!("CRASH_OPS total={t_total} sync={sync_ops} (binary-search over tripped)");

    let mut failures = 0usize;
    for i in 0..t_total as u64 {
        match crash_run(&base, i) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("GATE crash: FAIL — injection {i}/{t_total}: {e}");
                failures += 1;
            }
        }
    }

    // Corruption phase: flips at fixed fractions of the fully-acked WAL.
    let wal_len = std::fs::metadata(ref_dir.join(WAL_FILE_NAME)).map(|m| m.len()).unwrap_or(0);
    let mut flips_refused = 0usize;
    let mut flips_served = 0usize;
    let offsets: Vec<u64> = [1u64, 8, wal_len / 4, wal_len / 2, 3 * wal_len / 4, wal_len.saturating_sub(8), wal_len.saturating_sub(1)]
        .into_iter()
        .filter(|&o| o < wal_len)
        .collect();
    for off in offsets {
        match corrupt_run(&base, &ref_dir, off) {
            Ok(true) => flips_served += 1,
            Ok(false) => flips_refused += 1,
            Err(e) => {
                eprintln!("GATE crash: FAIL — corruption @{off}: {e}");
                failures += 1;
            }
        }
    }
    println!(
        "CORRUPTION flips={} served={} refused={}",
        flips_served + flips_refused,
        flips_served,
        flips_refused
    );

    std::panic::set_hook(prev_hook);
    let _ = std::fs::remove_dir_all(&base);
    if failures > 0 {
        eprintln!("GATE crash: RED");
        std::process::exit(1);
    }
    println!("GATE crash: GREEN — {t_total}/{t_total} crash points injected (each fired), fail-closed oracle clean, count asserted");
}

/// Prove the oracles can fail (and pass) on synthetic states.
fn selftest() -> i32 {
    let mut caught = 0;
    let mut total = 0;
    let ok_outcomes = Outcomes { acked_tx1: true, acked_single: true, acked_tx2: true, ..Default::default() };

    fn state(vals: [Option<&str>; 5]) -> Vec<(&'static str, Option<&str>)> {
        ALL_KEYS.iter().zip(vals).map(|(k, v)| (*k, v)).collect()
    }

    // S1: acked write lost.
    total += 1;
    let s = state([None, None, Some("1"), Some("1"), Some("1")]);
    if check_crash_state(&s, &ok_outcomes).is_err() {
        println!("SELFTEST crash: caught=acked-lost");
        caught += 1;
    } else {
        eprintln!("SELFTEST crash: MISSED acked-lost");
    }

    // S2: half TX (unacked — all-or-nothing must still hold).
    total += 1;
    let s = state([Some("1"), None, None, None, None]);
    if check_crash_state(&s, &Outcomes::default()).is_err() {
        println!("SELFTEST crash: caught=half-tx");
        caught += 1;
    } else {
        eprintln!("SELFTEST crash: MISSED half-tx");
    }

    // S3: silent-wrong value.
    total += 1;
    let s = state([Some("1"), Some("1"), Some("X"), None, None]);
    if check_crash_state(&s, &Outcomes { acked_tx1: true, ..Default::default() }).is_err() {
        println!("SELFTEST crash: caught=silent-wrong");
        caught += 1;
    } else {
        eprintln!("SELFTEST crash: MISSED silent-wrong");
    }

    // S4: healthy late crash (everything acked and present) must PASS.
    total += 1;
    let s = state([Some("1"), Some("1"), Some("1"), Some("1"), Some("1")]);
    if check_crash_state(&s, &ok_outcomes).is_ok() {
        println!("SELFTEST crash: passed=healthy-late (oracle not always-red)");
        caught += 1;
    } else {
        eprintln!("SELFTEST crash: healthy-late state REJECTED — oracle always-red");
    }

    // S5: healthy early crash (nothing acked, single unacked present) must PASS.
    total += 1;
    let s = state([None, None, Some("1"), None, None]);
    if check_crash_state(&s, &Outcomes::default()).is_ok() {
        println!("SELFTEST crash: passed=healthy-early");
        caught += 1;
    } else {
        eprintln!("SELFTEST crash: healthy-early state REJECTED — oracle always-red");
    }

    // S6: corruption oracle — silent-wrong after flip.
    total += 1;
    let s = state([Some("1"), Some("1"), Some("9"), Some("1"), Some("1")]);
    if check_corrupt_state(&s).is_err() {
        println!("SELFTEST crash: caught=corrupt-silent-wrong");
        caught += 1;
    } else {
        eprintln!("SELFTEST crash: MISSED corrupt-silent-wrong");
    }

    println!("SELFTEST crash: {caught}/{total} oracle checks caught");
    if caught == total { 0 } else { 1 }
}
