//! RFC-0060 P2.11: `pedra archive verify` product path.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use pedradb_core::{Db, HistoryHorizon, HistoryOptions, OpenOptions, StdEnv};

fn pedra() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_pedra"));
    c.env_remove("PEDRA_VERIFIED");
    c
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pedra-cli-archive-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn pedra_archive_verify_empty_is_clean() {
    let remote = scratch("empty");
    std::fs::create_dir_all(&remote).unwrap();
    let out = pedra()
        .args(["archive", "verify", remote.to_str().unwrap()])
        .output()
        .expect("archive verify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "empty remote must be clean\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stdout.contains("errors=0"), "{stdout}");
    let _ = std::fs::remove_dir_all(&remote);
}

fn seed_remote(local: &Path, remote: &Path) {
    let mut db = Db::open_with(
        local,
        OpenOptions {
            history: HistoryOptions {
                horizon: HistoryHorizon::Window(Duration::from_secs(0)),
                cap_bytes: 1 << 20,
            },
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    // seq_times samples every 32 publishes — need a cutoff > 1 so
    // compact_horizon archives superseded versions.
    for i in 0..40u32 {
        db.put(b"k", format!("v{i}").as_bytes()).unwrap();
    }
    db.compact_horizon().unwrap();
    db.set_remote_history(StdEnv, remote);
    db.upload_history_now().unwrap();
    db.close().unwrap();
}

#[test]
fn pedra_archive_verify_ok_then_flags_poison() {
    let local = scratch("ok-local");
    let remote = scratch("ok-remote");
    seed_remote(&local, &remote);
    let out = pedra()
        .args(["archive", "verify", remote.to_str().unwrap()])
        .output()
        .expect("archive verify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "seeded remote must be clean\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stdout.contains("errors=0"), "{stdout}");
    assert!(
        stdout.contains("segments=") && !stdout.contains("segments=0"),
        "must have uploaded a segment: {stdout}"
    );
    let hist = std::fs::read_dir(&remote)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "hist"))
        .expect("remote hist object");
    let mut bytes = std::fs::read(&hist).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xff;
    std::fs::write(&hist, bytes).unwrap();
    let fail = pedra()
        .args(["archive", "verify", remote.to_str().unwrap()])
        .output()
        .expect("archive verify poison");
    let stdout = String::from_utf8_lossy(&fail.stdout);
    let stderr = String::from_utf8_lossy(&fail.stderr);
    assert!(
        !fail.status.success(),
        "poison remote hist must fail\nstdout: {stdout}\nstderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&local);
    let _ = std::fs::remove_dir_all(&remote);
}

/// RFC-0060 P2.12: `pedra archive verify` names a LATEST CRC mismatch.
#[test]
fn pedra_archive_verify_flags_corrupt_latest() {
    let local = scratch("lat-local");
    let remote = scratch("lat-remote");
    seed_remote(&local, &remote);
    let latest = remote.join("LATEST");
    let body = std::fs::read_to_string(&latest).unwrap();
    let name = body.split('\n').next().unwrap();
    std::fs::write(&latest, format!("{name}\nffffffff")).unwrap();
    let out = pedra()
        .args(["archive", "verify", remote.to_str().unwrap()])
        .output()
        .expect("archive verify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "LATEST crc mismatch must fail\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("LATEST") || stderr.contains("LATEST"),
        "must name LATEST: {stdout} {stderr}"
    );
    let _ = std::fs::remove_dir_all(&local);
    let _ = std::fs::remove_dir_all(&remote);
}
