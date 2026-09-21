//! RFC-0203 P1.1 — darwin barrier anchor: `fcntl(F_FULLFSYNC)` vs
//! `fdatasync`, intra-host, dated and re-runnable.
//!
//! This is a MEASUREMENT, never a theorem: ns numbers live in
//! `findings/` + `scripts/ratchet/host_anchors.tsv` (RFC-0187 boundary —
//! physical persistence stays experiment), never in Lean.
//!
//! Re-run:
//! ```text
//! cargo run -q --release -p pedradb-posix --example fullfsync_anchor
//! ```
//!
//! RFC-0204 P2.1 — re-anchor rite (`--gate-quiet[=THRESHOLD]`, default
//! 1.0): the 1-minute loadavg is read BEFORE any measurement; a busy
//! box exits 1 without measuring (the honest outcome — the DIAG row
//! stays, nothing is appended). A quiet run prints, besides the usual
//! report, TABLE-READY rows (the exact `host_anchors.tsv` shape,
//! supersession column empty) and the rite: record the run in a dated
//! finding, append the row, then date-supersede the old row of the
//! same class (old rows are NEVER deleted). On linux the same flag
//! gates the ISOLATED fdatasync(2) ns anchor (RFC-0204 P2.2).
//!
//! Prints, per flavor (`fdatasync` via libSystem `fdatasync_file`;
//! `F_FULLFSYNC` via std `File::sync_all`, which IS `fcntl(F_FULLFSYNC)`
//! on darwin): N timed barriers after a real 64-byte write each, median
//! (p50) and spread (p10..p90), plus the host, the wall-clock date, and
//! `uptime` loadavg at the moment of the run — the honest `quiet`/`DIAG`
//! label for the anchor is decided FROM that loadavg (quiet only if the
//! box is actually quiet), and the ratio F_FULLFSYNC/fdatasync is printed
//! as an intra-host class multiplier (the count theorems stay
//! class-independent; WorkIo.lean).
//!
//! On non-darwin hosts this prints the same fdatasync numbers and notes
//! that `File::sync_all` is NOT `F_FULLFSYNC` there — the darwin anchor
//! is only meaningful on darwin.

use std::fs::File;
use std::io::Write;
use std::process::Command;
use std::time::{Duration, Instant};

use pedradb_posix::fdatasync_file;

fn p(samples: &[Duration], q: f64) -> Duration {
    let idx = ((samples.len() as f64 - 1.0) * q).round() as usize;
    samples[idx.min(samples.len() - 1)]
}

fn flavor(f: &mut File, n: usize, fullfsync: bool) -> Vec<Duration> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        f.write_all(&[(i % 251) as u8; 64]).unwrap();
        let t = Instant::now();
        if fullfsync {
            f.sync_all().unwrap(); // darwin: fcntl(F_FULLFSYNC)
        } else {
            fdatasync_file(f).unwrap(); // darwin: libSystem fdatasync
        }
        out.push(t.elapsed());
    }
    out.sort();
    out
}

/// 1-minute loadavg, first float after the last ':' of the load
/// section; `/proc/loadavg` (linux) then `sysctl -n vm.loadavg`
/// (darwin) then `uptime` (fallback). None = could not read.
fn loadavg_1min() -> Option<f64> {
    if let Ok(proc_load) = std::fs::read_to_string("/proc/loadavg") {
        if let Some(first) = proc_load.split_whitespace().next() {
            if let Ok(v) = first.parse::<f64>() {
                return Some(v);
            }
        }
    }
    let sysctl = Command::new("sysctl").args(["-n", "vm.loadavg"]).output().ok();
    if let Some(o) = sysctl {
        if let Ok(s) = String::from_utf8(o.stdout) {
            // macOS sysctl prints the locale decimal separator (e.g.
            // `{ 108,36 ... }` under pt-BR) — normalize to '.'
            for tok in s.replace(',', ".").split_whitespace() {
                if let Ok(v) = tok.parse::<f64>() {
                    return Some(v);
                }
            }
        }
    }
    let up = Command::new("uptime").output().ok();
    if let Some(o) = up {
        if let Ok(s) = String::from_utf8(o.stdout) {
            if let Some(load_part) = s.split("load average").nth(1) {
                if let Some(after) = load_part.split_once(':').map(|(_, a)| a) {
                    for tok in after.replace(',', ".").split_whitespace() {
                        if let Ok(v) = tok.parse::<f64>() {
                            return Some(v);
                        }
                    }
                }
            }
        }
    }
    None
}

fn main() {
    let mut n: usize = 200;
    let mut gate_quiet = false;
    let mut threshold = 1.0f64;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--gate-quiet" {
            gate_quiet = true;
            if let Some(next) = args.get(i + 1) {
                if let Ok(t) = next.parse::<f64>() {
                    threshold = t;
                    i += 1;
                }
            }
        } else if let Some(v) = a.strip_prefix("--gate-quiet=") {
            gate_quiet = true;
            threshold = v.parse().unwrap_or_else(|_| {
                eprintln!("bad --gate-quiet threshold: {v:?}");
                std::process::exit(2);
            });
        } else if let Ok(v) = a.parse::<usize>() {
            n = v;
        } else {
            eprintln!("unknown arg {a:?}");
            eprintln!("usage: fullfsync_anchor [n] [--gate-quiet[=THRESHOLD]]");
            std::process::exit(2);
        }
        i += 1;
    }

    // RFC-0204 P2.1: the gate runs BEFORE any measurement — a busy box
    // measures nothing (exit 1); a quiet box measures and emits
    // table-ready rows.
    if gate_quiet {
        match loadavg_1min() {
            None => {
                eprintln!("gate-quiet: cannot read loadavg — refusing to measure (honest gate)");
                std::process::exit(1);
            }
            Some(l) if l >= threshold => {
                println!(
                    "gate-quiet: loadavg(1m)={l:.2} >= {threshold} — NOT quiet; refusing to measure"
                );
                println!("gate-quiet: the existing row stays; nothing appended");
                std::process::exit(1);
            }
            Some(l) => {
                println!("gate-quiet: loadavg(1m)={l:.2} < {threshold} — quiet window, measuring");
            }
        }
    }

    let host = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let now = humantime_date();
    let loadavg = Command::new("uptime").output().ok().and_then(|o| {
        String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
    });

    let dir = std::env::temp_dir().join(format!("pedra-fullfsync-anchor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("anchor.bin");
    let mut f = File::create(&path).unwrap();
    f.write_all(&[0u8; 4096]).unwrap();

    // warm both paths once so first-call setup is not a sample
    let _ = fdatasync_file(&f).unwrap();
    let _ = f.sync_all().unwrap();

    let fd = flavor(&mut f, n, false);
    let ff = flavor(&mut f, n, true);
    let _ = std::fs::remove_dir_all(&dir);

    let fd_p50 = p(&fd, 0.50).as_nanos();
    let ff_p50 = p(&ff, 0.50).as_nanos();
    println!("host: {host}");
    println!("date: {now}");
    println!("samples-per-flavor: {n}");
    if let Some(up) = &loadavg {
        println!("uptime: {up}");
    }
    println!(
        "fdatasync      p50={}ns  spread(p10..p90)={}..{}ns",
        fd_p50,
        p(&fd, 0.10).as_nanos(),
        p(&fd, 0.90).as_nanos()
    );
    println!(
        "F_FULLFSYNC    p50={}ns  spread(p10..p90)={}..{}ns",
        ff_p50,
        p(&ff, 0.10).as_nanos(),
        p(&ff, 0.90).as_nanos()
    );
    if fd_p50 > 0 {
        println!("ratio F_FULLFSYNC/fdatasync (p50): {:.1}x", ff_p50 as f64 / fd_p50 as f64);
    }
    if std::env::consts::OS != "macos" {
        println!("note: non-darwin host — File::sync_all is NOT F_FULLFSYNC here; the darwin anchor row needs a darwin run");
    }
    if gate_quiet {
        // RFC-0204 P2.1/P2.2 — table-ready rows (the exact
        // host_anchors.tsv shape; supersession column empty) for the
        // rite: record the run in a dated finding, append the row,
        // date-supersede the old row of the same class.
        if std::env::consts::OS == "macos" {
            let src = format!("findings/{now}-rfc0204-p21-darwin-reanchor.md");
            println!("row-ready:\tdarwin_fdatasync\tDARWIN_QUIET_0204_P21\t{fd_p50}\t{now}\t{host}\tquiet\t{src}\t");
            println!("row-ready:\tdarwin_fullfsync\tDARWIN_QUIET_0204_P21\t{ff_p50}\t{now}\t{host}\tquiet\t{src}\t");
        } else {
            let src = format!("findings/{now}-rfc0204-p22-linux-isolated.md");
            println!("row-ready:\tlinux_fdatasync\tLINUX_ISOLATED_0204_P22\t{fd_p50}\t{now}\t{host}\tquiet\t{src}\t");
        }
        println!("rite: write the finding with this output, append the row-ready line(s),");
        println!("rite: then supersede the old row of the same class: `<new-anchor-id> <today>` in the last column");
        println!("rite: (old rows are NEVER deleted); then `cargo test -p pedradb-core --test host_anchor_table`");
    }
    println!("label hint: quiet only if the loadavg above is actually quiet; else DIAG");
}

/// Local date without a dependency: civil-from-days (Howard Hinnant).
fn humantime_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
