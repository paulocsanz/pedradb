use snapshot_bench::snapshot::SnapshotStore;
use snapshot_bench::{KvEntry, KvUpdate, PedraDbConfig, PedraDbSnapshot, VersionToken, WatchCursor};
use tempfile::TempDir;

fn get_rss_kb() -> usize {
    let pid = std::process::id();
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .expect("ps");
    let text = String::from_utf8_lossy(&out.stdout);
    text.trim().parse::<usize>().unwrap_or(0)
}

#[test]
fn test_empty_loop_rss() {
    let n = 3_000_000;
    let mut batch = Vec::with_capacity(1024);
    let rss_start_kb = get_rss_kb();
    for i in (0..n).step_by(1024) {
        batch.clear();
        let end = (i + 1024).min(n);
        for j in i..end {
            batch.push(KvUpdate::Put(KvEntry {
                key: format!("route.svc-{:06}.{:08}", j / 1000, j % 1000),
                value: vec![0xaa; 200],
                version: VersionToken::from_u64(j as u64 + 1),
            }));
        }
    }
    let rss_end_kb = get_rss_kb();
    eprintln!("BASELINE EMPTY LOOP: start {:.1}MB, end {:.1}MB", rss_start_kb as f64 / 1024.0, rss_end_kb as f64 / 1024.0);
}

#[test]
fn test_pedradb_ingest_rss_and_scan() {
    for n in [200_000, 500_000, 1_000_000, 2_000_000] {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store");
        let mut store = PedraDbSnapshot::open(&store_path, PedraDbConfig::default()).expect("open").1;
        let mut batch = Vec::with_capacity(1024);
        let rss_start_kb = get_rss_kb();

        for i in (0..n).step_by(1024) {
            batch.clear();
            let end = (i + 1024).min(n);
            for j in i..end {
                batch.push(KvUpdate::Put(KvEntry {
                    key: format!("route.svc-{:06}.{:08}", j / 1000, j % 1000),
                    value: vec![0xaa; 200],
                    version: VersionToken::from_u64(j as u64 + 1),
                }));
            }
            store.apply(&batch, &WatchCursor::from_u64(end as u64)).expect("apply");
        }
        let rss_after_ingest_kb = get_rss_kb();
        let diag_before_settle = store.memory_diag_string();
        let t_settle = std::time::Instant::now();
        store.settle().expect("settle");
        let settle_ms = t_settle.elapsed().as_secs_f64() * 1000.0;
        let rss_after_settle_kb = get_rss_kb();
        let diag_after_settle = store.memory_diag_string();

        let t_scan = std::time::Instant::now();
        let _scanned = store.range("route.svc-000010.").expect("scan");
        let scan_us = t_scan.elapsed().as_secs_f64() * 1e6;

        let mut lat_miss = Vec::with_capacity(10_000);
        let mk_sample = format!("route.svc-9{:06}.{:08}", 0, 0);
        let is_settled = store.is_settled_sst_only();
        let fast_miss = store.fast_outside_sst_miss(&format!("data\0{mk_sample}"));
        eprintln!("SAMPLE MISS CHECK: is_settled={is_settled}, fast_miss={fast_miss}");
        for i in 0..10_000 {
            let mk = format!("route.svc-9{:06}.{:08}", i % 999_999, i % 1000);
            let t = std::time::Instant::now();
            let res = store.get(&mk).expect("get");
            lat_miss.push(t.elapsed());
            assert!(res.is_none());
        }
        lat_miss.sort();
        let miss_p50_ns = lat_miss[lat_miss.len() / 2].as_nanos();

        eprintln!("N={n}: RSS ingest={:.1}MB ({:.1} B/key), settle={:.1}MB ({:.1}ms), scan={:.1}us, miss_p50={}ns\n  DIAG_BEFORE: {diag_before_settle}\n  DIAG_AFTER:  {diag_after_settle}",
            rss_after_ingest_kb as f64 / 1024.0,
            (rss_after_ingest_kb.saturating_sub(rss_start_kb)) as f64 * 1024.0 / n as f64,
            rss_after_settle_kb as f64 / 1024.0,
            settle_ms,
            scan_us,
            miss_p50_ns,
        );
    }
}
