//! RFC-0203 P1.2 — consumption test: `scripts/ratchet/host_anchors.tsv`
//! is the dated, sourced anchor table for host-class ns costs.
//!
//! RFC-0204 P2.1 — supersession semantics: old rows are NEVER deleted;
//! a superseded row carries `<anchor-id> <date>` in its last column,
//! pointing at a NEWER MEASURED row of the same class. Exactly one
//! LIVE measured row (supersession empty, label quiet|DIAG) exists per
//! host class — that row is the current anchor. `deferred` (RFC-0204
//! P2.2, 0203 vocabulary) is dated measurement DEBT: value `-`, never
//! a live anchor, superseded by the real measurement when a quiet
//! window exists.
//!
//! The count theorems are class-INDEPENDENT (`WorkIo.lean` any-class:
//! `wal_commit_plan_at_most_one_fdatasync_any_class`,
//! `barrier_count_class_independent`) — anchors only fill the physical ns
//! cost per class, never change a count. This test pins that contract:
//! every `HostIoClass` constructor has a live measured row with a date
//! and a repo-existing source; every row's value parses (u64 ns, the
//! documented phase-pin form, or `-` for a deferral); supersession
//! chains terminate at a live row with no cycles. The write-cycle
//! kernel (0192) is NOT edited or consulted.

use std::fs;
use std::path::PathBuf;

/// One parsed anchor row (fields already trimmed).
#[derive(Clone, Debug, PartialEq)]
struct AnchorRow {
    classe: String,
    ancora: String,
    valor_ns: String,
    data: String,
    host: String,
    rotulo: String,
    fonte: String,
    supersessao: String,
}

const REQUIRED_CLASSES: [&str; 3] = ["linux_fdatasync", "darwin_fullfsync", "darwin_fdatasync"];

fn is_date(s: &str) -> bool {
    let d = s.as_bytes();
    d.len() == 10
        && d[0..4].iter().all(u8::is_ascii_digit)
        && d[4] == b'-'
        && d[5..7].iter().all(u8::is_ascii_digit)
        && d[7] == b'-'
        && d[8..10].iter().all(u8::is_ascii_digit)
}

fn is_u64(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

fn is_phase_pin(s: &str) -> bool {
    s.matches(';').count() >= 1
        && s.split(';').all(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or(("", ""));
            !k.is_empty() && !v.is_empty() && v.chars().all(|c| c.is_ascii_digit())
        })
}

/// Pure parser/validator over the table text: field shape, honest
/// vocabulary, deferral shape, and supersession coherence (successor
/// exists, same class, measured and newer; chains terminate; no
/// cycles; exactly one live measured row per host class).
fn parse_anchor_table(tsv: &str) -> Result<Vec<AnchorRow>, String> {
    let mut rows: Vec<AnchorRow> = Vec::new();
    for (idx, raw) in tsv.lines().enumerate() {
        let line = raw.split('#').next().unwrap().trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let where_ = format!("row {}: ", idx + 1);
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 8 {
            return Err(format!(
                "{where_}8 tab-separated fields expected (class anchor ns_value date host label source superseded_by), got {}",
                fields.len()
            ));
        }
        let row = AnchorRow {
            classe: fields[0].trim().to_string(),
            ancora: fields[1].trim().to_string(),
            valor_ns: fields[2].trim().to_string(),
            data: fields[3].trim().to_string(),
            host: fields[4].trim().to_string(),
            rotulo: fields[5].trim().to_string(),
            fonte: fields[6].trim().to_string(),
            supersessao: fields[7].trim().to_string(),
        };
        let r = &row;
        if !REQUIRED_CLASSES.contains(&r.classe.as_str()) {
            return Err(format!("{where_}unknown host class {:?}", r.classe));
        }
        match r.rotulo.as_str() {
            "quiet" | "DIAG" => {
                if !(is_u64(&r.valor_ns) || is_phase_pin(&r.valor_ns)) {
                    return Err(format!(
                        "{where_}measured row valor_ns must be u64 ns or phase pin `k=v;k=v`: {:?}",
                        r.valor_ns
                    ));
                }
            }
            "deferred" => {
                if r.valor_ns != "-" {
                    return Err(format!(
                        "{where_}deferred row must carry ns_value `-` (dated debt, no measurement): {:?}",
                        r.valor_ns
                    ));
                }
            }
            other => {
                return Err(format!("{where_}label must be quiet|DIAG|deferred: {other:?}"));
            }
        }
        if !is_date(&r.data) {
            return Err(format!("{where_}data must be YYYY-MM-DD: {:?}", r.data));
        }
        if r.fonte.split_whitespace().next().unwrap_or_default().is_empty() {
            return Err(format!("{where_}source missing — every row cites a finding"));
        }
        if !r.supersessao.is_empty() {
            let parts: Vec<&str> = r.supersessao.split_whitespace().collect();
            if parts.len() != 2 || !is_date(parts[1]) {
                return Err(format!(
                    "{where_}superseded_by must be `<anchor-id> <YYYY-MM-DD>`: {:?}",
                    r.supersessao
                ));
            }
        }
        if rows
            .iter()
            .any(|prev| prev.classe == r.classe && prev.ancora == r.ancora)
        {
            return Err(format!(
                "{where_}duplicate anchor id {:?} within class {:?}",
                r.ancora, r.classe
            ));
        }
        rows.push(row);
    }
    if rows.is_empty() {
        return Err("anchor table has no rows".to_string());
    }

    // supersession coherence: successor exists, same class, MEASURED
    // (never a deferral) and not older than the row it supersedes.
    for r in &rows {
        if r.supersessao.is_empty() {
            continue;
        }
        let (succ_id, succ_date) = r
            .supersessao
            .split_once(' ')
            .expect("checked two tokens above");
        let succ = rows
            .iter()
            .find(|s| s.classe == r.classe && s.ancora == succ_id)
            .ok_or_else(|| {
                format!(
                    "row {:?} supersedes unknown anchor {succ_id:?} of class {:?}",
                    r.ancora, r.classe
                )
            })?;
        if succ.classe != r.classe {
            return Err(format!(
                "row {:?} (class {}) is superseded by {succ_id:?} of a DIFFERENT class {}",
                r.ancora, r.classe, succ.classe
            ));
        }
        if succ.rotulo == "deferido" {
            return Err(format!(
                "row {:?} is superseded by {succ_id:?}, which is a deferral — a deferral never replaces a measurement",
                r.ancora
            ));
        }
        if succ.data < r.data {
            return Err(format!(
                "row {:?} ({}) is superseded by the OLDER {succ_id:?} ({succ_date})",
                r.ancora, r.data
            ));
        }
    }

    // chains terminate at a live row; no cycles
    for start in &rows {
        let mut hop = start;
        let mut steps = 0usize;
        while !hop.supersessao.is_empty() {
            steps += 1;
            if steps > rows.len() {
                return Err(format!(
                    "supersession chain from {:?} does not terminate (cycle?)",
                    start.ancora
                ));
            }
            let succ_id = hop.supersessao.split_whitespace().next().unwrap();
            hop = rows
                .iter()
                .find(|s| s.classe == hop.classe && s.ancora == succ_id)
                .expect("successor existence checked above");
        }
    }

    // exactly one LIVE measured row per host class (the current anchor)
    for class in REQUIRED_CLASSES {
        let live = rows
            .iter()
            .filter(|r| r.classe == class && r.supersessao.is_empty() && r.rotulo != "deferido")
            .count();
        if live != 1 {
            return Err(format!(
                "class {class} has {live} live measured rows — exactly one is the current anchor \
                 (supersede the old row, never delete it)"
            ));
        }
    }
    Ok(rows)
}

#[test]
fn host_anchor_table_is_dated_sourced_and_class_complete() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tsv = fs::read_to_string(repo.join("scripts/ratchet/host_anchors.tsv"))
        .expect("scripts/ratchet/host_anchors.tsv must exist (RFC-0203 P1.2)");

    let rows = parse_anchor_table(&tsv)
        .expect("host_anchors.tsv parses under the RFC-0204 P2.1 supersession semantics");

    // fonte: first token is a repo path that exists
    for r in &rows {
        let src = r.fonte.split_whitespace().next().unwrap_or_default();
        assert!(
            repo.join(src).is_file(),
            "fonte {src:?} does not exist in the repo — every anchor cites its measurement"
        );
    }
}

#[test]
fn supersession_semantics_are_enforced() {
    let good = "darwin_fullfsync\tOLD_DIAG\t1000\t2026-09-01\tmacos-aarch64\tDIAG\tf.md\tNEW_QUIET 2026-09-11\n\
                darwin_fullfsync\tNEW_QUIET\t900\t2026-09-11\tmacos-aarch64\tquiet\tf.md\t\n\
                darwin_fdatasync\tFD_DIAG\t500\t2026-09-11\tmacos-aarch64\tDIAG\tf.md\t\n\
                linux_fdatasync\tLX_PIN\tenc=1;wr=2\t2026-09-10\tlinux-p149b\tquiet\tf.md\t\n";
    assert!(parse_anchor_table(good).is_ok(), "good table with one dated supersession must parse");

    // unknown successor id
    let bad = good.replacen("NEW_QUIET 2026-09-11", "GHOST 2026-09-11", 1);
    assert!(parse_anchor_table(&bad).is_err(), "unknown supersessor must be RED");

    // superseded by a deferral
    let defer = "linux_fdatasync\tLX_PIN\tenc=1;wr=2\t2026-09-10\tlinux-p149b\tquiet\tf.md\tLX_DEF 2026-09-11\n\
                 linux_fdatasync\tLX_DEF\t-\t2026-09-11\tlinux\tdeferido\tf.md\t\n";
    assert!(parse_anchor_table(defer).is_err(), "a deferral must never supersede a measurement");

    // cycle: A -> B -> A
    let cycle = "darwin_fdatasync\tA\t1\t2026-09-01\tm\tDIAG\tf.md\tB 2026-09-02\n\
                 darwin_fdatasync\tB\t2\t2026-09-02\tm\tDIAG\tf.md\tA 2026-09-03\n";
    assert!(parse_anchor_table(cycle).is_err(), "supersession cycle must be RED");

    // two live measured rows of one class (old row not superseded)
    let two_live = "darwin_fdatasync\tFD_OLD\t1\t2026-09-01\tm\tDIAG\tf.md\t\n\
                    darwin_fdatasync\tFD_NEW\t2\t2026-09-11\tm\tquiet\tf.md\t\n";
    assert!(parse_anchor_table(two_live).is_err(), "two live anchors of one class must be RED");

    // deferral with a value
    let def_val = "darwin_fdatasync\tFD_DIAG\t500\t2026-09-11\tm\tDIAG\tf.md\t\n\
                   darwin_fullfsync\tFF_DEF\t123\t2026-09-11\tm\tdeferido\tf.md\t\n\
                   linux_fdatasync\tLX\t1\t2026-09-11\tm\tquiet\tf.md\t\n";
    assert!(parse_anchor_table(def_val).is_err(), "deferido with a measured value must be RED");

    // deferral as tracked debt is fine (live measured row still unique)
    let def_ok = "darwin_fdatasync\tFD_DIAG\t500\t2026-09-11\tm\tDIAG\tf.md\t\n\
                  darwin_fullfsync\tFF_OK\t1000\t2026-09-11\tm\tDIAG\tf.md\t\n\
                  linux_fdatasync\tLX_PIN\tenc=1;wr=2\t2026-09-10\tlinux\tquiet\tf.md\t\n\
                  linux_fdatasync\tLX_DEF\t-\t2026-09-11\tlinux\tdeferido\tf.md\t\n";
    assert!(parse_anchor_table(def_ok).is_ok(), "a dated deferral row must be GREEN next to the live anchor");

    // missing live row for a required class
    let no_live = "darwin_fdatasync\tFD\t500\t2026-09-11\tm\tDIAG\tf.md\t\n";
    assert!(parse_anchor_table(no_live).is_err(), "a class without a live measured row must be RED");
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
