//! C1.3: multi-process smoke — two OS processes run the same World seed and
//! must agree on `trace_hash` (via world_smoke stdout).

use std::process::Command;

fn run_smoke(seed: u64) -> String {
    let out = Command::new("cargo")
        .args([
            "run",
            "--release",
            "--quiet",
            "--bin",
            "world_smoke",
            "--",
            &seed.to_string(),
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("spawn world_smoke");
    assert!(
        out.status.success(),
        "world_smoke failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn extract_hash(s: &str) -> Option<String> {
    for line in s.lines() {
        if let Some(i) = line.find("hash=") {
            let rest = &line[i + 5..];
            let tok = rest.split_whitespace().next().unwrap_or("");
            if !tok.is_empty() {
                return Some(tok.to_string());
            }
        }
    }
    // also accept hex after hash=
    None
}

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(42);
    let a = run_smoke(seed);
    let b = run_smoke(seed);
    let ha = extract_hash(&a).expect("hash from process A");
    let hb = extract_hash(&b).expect("hash from process B");
    assert_eq!(ha, hb, "multi-process replay mismatch");
    println!("multiproc_trace_smoke_ok seed={seed} hash={ha} processes=2");
}
