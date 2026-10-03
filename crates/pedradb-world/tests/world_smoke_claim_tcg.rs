//! RFC-0079 P1.2: native `world_smoke --claim-tcg` is refused.

use std::process::Command;

#[test]
fn world_smoke_refuses_claim_tcg_on_native() {
    let bin = env!("CARGO_BIN_EXE_world_smoke");
    let out = Command::new(bin)
        .arg("1")
        .arg("--claim-tcg")
        .output()
        .expect("spawn world_smoke");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(2),
        "native --claim-tcg must exit 2 stdout={stdout} stderr={stderr}"
    );
    assert!(
        stderr.contains("tcg_guest_admitted=false") || stderr.contains("--claim-tcg refused"),
        "stderr must name the refuse: {stderr}"
    );
    assert!(
        stdout.contains("tcg_guest=0"),
        "smoke line must name tcg_guest=0: {stdout}"
    );
    assert!(
        pedradb_world::allow_claim_tcg_flag_as_is(true, false),
        "AS-IS dente: --claim-tcg on native would pass"
    );
}
