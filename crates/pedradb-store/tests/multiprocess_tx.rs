//! Multi-process elect + cross-range multi-key TX durability.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let i = N.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("montanha-mp-tx-{n}-{i}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn multi_process_commit_tx_write_then_verify() {
    let bin = env!("CARGO_BIN_EXE_montanha-store-smoke");
    let dir = temp_dir();
    let st = Command::new(bin)
        .args(["write", dir.to_str().unwrap()])
        .status()
        .expect("spawn write");
    assert!(st.success(), "write process failed: {st}");
    let st = Command::new(bin)
        .args(["verify", dir.to_str().unwrap()])
        .status()
        .expect("spawn verify");
    assert!(st.success(), "verify process failed: {st}");
    let _ = std::fs::remove_dir_all(&dir);
}
