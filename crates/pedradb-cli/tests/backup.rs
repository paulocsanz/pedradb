//! RFC-0060 P2.8/P2.10/P2.14: `pedra backup` + `verify-backup` / `verify` product path.

use std::process::Command;

use pedradb_core::Db;

fn pedra() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_pedra"));
    c.env_remove("PEDRA_VERIFIED");
    c
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pedra-cli-backup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn pedra_verify_backup_ok() {
    let db = scratch("ok-db");
    let bak = scratch("ok-bak");
    let demo = pedra()
        .args(["demo", db.to_str().unwrap()])
        .output()
        .expect("demo");
    assert!(
        demo.status.success(),
        "demo: {}",
        String::from_utf8_lossy(&demo.stderr)
    );
    let backup = pedra()
        .args(["backup", db.to_str().unwrap(), bak.to_str().unwrap()])
        .output()
        .expect("backup");
    assert!(
        backup.status.success(),
        "backup: {} {}",
        String::from_utf8_lossy(&backup.stdout),
        String::from_utf8_lossy(&backup.stderr)
    );
    let out = pedra()
        .args(["verify-backup", bak.to_str().unwrap(), "1"])
        .output()
        .expect("verify-backup");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "verify-backup clean must pass\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stdout.contains("ok id=1"), "{stdout}");
    let _ = std::fs::remove_dir_all(&db);
    let _ = std::fs::remove_dir_all(&bak);
}

/// RFC-0060 P2.10: poison CHANGELOG in the base copy; `pedra verify-backup` fails.
#[test]
fn pedra_verify_backup_flags_poison_changelog() {
    let db = scratch("ch-db");
    let bak = scratch("ch-bak");
    assert!(pedra()
        .args(["demo", db.to_str().unwrap()])
        .output()
        .unwrap()
        .status
        .success());
    assert!(pedra()
        .args(["backup", db.to_str().unwrap(), bak.to_str().unwrap()])
        .output()
        .unwrap()
        .status
        .success());
    let ch = bak.join("base-000001").join("CHANGELOG");
    let mut bytes = if ch.exists() {
        std::fs::read(&ch).unwrap()
    } else {
        let mut b = b"PDBCHLG1".to_vec();
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b
    };
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&ch, &bytes).unwrap();
    let out = pedra()
        .args(["verify-backup", bak.to_str().unwrap(), "1"])
        .output()
        .expect("verify-backup");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "poison CHANGELOG must fail verify-backup\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("CHANGELOG")
            || stderr.contains("at-rest scrub")
            || stdout.contains("CHANGELOG"),
        "must mention scrub/CHANGELOG: {stdout} {stderr}"
    );
    let _ = std::fs::remove_dir_all(&db);
    let _ = std::fs::remove_dir_all(&bak);
}

/// RFC-0060 P2.14: `pedra verify` on a backup root CRC-walks `wal/*.warch`.
#[test]
fn pedra_verify_backup_root_flags_corrupt_warch() {
    let db = scratch("warch-db");
    let bak = scratch("warch-bak");
    assert!(pedra()
        .args(["demo", db.to_str().unwrap()])
        .output()
        .unwrap()
        .status
        .success());
    assert!(pedra()
        .args(["backup", db.to_str().unwrap(), bak.to_str().unwrap()])
        .output()
        .unwrap()
        .status
        .success());
    let clean = pedra()
        .args(["verify", bak.to_str().unwrap()])
        .output()
        .expect("verify bak");
    assert!(
        clean.status.success(),
        "fresh backup root must verify: {} {}",
        String::from_utf8_lossy(&clean.stdout),
        String::from_utf8_lossy(&clean.stderr)
    );
    {
        let mut live = Db::open(&db).unwrap();
        live.put(b"extra", b"x").unwrap();
        live.close().unwrap();
    }
    let ship = pedra()
        .args(["ship-wal", db.to_str().unwrap(), bak.to_str().unwrap()])
        .output()
        .expect("ship-wal");
    assert!(
        ship.status.success(),
        "ship-wal: {} {}",
        String::from_utf8_lossy(&ship.stdout),
        String::from_utf8_lossy(&ship.stderr)
    );
    let wal_dir = bak.join("wal");
    let warch = std::fs::read_dir(&wal_dir)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "warch"))
        .expect("warch segment");
    let mut bytes = std::fs::read(&warch).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&warch, &bytes).unwrap();
    let fail = pedra()
        .args(["verify", bak.to_str().unwrap()])
        .output()
        .expect("verify poison warch");
    let stdout = String::from_utf8_lossy(&fail.stdout);
    let stderr = String::from_utf8_lossy(&fail.stderr);
    assert!(
        !fail.status.success(),
        "poison warch must fail pedra verify\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("warch") || stderr.contains("wal archive") || stdout.contains("warch"),
        "must mention warch: {stdout} {stderr}"
    );
    let _ = std::fs::remove_dir_all(&db);
    let _ = std::fs::remove_dir_all(&bak);
}

/// RFC-0060 P2.17: `pedra verify` on a backup root names a poison `CATALOG`.
#[test]
fn pedra_verify_backup_root_flags_corrupt_catalog() {
    let db = scratch("cat-db");
    let bak = scratch("cat-bak");
    assert!(pedra()
        .args(["demo", db.to_str().unwrap()])
        .output()
        .unwrap()
        .status
        .success());
    assert!(pedra()
        .args(["backup", db.to_str().unwrap(), bak.to_str().unwrap()])
        .output()
        .unwrap()
        .status
        .success());
    let cat = bak.join("CATALOG");
    let mut bytes = std::fs::read(&cat).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&cat, &bytes).unwrap();
    let fail = pedra()
        .args(["verify", bak.to_str().unwrap()])
        .output()
        .expect("verify poison catalog");
    let stdout = String::from_utf8_lossy(&fail.stdout);
    let stderr = String::from_utf8_lossy(&fail.stderr);
    assert!(
        !fail.status.success(),
        "poison CATALOG must fail pedra verify\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("CATALOG") || stderr.contains("CATALOG"),
        "must name CATALOG: {stdout} {stderr}"
    );
    let _ = std::fs::remove_dir_all(&db);
    let _ = std::fs::remove_dir_all(&bak);
}
