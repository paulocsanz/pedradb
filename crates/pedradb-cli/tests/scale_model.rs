//! RFC-0176 P1.2: `pedra scale-model` prints the kernel forecast.
//! Numbers come from [`pedradb_core::scale_kernel::scale_forecast`], not a
//! second copy of the formulas.

use pedradb_core::scale_kernel::scale_forecast;
use std::process::Command;

fn pedra() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pedra"))
}

const RAM_64GIB: u64 = 64 << 30;

#[test]
fn rfc0176_scale_forecast_64gib_one_and_ten_billion() {
    let f1 = scale_forecast(1_000_000_000, RAM_64GIB);
    let f10 = scale_forecast(10_000_000_000, RAM_64GIB);
    assert_eq!(f1.p_best, 5, "1B P_best");
    assert_eq!(f10.p_best, 6, "10B P_best");
    assert!(!f1.hot, "1B @ 64 GiB is not hot");
    assert!(!f10.hot, "10B @ 64 GiB is not hot");
    assert!(
        f1.best_ns < f1.happy_ns && f1.happy_ns < f1.worst_ns,
        "1B T {} {} {}",
        f1.best_ns,
        f1.happy_ns,
        f1.worst_ns
    );
    assert!(
        f10.best_ns < f10.happy_ns && f10.happy_ns < f10.worst_ns,
        "10B T {} {} {}",
        f10.best_ns,
        f10.happy_ns,
        f10.worst_ns
    );
}

#[test]
fn rfc0176_pedra_scale_model_prints_kernel_table() {
    let ram = RAM_64GIB.to_string();
    let out1 = pedra()
        .args(["scale-model", "--keys", "1000000000", "--ram", &ram])
        .output()
        .expect("spawn 1B");
    let s1 = String::from_utf8_lossy(&out1.stdout);
    assert!(
        out1.status.success(),
        "1B failed: {}\n{}",
        s1,
        String::from_utf8_lossy(&out1.stderr)
    );
    assert!(s1.contains("P_best=5"), "1B stdout: {s1}");
    assert!(
        s1.contains("mode=bounded-cache") && s1.contains("hot=0"),
        "1B not-hot: {s1}"
    );

    let out10 = pedra()
        .args(["scale-model", "--keys", "10000000000", "--ram", &ram])
        .output()
        .expect("spawn 10B");
    let s10 = String::from_utf8_lossy(&out10.stdout);
    assert!(
        out10.status.success(),
        "10B failed: {}\n{}",
        s10,
        String::from_utf8_lossy(&out10.stderr)
    );
    assert!(s10.contains("P_best=6"), "10B stdout: {s10}");
    assert!(
        s10.contains("mode=bounded-cache") && s10.contains("hot=0"),
        "10B not-hot: {s10}"
    );

    let f1 = scale_forecast(1_000_000_000, RAM_64GIB);
    assert!(
        s1.contains(&format!("T_ns best={}", f1.best_ns)),
        "CLI must print kernel best_ns; stdout={s1}"
    );
}

#[test]
fn rfc0176_scale_model_listed_in_usage() {
    let out = pedra().output().expect("spawn");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("scale-model"),
        "usage must list scale-model: {err}"
    );
}
