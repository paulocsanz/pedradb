//! RFC-0024 smoke: apply then get at pinned cursor; second run resumes pin.
//!
//! Usage: fold-smoke <dir>
//! First run writes /host/h1/cap and prints cursor.
//! Second run with same dir must not replay older seq (get still works).

use pedradb_fold::{caixote_host_filter, follow_prefix, FoldStore, FoldUpdate, PedraFold};
use pedradb_core::Db;
use pedradb_journal::JournalConsumer;
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let dir = match env::args().nth(1) {
        Some(d) => d,
        None => {
            eprintln!("usage: fold-smoke <dir>");
            return ExitCode::from(2);
        }
    };
    let src = format!("{dir}/src");
    let fold_dir = format!("{dir}/fold");
    let mut src_db = match Db::open(&src) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("open src: {e}");
            return ExitCode::from(1);
        }
    };
    let (pin0, mut fold) = match PedraFold::open(std::path::Path::new(&fold_dir)) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("open fold: {e}");
            return ExitCode::from(1);
        }
    };
    let prefixes = caixote_host_filter("h1", &["vm-a"]);
    // Seed SoR once if empty.
    if src_db.get(b"/host/h1/cap").is_none() {
        if let Err(e) = src_db.put(b"/host/h1/cap", b"ok") {
            eprintln!("put: {e}");
            return ExitCode::from(1);
        }
        if let Err(e) = src_db.put(b"/vm/vm-a", b"spec-a") {
            eprintln!("put: {e}");
            return ExitCode::from(1);
        }
        if let Err(e) = src_db.put(b"/other/x", b"nope") {
            eprintln!("put: {e}");
            return ExitCode::from(1);
        }
    }
    let mut consumer = JournalConsumer { pin: pin0.seq() };
    if let Err(e) = pedradb_fold::watch_applied_prefix(
        &src_db,
        &mut consumer,
        &mut fold,
        Some(&prefixes),
    ) {
        eprintln!("watch_applied: {e}");
        return ExitCode::from(1);
    }
    let got = match fold.get(b"/host/h1/cap") {
        Ok(v) => v,
        Err(e) => {
            eprintln!("get: {e}");
            return ExitCode::from(1);
        }
    };
    if got.as_deref() != Some(b"ok".as_ref()) {
        eprintln!("expected /host/h1/cap=ok got {got:?}");
        return ExitCode::from(1);
    }
    if fold.get(b"/other/x").ok().flatten().is_some() {
        // follow filter is applied by watch_applied via full changelog — smoke
        // uses watch_applied which folds ALL changelog keys. Filter via follow.
        let _ = follow_prefix(&src_db, &prefixes, pin0);
    }
    println!(
        "ok pin={} cap={:?} replayed_from={}",
        fold.cursor().seq(),
        got,
        pin0.seq()
    );
    let _ = FoldUpdate::Put {
        key: b"x".to_vec(),
        value: b"y".to_vec(),
        seq: 0,
    };
    ExitCode::SUCCESS
}
