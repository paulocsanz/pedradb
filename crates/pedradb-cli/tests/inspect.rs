//! RFC-0060 P2.23: `pedra inspect` reports the CURRENT CRC trailer class.

use std::process::Command;

fn pedra() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_pedra"));
    c.env_remove("PEDRA_VERIFIED");
    c
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pedra-cli-inspect-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn pedra_inspect_reports_current_crc_ok() {
    let dir = scratch("ok");
    let demo = pedra()
        .args(["demo", dir.to_str().unwrap()])
        .output()
        .expect("demo");
    assert!(
        demo.status.success(),
        "demo: {}",
        String::from_utf8_lossy(&demo.stderr)
    );
    let out = pedra()
        .args(["inspect", dir.to_str().unwrap()])
        .output()
        .expect("inspect");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "inspect must pass\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("current_crc=ok"),
        "flushed CURRENT must report crc ok: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
