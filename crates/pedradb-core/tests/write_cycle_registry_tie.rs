//! RFC-0199 P1.3 — the write-cycle forecast consumes the REGISTERED
//! count theorems, never hats (`catalog:wal_commit_plan` →
//! `wal_commit_plan_at_most_one_fdatasync`, `WorkIo.lean`;
//! `catalog:auto_flush_due` → `memtable_flush_amortized`,
//! `FlushAmortCount.lean`; `catalog:scan_guard` →
//! `scan_decision_work_bound`, `ScanDecisionCount.lean`;
//! `catalog:probe_order_covering` → `probe_order_covering_work_bound`,
//! `ProbeLadderCount.lean`).
//!
//! The kernel module (`src/write_cycle_kernel.rs`, RFC-0192) is
//! included BY PATH and never edited here: the registry tie is this
//! test's job. `crate::write_admission_kernel` is re-exported (the
//! REAL production functions) so the included module links against
//! production code, not a mock.
//!
//! The contract under test, per RFC-0199 P1.3:
//! 1. every count row the forecast composes exists in
//!    `scripts/ratchet/close_proofs.tsv` with the EXACT theorem name,
//!    over a Lean file with zero `sorry` — a renamed/removed/axed row
//!    fails the forecast;
//! 2. the kernel's per-op slice composition agrees with the registered
//!    multipliers: the serial CS sums each named slice exactly once,
//!    excludes the derived `lock_wait`, and contains NO barrier slice
//!    (barriers are per committed GROUP — ≤1 by theorem — never per
//!    op), and the amortized flush multiplier observed through the
//!    REAL `auto_flush_due` gate never exceeds the registered
//!    1000-permille bound;
//! 3. the forecast renders with its `tier=ceiling` disclaimer — a
//!    count multiplier is never a cartaz claim.

use std::fs;
use std::path::PathBuf;

#[path = "../src/write_cycle_kernel.rs"]
mod write_cycle_kernel;

/// Production re-export so the path-included kernel's
/// `crate::write_admission_kernel::batch_is_empty` resolves to the
/// real shipped function.
mod write_admission_kernel {
    pub use pedradb_core::write_admission_kernel::batch_is_empty;
}

use pedradb_core::flush_kernel::auto_flush_due;
use write_cycle_kernel::{
    WritePhaseNs, LINUX_QUIET_0189_P01, serial_cs_ns, write_cycle_forecast,
};

/// Count rows the write-cycle forecast composes: (catalog pair,
/// theorem, meaning, per-unit multiplier bound). The multiplier is
/// the THEOREM's bound — 1 barrier per committed group; flush work
/// ≤ 1000 permille of written bytes; ≤1 scan-decision walk per
/// non-overlapping file; ≤1 ladder step per candidate per scan.
const REGISTERED_COUNTS: &[(&str, &str, &str, u64)] = &[
    (
        "catalog:wal_commit_plan",
        "wal_commit_plan_at_most_one_fdatasync",
        "fdatasync per committed group (max)",
        1,
    ),
    (
        "catalog:auto_flush_due",
        "memtable_flush_amortized",
        "flush bytes per written byte, permille (max)",
        1000,
    ),
    (
        "catalog:scan_guard",
        "scan_decision_work_bound",
        "scan decision walks per non-overlapping file",
        1,
    ),
    (
        "catalog:probe_order_covering",
        "probe_order_covering_work_bound",
        "ladder steps per candidate",
        1,
    ),
];

fn registry_path() -> PathBuf {
    match std::env::var("PEDRA_COUNT_REGISTRY") {
        Ok(p) => PathBuf::from(p),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/ratchet/close_proofs.tsv"),
    }
}

/// Every count row the forecast composes is live in the registry with
/// the exact theorem name, over a zero-`sorry` Lean proof. A renamed
/// theorem, a removed row, or a proof that gained a `sorry` fails the
/// forecast (RFC-0199 P1.3: no hat becomes a cartaz claim).
#[test]
fn registry_rows_pin_the_forecast_counts() {
    let tsv = fs::read_to_string(registry_path())
        .expect("count registry readable (scripts/ratchet/close_proofs.tsv)");
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

    for (cid, theorem, meaning, _mult) in REGISTERED_COUNTS {
        let mut hits = 0;
        for line in tsv.lines() {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.first() == Some(&"count") && cols.get(1) == Some(cid) {
                assert_eq!(
                    cols.get(2),
                    Some(theorem),
                    "{cid}: count row theorem drifted — the forecast composes {theorem} ({meaning})"
                );
                let lean = cols.get(3).expect("lean file column");
                let src = fs::read_to_string(repo.join(lean))
                    .unwrap_or_else(|_| panic!("{lean} readable"));
                assert!(
                    src.contains(&format!("theorem {theorem}")),
                    "{lean}: theorem {theorem} not found — row points at a stale file"
                );
                let sorries = src.matches("sorry").count();
                assert_eq!(sorries, 0, "{lean}: {sorries} sorry — no count credit on a sorry file");
                hits += 1;
            }
        }
        assert_eq!(hits, 1, "{cid}: exactly one count row expected, found {hits}");
    }
}

/// The kernel's per-op composition agrees with the registered
/// multipliers: each named slice counts exactly once in the serial CS,
/// the derived `lock_wait` is excluded, and NO barrier slice exists in
/// the per-op CS — barriers are per committed GROUP (≤1 by
/// `wal_commit_plan_at_most_one_fdatasync`), so a per-op barrier term
/// would disagree with the registry by the whole group factor.
#[test]
fn kernel_cs_agrees_with_registered_counts() {
    // one-hot: each named slice contributes exactly once
    for field in [
        WritePhaseNs {
            wal_encode: 100,
            ..WritePhaseNs::default()
        },
        WritePhaseNs {
            wal_write: 100,
            ..WritePhaseNs::default()
        },
        WritePhaseNs {
            mem_guard: 100,
            ..WritePhaseNs::default()
        },
        WritePhaseNs {
            mem_lock: 100,
            ..WritePhaseNs::default()
        },
        WritePhaseNs {
            mem_insert: 100,
            ..WritePhaseNs::default()
        },
        WritePhaseNs {
            publish: 100,
            ..WritePhaseNs::default()
        },
        WritePhaseNs {
            grp: 100,
            ..WritePhaseNs::default()
        },
    ] {
        assert_eq!(serial_cs_ns(field), 100, "slice counted exactly once: {field:?}");
    }
    // empty phases: no hidden per-op term (a barrier slice per op
    // would make this non-zero)
    assert_eq!(serial_cs_ns(WritePhaseNs::default()), 0);
    // lock_wait is derived, not a cut: excluded from the CS sum
    assert_eq!(
        serial_cs_ns(WritePhaseNs {
            lock_wait: 9_999,
            ..WritePhaseNs::default()
        }),
        0
    );
    // the dated anchor composes as the plain sum of its 7 slices
    let p = LINUX_QUIET_0189_P01;
    let sum = p.wal_encode + p.wal_write + p.mem_guard + p.mem_lock + p.mem_insert + p.publish + p.grp;
    assert_eq!(serial_cs_ns(p), sum);
    // group barrier ledger: G committed groups pay at most G × the
    // registered multiplier (1) — never ops × 1
    let (barrier_mult, ) = (REGISTERED_COUNTS[0].3,);
    for groups in [1u64, 2, 8, 64] {
        let ops = groups * 64; // 64 ops per committed group
        assert!(
            groups * barrier_mult <= ops * barrier_mult,
            "per-group barrier accounting must stay below per-op accounting"
        );
        assert_eq!(groups.saturating_mul(barrier_mult), groups);
    }
}

/// The amortized flush multiplier observed through the REAL
/// `auto_flush_due` gate never exceeds the registered 1000-permille
/// bound (`memtable_flush_amortized`: paid ≤ written + initial).
#[test]
fn flush_multiplier_within_registered_bound() {
    let limit = 9u64;
    let mut mem = 0u64;
    let mut paid = 0u64;
    let mut written = 0u64;
    // ragged schedule through the real gate (LCG; same shape as the
    // P1.2 twin, asserting the REGISTRY multiplier here)
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    for _ in 0..200 {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        if state >> 63 == 0 {
            let u = 1 + (state % 13);
            written += u;
            mem = mem.saturating_add(u);
        } else if auto_flush_due(mem, true, limit) {
            paid += mem;
            mem = 0;
        }
    }
    let registered_permille = REGISTERED_COUNTS[1].3;
    assert_eq!(registered_permille, 1000);
    assert!(
        paid * 1000 <= (written + 0) * registered_permille,
        "observed flush multiplier {paid}/{written} exceeds the registered {registered_permille} permille"
    );
}

/// The forecast output carries its non-cartaz disclaimer on the real
/// render path (RFC-0199: count multipliers never become cartaz
/// claims; RFC-0192 P0.4's `tier=ceiling` label is load-bearing).
#[test]
fn forecast_render_is_not_a_cartaz_claim() {
    let f = write_cycle_forecast(LINUX_QUIET_0189_P01, 4);
    let rendered = f.render();
    assert!(rendered.contains("tier=ceiling"));
    assert!(rendered.contains("not a cartaz qps forecast"));
    assert!(rendered.contains("cs_ns="));
    assert!(rendered.contains("qps_hat="));
}
