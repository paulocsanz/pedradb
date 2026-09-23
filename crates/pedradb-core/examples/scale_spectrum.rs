//! RFC-0176: measure get ns vs predicted best / happy / worst, with a
//! noisy-neighbor thread. Tune `SCALE_TAU_*` if the quiet run misses the band.
//!
//!   cargo run --release -p pedradb-core --example scale_spectrum -- 20000

use std::fs::OpenOptions as FsOpen;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use pedradb_core::scale_kernel::{
    happy_hot_bps, point_get_probes, predict_get_ns, probes_worst, SCALE_BPS,
    SCALE_BYTES_PER_ENTRY, SCALE_HAPPY_NOISY_BPS, SCALE_L0_BEST, SCALE_L0_WORST, SCALE_TAU_DISK_NS,
    SCALE_TAU_RAM_NS, SCALE_WORST_NOISY_BPS,
};
use pedradb_core::{db::Db, OpenOptions};

fn main() {
    let n: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20_000);
    let gets: u64 = (n / 4).clamp(2_000, 8_000);
    let dir = std::env::temp_dir().join(format!(
        "pedra-scale-spectrum-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut db = Db::open_with(
        &dir,
        OpenOptions {
            sync: false,
            auto_flush_bytes: Some(64 * 1024),
            auto_compact_sst_count: Some(4),
            ..OpenOptions::default()
        },
    )
    .expect("open");

    let val = vec![b'v'; 200];
    let t_h = Instant::now();
    for i in 0..n {
        let k = format!("k{i:08}");
        db.put(k.as_bytes(), &val).expect("put");
    }
    db.flush().ok();
    let hydrate_s = t_h.elapsed().as_secs_f64();
    let lambda = n as f64 / hydrate_s.max(1e-9);

    let levels = 1u64; // tiny n sits in L0/L1
    let p_best = point_get_probes(levels, SCALE_L0_BEST);
    let p_worst = probes_worst(levels, SCALE_L0_WORST);
    let store = n.saturating_mul(SCALE_BYTES_PER_ENTRY);
    let ram = 8u64 << 30;
    let hot = happy_hot_bps(store, ram);
    let best_ns = predict_get_ns(p_best, SCALE_TAU_RAM_NS, SCALE_TAU_DISK_NS, SCALE_BPS, 0);
    let happy_ns = predict_get_ns(
        p_best,
        SCALE_TAU_RAM_NS,
        SCALE_TAU_DISK_NS,
        hot,
        SCALE_HAPPY_NOISY_BPS,
    );
    let worst_ns = predict_get_ns(
        p_worst,
        SCALE_TAU_RAM_NS,
        SCALE_TAU_DISK_NS,
        0,
        SCALE_WORST_NOISY_BPS,
    );

    let quiet = measure_gets(&mut db, n, gets);
    let noisy = measure_gets_noisy(&mut db, n, gets, &dir);
    db.close().ok();
    let _ = std::fs::remove_dir_all(&dir);

    let eta = if quiet > 0 {
        (noisy as f64 / quiet as f64) - 1.0
    } else {
        0.0
    };

    println!("n={n} gets={gets} hydrate={hydrate_s:.3}s lambda={lambda:.0}/s");
    println!("P_best={p_best} P_worst={p_worst} hot_bps={hot}");
    println!("predict_ns best={best_ns} happy={happy_ns} worst={worst_ns}");
    println!("measured_ns quiet={quiet} noisy={noisy} eta={eta:.3}");

    // Quiet tiny-n run is the RAM happy path. Band: [best/4, worst*4].
    let lo = best_ns / 4;
    let hi = worst_ns.saturating_mul(4).max(best_ns.saturating_mul(8));
    let in_band = quiet >= lo && quiet <= hi;
    println!(
        "band=[{lo},{hi}] quiet_in_band={in_band} noisy_ge_quiet={}",
        noisy >= quiet
    );
    if !in_band {
        eprintln!("SCALE_SPECTRUM_MISS quiet={quiet} not in [{lo},{hi}] — retune TAU");
        std::process::exit(2);
    }
    if noisy < quiet {
        eprintln!("SCALE_SPECTRUM_NOISY_INVERTED — neighbor did not slow gets");
        std::process::exit(3);
    }
    println!("SCALE_SPECTRUM_OK");
}

fn measure_gets(db: &Db, n: u64, gets: u64) -> u64 {
    let t0 = Instant::now();
    for i in 0..gets {
        let k = format!("k{:08}", (i * 7) % n);
        let _ = db.get(k.as_bytes());
    }
    let ns = t0.elapsed().as_nanos() as u64;
    ns / gets.max(1)
}

fn measure_gets_noisy(db: &Db, n: u64, gets: u64, dir: &std::path::Path) -> u64 {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let hammer = dir.join("noisy.bin");
    let h = std::thread::spawn(move || {
        let mut f = FsOpen::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&hammer)
            .expect("noisy file");
        let buf = vec![0xab_u8; 64 * 1024];
        while !flag.load(Ordering::Relaxed) {
            let _ = f.write_all(&buf);
            let _ = f.sync_data();
        }
    });
    let ns = measure_gets(db, n, gets);
    stop.store(true, Ordering::Relaxed);
    let _ = h.join();
    ns
}
