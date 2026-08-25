//! RFC-0060: `pedra verify` and `pedra maintain --verify` drive the shipped
//! at-rest scrub (same `verify_at_rest` the library tests call).

use std::process::Command;

fn pedra() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_pedra"));
    c.env_remove("PEDRA_VERIFIED");
    c
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pedra-cli-verify-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn pedra_verify_twice_reports_errors_zero() {
    let dir = scratch("twice");
    let demo = pedra()
        .args(["demo", dir.to_str().unwrap()])
        .output()
        .expect("demo");
    assert!(
        demo.status.success(),
        "demo failed: {}",
        String::from_utf8_lossy(&demo.stderr)
    );
    let mut lines = Vec::new();
    for i in 1..=2 {
        let out = pedra()
            .args(["verify", dir.to_str().unwrap()])
            .output()
            .expect("verify");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "verify #{i} failed\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(stdout.contains("errors=0"), "run {i}: {stdout}");
        assert!(stdout.contains("files="), "run {i}: {stdout}");
        assert!(stdout.contains("bytes="), "run {i}: {stdout}");
        lines.push(stdout.lines().next().unwrap_or("").to_string());
    }
    assert_eq!(lines[0], lines[1], "two launches must agree");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pedra_maintain_verify_runs_the_scrub() {
    let dir = scratch("maintain");
    let demo = pedra()
        .args(["demo", dir.to_str().unwrap()])
        .output()
        .expect("demo");
    assert!(demo.status.success());
    let out = pedra()
        .args(["maintain", dir.to_str().unwrap(), "--verify"])
        .output()
        .expect("maintain");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "maintain --verify failed\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("maintain pass="),
        "maintain pass missing: {stdout}"
    );
    assert!(
        stdout.contains("errors=0"),
        "scrub did not run (no errors=0): {stdout}"
    );
    assert!(stdout.contains("files="), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0060 P2.3: the product binary names a garbage `CURRENT`.
#[test]
fn pedra_verify_flags_garbage_current() {
    let dir = scratch("cur");
    let demo = pedra()
        .args(["demo", dir.to_str().unwrap()])
        .output()
        .expect("demo");
    assert!(demo.status.success());
    std::fs::write(dir.join("CURRENT"), b"not-a-manifest\n").unwrap();
    let out = pedra()
        .args(["verify", dir.to_str().unwrap()])
        .output()
        .expect("verify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "garbage CURRENT must fail pedra verify\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("CURRENT") || stderr.contains("CURRENT"),
        "must name CURRENT: {stdout} {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0060 P2.15: `pedra verify` names a CURRENT CRC mismatch.
#[test]
fn pedra_verify_flags_current_crc_mismatch() {
    let dir = scratch("cur-crc");
    let demo = pedra()
        .args(["demo", dir.to_str().unwrap()])
        .output()
        .expect("demo");
    assert!(demo.status.success());
    let cur = dir.join("CURRENT");
    let body = std::fs::read_to_string(&cur).unwrap();
    let name = body.lines().next().unwrap().trim();
    std::fs::write(&cur, format!("{name}\nffffffff\n")).unwrap();
    let out = pedra()
        .args(["verify", dir.to_str().unwrap()])
        .output()
        .expect("verify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "CURRENT crc mismatch must fail\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("CURRENT") || stderr.contains("CURRENT"),
        "must name CURRENT: {stdout} {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0060 P2.21: leftover `CURRENT.tmp` is a named FAIL in the product binary.
#[test]
fn pedra_verify_flags_leftover_install_temp() {
    let dir = scratch("tmp");
    let demo = pedra()
        .args(["demo", dir.to_str().unwrap()])
        .output()
        .expect("demo");
    assert!(demo.status.success());
    std::fs::write(dir.join("CURRENT.tmp"), b"torn pointer").unwrap();
    let out = pedra()
        .args(["verify", dir.to_str().unwrap()])
        .output()
        .expect("verify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "leftover CURRENT.tmp must fail pedra verify\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("CURRENT.tmp") || stderr.contains("CURRENT.tmp"),
        "must name CURRENT.tmp: {stdout} {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0060 P2.22: `pedra verify` names a garbage `CORRUPTLOG`.
#[test]
fn pedra_verify_flags_garbage_corruptlog() {
    let dir = scratch("clog");
    let demo = pedra()
        .args(["demo", dir.to_str().unwrap()])
        .output()
        .expect("demo");
    assert!(demo.status.success());
    std::fs::write(dir.join("CORRUPTLOG"), b"not-a-journal\n").unwrap();
    let out = pedra()
        .args(["verify", dir.to_str().unwrap()])
        .output()
        .expect("verify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "garbage CORRUPTLOG must fail\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("CORRUPTLOG") || stderr.contains("CORRUPTLOG"),
        "must name CORRUPTLOG: {stdout} {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0060 P2.24: `pedra verify` names a poison `CHANGELOG.corrupt` quarantine.
#[test]
fn pedra_verify_flags_changelog_quarantine() {
    let dir = scratch("chq");
    let demo = pedra()
        .args(["demo", dir.to_str().unwrap()])
        .output()
        .expect("demo");
    assert!(demo.status.success());
    let mut b = b"PDBCHLG1".to_vec();
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    let last = b.len() - 1;
    b[last] ^= 0xff;
    std::fs::write(dir.join("CHANGELOG.corrupt"), &b).unwrap();
    let out = pedra()
        .args(["verify", dir.to_str().unwrap()])
        .output()
        .expect("verify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "poison CHANGELOG.corrupt must fail\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("CHANGELOG.corrupt") || stderr.contains("CHANGELOG.corrupt"),
        "must name CHANGELOG.corrupt: {stdout} {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0060 P2.25: leftover `VALUES.vlog.adopt` is a named FAIL.
#[test]
fn pedra_verify_flags_leftover_vlog_adopt() {
    let dir = scratch("adopt");
    let demo = pedra()
        .args(["demo", dir.to_str().unwrap()])
        .output()
        .expect("demo");
    assert!(demo.status.success());
    std::fs::write(dir.join("VALUES.vlog.adopt"), b"legacy").unwrap();
    let out = pedra()
        .args(["verify", dir.to_str().unwrap()])
        .output()
        .expect("verify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "leftover adopt must fail\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("VALUES.vlog.adopt") || stderr.contains("VALUES.vlog.adopt"),
        "must name VALUES.vlog.adopt: {stdout} {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0060 P2.26: `pedra verify` names a poison `CFREG`.
#[test]
fn pedra_verify_flags_corrupted_cfreg() {
    let dir = scratch("cfreg");
    let demo = pedra()
        .args(["demo", dir.to_str().unwrap()])
        .output()
        .expect("demo");
    assert!(demo.status.success());
    std::fs::write(dir.join("CFREG"), b"not-a-registry\n").unwrap();
    let out = pedra()
        .args(["verify", dir.to_str().unwrap()])
        .output()
        .expect("verify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "poison CFREG must fail\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("CFREG") || stderr.contains("CFREG"),
        "must name CFREG: {stdout} {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0060 P2.27: `pedra verify` names an unrecognized leftover file.
#[test]
fn pedra_verify_flags_unrecognized_file() {
    let dir = scratch("junk");
    let demo = pedra()
        .args(["demo", dir.to_str().unwrap()])
        .output()
        .expect("demo");
    assert!(demo.status.success());
    std::fs::write(dir.join("junk.dat"), b"??").unwrap();
    let out = pedra()
        .args(["verify", dir.to_str().unwrap()])
        .output()
        .expect("verify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "unrecognized file must fail\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("junk.dat") || stderr.contains("junk.dat"),
        "must name junk.dat: {stdout} {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0060 P2.6: `pedra verify` names a rotten CHANGELOG cache.
#[test]
fn pedra_verify_flags_corrupted_changelog() {
    let dir = scratch("chlog");
    let demo = pedra()
        .args(["demo", dir.to_str().unwrap()])
        .output()
        .expect("demo");
    assert!(demo.status.success());
    let path = dir.join("CHANGELOG");
    let mut bytes = if path.exists() {
        std::fs::read(&path).unwrap()
    } else {
        let mut b = b"PDBCHLG1".to_vec();
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b
    };
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&path, &bytes).unwrap();
    let out = pedra()
        .args(["verify", dir.to_str().unwrap()])
        .output()
        .expect("verify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !out.status.success(),
        "rotten CHANGELOG must fail pedra verify: {stdout}"
    );
    assert!(
        stdout.contains("CHANGELOG"),
        "must name CHANGELOG: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
