//! RFC-0058 P2.3: `PEDRA_VERIFIED=1` is the one-line product switch —
//! every live-open CLI command runs the verified profile (`StdEnv`, no
//! io_uring ring). The contract is observable: the banner names the
//! profile version, the demo round-trip (write TX, close, reopen)
//! succeeds under the profile, and without the env var the full mode
//! opens silently.

use std::process::Command;

fn pedra() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_pedra"));
    c.env_remove("PEDRA_VERIFIED");
    c
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("pedra-cli-verified-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn verified_flag_pins_the_profile_and_survives_reopen() {
    let dir = scratch("on");
    let out = pedra()
        .env("PEDRA_VERIFIED", "1")
        .args(["demo", dir.to_str().unwrap()])
        .output()
        .expect("spawn pedra");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "demo failed under PEDRA_VERIFIED=1\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("PEDRA_VERIFIED=1"),
        "banner missing: {stderr}"
    );
    assert!(
        stderr.contains("no io_uring ring"),
        "banner does not name the ring gate (RFC-0058 P2.2): {stderr}"
    );
    assert!(
        stderr.contains("verified_admits_ring=0") && stderr.contains("posix()"),
        "banner must name verified_admits_ring next to posix() (RFC-0080 P1.1): {stderr}"
    );
    assert!(stdout.contains("reopen ok"), "reopen failed: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn default_mode_has_no_verified_banner() {
    let dir = scratch("off");
    let out = pedra()
        .args(["demo", dir.to_str().unwrap()])
        .output()
        .expect("spawn pedra");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "demo failed in full mode\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        !stderr.contains("PEDRA_VERIFIED"),
        "verified banner leaked into full mode: {stderr}"
    );
    assert!(stdout.contains("reopen ok"), "reopen failed: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}
