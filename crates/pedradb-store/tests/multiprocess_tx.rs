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

/// RFC-0017: multi-process multiwrite + fast RO path in a separate OS process.
#[test]
fn multi_process_multiwrite_and_fast_ro() {
    let bin = env!("CARGO_BIN_EXE_montanha-store-smoke");
    let dir = temp_dir();
    let st = Command::new(bin)
        .args(["multiwrite", dir.to_str().unwrap()])
        .status()
        .expect("spawn multiwrite");
    assert!(st.success(), "multiwrite process failed: {st}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Universe A: elect + put + minority partition + heal (durable dirs, one process CLI).
#[test]
fn multi_process_partition_elect_put() {
    let bin = env!("CARGO_BIN_EXE_montanha-store-smoke");
    let dir = temp_dir();
    let st = Command::new(bin)
        .args(["partition", dir.to_str().unwrap()])
        .status()
        .expect("spawn partition");
    assert!(st.success(), "partition process failed: {st}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Universe B+C: canaries write process then verify process (lease reopen + index + journal).
#[test]
fn multi_process_canaries_write_then_verify() {
    let bin = env!("CARGO_BIN_EXE_montanha-store-smoke");
    let dir = temp_dir();
    let st = Command::new(bin)
        .args(["canaries", dir.to_str().unwrap()])
        .status()
        .expect("spawn canaries");
    assert!(st.success(), "canaries process failed: {st}");
    let st = Command::new(bin)
        .args(["canaries-verify", dir.to_str().unwrap()])
        .status()
        .expect("spawn canaries-verify");
    assert!(st.success(), "canaries-verify process failed: {st}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0017 P2.3: DCS layer + app batch only on multi-process StoreCluster (no dual DCS path).
#[test]
fn multi_process_dcs_layer_freeze() {
    let bin = env!("CARGO_BIN_EXE_montanha-store-smoke");
    let dir = temp_dir();
    let st = Command::new(bin)
        .args(["dcs-layer", dir.to_str().unwrap()])
        .status()
        .expect("spawn dcs-layer");
    assert!(st.success(), "dcs-layer process failed: {st}");
    let st = Command::new(bin)
        .args(["dcs-layer-verify", dir.to_str().unwrap()])
        .status()
        .expect("spawn dcs-layer-verify");
    assert!(st.success(), "dcs-layer-verify process failed: {st}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0022 P0.4: etcd-need face create/CAS/get/watch only on multi-process store.
#[test]
fn multi_process_etcd_need_face_freeze() {
    let bin = env!("CARGO_BIN_EXE_montanha-store-smoke");
    let dir = temp_dir();
    let st = Command::new(bin)
        .args(["etcd-need", dir.to_str().unwrap()])
        .status()
        .expect("spawn etcd-need");
    assert!(st.success(), "etcd-need process failed: {st}");
    let st = Command::new(bin)
        .args(["etcd-need-verify", dir.to_str().unwrap()])
        .status()
        .expect("spawn etcd-need-verify");
    assert!(st.success(), "etcd-need-verify process failed: {st}");
    let _ = std::fs::remove_dir_all(&dir);
}
