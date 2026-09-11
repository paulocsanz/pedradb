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

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(200);

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
