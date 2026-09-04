//! Profile-only harness (RFC-0037 P1.1): rebuild the state the bench sees at
//! the `deps_scan` point (seed 2 rounds + apply txns, memtable still resident),
//! then loop the exact scan op (`count_cf` over a 25-key MVCC window in the
//! write CF) so a sampler (`sample`, Instruments) can attribute scan time.
//! Not a bench: numbers here are lab-only.
//!
//! Usage: cargo run --release -p rocksdb-parity-bench --example scan_profile [dir]
//! Env: SCAN_SECONDS (8).

use rocksdb_parity_bench::engines::CompatEngine;
use rocksdb_parity_bench::{ukey, Cfg, Engine, YcsbRunner};

fn xorshift(rng: &mut u64) -> u64 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    *rng
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/pedra-scan-profile".into());
    let records = 4096_usize;
    let ops = 2000_usize;
    let batch = 8_usize;
    let payload = 1000_usize;
    let cfg = Cfg {
        records,
        ops,
        payload,
        zipfian: true,
        batch,
    };
    let e = CompatEngine::open(std::path::Path::new(&dir));
    let mut r = YcsbRunner::new(cfg);
    r.seed(&e); // 2 versions/record via batched commits (same as run_deps)
                // Apply shape (same loop as run_deps step 1): per op, batch=8 picks →
                // prewrite batch + commit batch.
    let yval = vec![b'd'; payload];
    let mut vers = vec![0u64; records];
    let mut zrng = 0u64;
    let theta = 0.99_f64;
    let mut s = 0.0;
    let cdf: Vec<f64> = (0..records)
        .map(|i| {
            s += 1.0 / (i as f64 + 1.0).powf(theta);
            s
        })
        .collect();
    let total = *cdf.last().unwrap();
    let mut pick = || {
        let u = (xorshift(&mut zrng) >> 11) as f64 / (1u64 << 53) as f64 * total;
        cdf.partition_point(|&c| c < u).min(records - 1)
    };
    for _ in 0..ops {
        let mut pre = Vec::with_capacity(batch * 2);
        let mut com = Vec::with_capacity(batch * 2);
        for _ in 0..batch {
            let u = pick();
            vers[u] = vers[u].saturating_add(1);
            let k = rocksdb_parity_bench::mvcc(u, vers[u]);
            pre.push(rocksdb_parity_bench::CfWrite::Put {
                cf: "lock",
                k: ukey(u),
                v: b"l".to_vec(),
            });
            pre.push(rocksdb_parity_bench::CfWrite::Put {
                cf: "default",
                k: k.clone(),
                v: yval.clone(),
            });
            com.push(rocksdb_parity_bench::CfWrite::Put {
                cf: "write",
                k,
                v: b"c".to_vec(),
            });
            com.push(rocksdb_parity_bench::CfWrite::Delete {
                cf: "lock",
                k: ukey(u),
            });
        }
        assert!(e.batch(std::mem::take(&mut pre)));
        assert!(e.batch(std::mem::take(&mut com)));
    }
    e.reset_read_probe();
    // Top the active memtable back up to the bench-time state (~30k entries):
    // apply batches until the probe shows a resident memtable again.
    fn mem_entries(e: &CompatEngine) -> usize {
        let p = e.read_probe_json().unwrap_or_default();
        let i = p.find("\"mem_entries\":").map(|i| i + 14).unwrap_or(0);
        p[i..]
            .split(',')
            .next()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }
    let mut topup = 0u64;
    while mem_entries(&e) < 25_000 {
        let mut pre = Vec::with_capacity(batch * 2);
        let mut com = Vec::with_capacity(batch * 2);
        for _ in 0..batch {
            let u = pick();
            vers[u] = vers[u].saturating_add(1);
            let k = rocksdb_parity_bench::mvcc(u, vers[u]);
            pre.push(rocksdb_parity_bench::CfWrite::Put {
                cf: "lock",
                k: ukey(u),
                v: b"l".to_vec(),
            });
            pre.push(rocksdb_parity_bench::CfWrite::Put {
                cf: "default",
                k: k.clone(),
                v: yval.clone(),
            });
            com.push(rocksdb_parity_bench::CfWrite::Put {
                cf: "write",
                k,
                v: b"c".to_vec(),
            });
            com.push(rocksdb_parity_bench::CfWrite::Delete {
                cf: "lock",
                k: ukey(u),
            });
        }
        assert!(e.batch(std::mem::take(&mut pre)));
        assert!(e.batch(std::mem::take(&mut com)));
        topup += 1;
        if topup > 2000 {
            break;
        }
    }
    let seconds: u64 = std::env::var("SCAN_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let mem_at_start = mem_entries(&e);
    let t0 = std::time::Instant::now();
    let mut n = 0u64;
    let mut z = 0u64;
    let mut errs = 0u64;
    while t0.elapsed().as_secs() < seconds {
        let idx = pick();
        match e.scan_count_cf("write", &ukey(idx), &ukey(idx + 25), 25) {
            Ok(c) => {
                z += c as u64;
                n += 1;
            }
            Err(_) => errs += 1,
        }
    }
    let dt = t0.elapsed().as_secs_f64();
    if let Some(p) = e.read_probe_json() {
        eprintln!("{p}");
    }
    eprintln!(
        "scan_profile ops={n} qps={:.0} avg_count={:.2} errs={errs} mem_start={mem_at_start}",
        n as f64 / dt,
        z as f64 / n.max(1) as f64
    );
}
