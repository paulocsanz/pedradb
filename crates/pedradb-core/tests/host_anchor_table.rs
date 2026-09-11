//! RFC-0203 P1.2 — consumption test: `scripts/ratchet/host_anchors.tsv`
//! is the dated, sourced anchor table for host-class ns costs.
//!
//! The count theorems are class-INDEPENDENT (`WorkIo.lean` any-class:
//! `wal_commit_plan_at_most_one_fdatasync_any_class`,
//! `barrier_count_class_independent`) — anchors only fill the physical ns
//! cost per class, never change a count. This test pins that contract:
//! every `HostIoClass` constructor is present with a date and a source
//! that exists in the repo; every row's value parses (u64 ns or the
//! documented phase-pin form); every label is an honest quiet/DIAG word.
//! The write-cycle kernel (0192) is NOT edited or consulted.

use std::fs;
use std::path::PathBuf;

#[test]
fn host_anchor_table_is_dated_sourced_and_class_complete() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tsv = fs::read_to_string(repo.join("scripts/ratchet/host_anchors.tsv"))
        .expect("scripts/ratchet/host_anchors.tsv must exist (RFC-0203 P1.2)");

    let mut seen: Vec<[String; 7]> = Vec::new();
    for line in tsv.lines() {
        let line = line.split('#').next().unwrap().trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            7,
            "anchor row needs 7 tab-separated fields: {line:?}"
        );
        let row: [String; 7] = std::array::from_fn(|i| fields[i].trim().to_string());
        let [classe, _ancora, valor_ns, data, _host, rotulo, fonte] = &row;

        // classe: a HostIoClass constructor (or the darwin G1 class, RFC-0036)
        assert!(
            matches!(classe.as_str(), "linux_fdatasync" | "darwin_fullfsync" | "darwin_fdatasync"),
            "unknown host class {classe:?}"
        );

        // valor_ns: u64 ns OR the documented phase-pin form k=v;k=v;...
        let is_u64 = valor_ns.chars().all(|c| c.is_ascii_digit()) && !valor_ns.is_empty();
        let is_phase_pin = valor_ns.matches(';').count() >= 1
            && valor_ns.split(';').all(|pair| {
                let (k, v) = pair.split_once('=').unwrap_or(("", ""));
                !k.is_empty() && !v.is_empty() && v.chars().all(|c| c.is_ascii_digit())
            });
        assert!(
            is_u64 || is_phase_pin,
            "valor_ns must be u64 ns or phase pin `k=v;k=v`: {valor_ns:?}"
        );

        // data: YYYY-MM-DD
        let d = data.as_bytes();
        assert!(
            d.len() == 10
                && d[0..4].iter().all(u8::is_ascii_digit)
                && d[4] == b'-'
                && d[5..7].iter().all(u8::is_ascii_digit)
                && d[7] == b'-'
                && d[8..10].iter().all(u8::is_ascii_digit),
            "data must be YYYY-MM-DD: {data:?}"
        );

        // rótulo: the honest vocabulary, nothing else
        assert!(
            matches!(rotulo.as_str(), "quiet" | "DIAG"),
            "rótulo must be quiet|DIAG: {rotulo:?}"
        );

        // fonte: first token is a repo path that exists
        let src = fonte.split_whitespace().next().unwrap_or_default();
        assert!(
            repo.join(src).is_file(),
            "fonte {src:?} does not exist in the repo — every anchor cites its measurement"
        );
        seen.push(row);
    }
    assert!(!seen.is_empty(), "anchor table has no rows");

    // both HostIoClass constructors are anchored, dated and sourced
    for class in ["linux_fdatasync", "darwin_fullfsync"] {
        assert!(
            seen.iter().any(|r| r[0] == class),
            "HostIoClass constructor {class} has no anchor row (WorkIo.lean enum must be fully anchored)"
        );
    }
}

#[test]
fn count_multipliers_stay_class_independent() {
    // The anchors above may only fill ns cost per class — the COUNT
    // theorems hold verbatim in every class (WorkIo.lean any-class).
    // Structural tie: the named theorems exist in the Lean source that
    // the lean gate compiles (sorry-free by check_depth_floor).
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workio = fs::read_to_string(repo.join("formal/aeneas/lean/WorkIo.lean"))
        .expect("formal/aeneas/lean/WorkIo.lean must exist");
    for theorem in [
        "wal_commit_plan_at_most_one_fdatasync_any_class",
        "barrier_count_class_independent",
    ] {
        assert!(
            workio.contains(&format!("theorem {theorem}")),
            "WorkIo.lean lost any-class theorem `{theorem}` — count multipliers must not become class-dependent"
        );
    }
}
