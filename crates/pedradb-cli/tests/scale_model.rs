//! RFC-0176 P1.2: `pedra scale-model` prints the kernel forecast.
//! Numbers come from [`pedradb_core::scale_kernel::scale_forecast`], not a
//! second copy of the formulas.

use pedradb_core::ratio_curve_kernel::{
    linux_overwrite_mc4_hat, ratio_curve_table, render_get_side_anchors,
    GET_SIDE_ANCHORS_2026_09_10, WRITE_FAMILY_ANCHORS_2026_09_10,
};
use pedradb_core::scale_kernel::scale_forecast;
use pedradb_core::write_cycle_kernel::{write_cycle_forecast, LINUX_QUIET_0189_P01};
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
    assert!(
        s1.contains("get_hit_qps_hat="),
        "GET clock must emit linux_get_hit_qps_hat; stdout={s1}"
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

#[test]
fn rfc0192_pedra_scale_model_write_prints_kernel() {
    let out = pedra()
        .args([
            "scale-model",
            "write",
            "--fixture",
            "linux-quiet",
            "--leaders",
            "4",
        ])
        .output()
        .expect("spawn write fixture");
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "write fixture failed: {}\n{}",
        s,
        String::from_utf8_lossy(&out.stderr)
    );
    let want = write_cycle_forecast(LINUX_QUIET_0189_P01, 4).render();
    assert_eq!(
        s.trim_end(),
        want.trim_end(),
        "CLI must print write_cycle_forecast.render()"
    );
    assert!(s.contains("tier=ceiling"));
    assert!(s.contains("cut=mem_guard"));
}

#[test]
fn linux_overwrite_mc4_hat_cli_equals_kernel() {
    let ram = (4u64 << 30).to_string();
    let out = pedra()
        .args([
            "scale-model",
            "write",
            "--keys",
            "25000000",
            "--ram",
            &ram,
            "--clients",
            "4",
        ])
        .output()
        .expect("spawn write hat");
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "write hat failed: {}\n{}",
        s,
        String::from_utf8_lossy(&out.stderr)
    );
    let want = linux_overwrite_mc4_hat(25_000_000, 4 << 30, 4).render();
    assert_eq!(s.trim_end(), want.trim_end());
    assert!(s.contains("host=linux-quiet"));
    assert!(s.contains("hat=1 (not cartaz)"));
}

#[test]
fn linux_overwrite_mc4_hat_cli_is_deterministic() {
    let ram = (4u64 << 30).to_string();
    let args = [
        "scale-model",
        "write",
        "--keys",
        "25000000",
        "--ram",
        ram.as_str(),
        "--clients",
        "4",
    ];
    let a = pedra().args(args).output().expect("run a");
    let b = pedra().args(args).output().expect("run b");
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b.stdout);
}

#[test]
fn rfc0197_scale_model_ratio_prints_curve() {
    let ram = (4u64 << 30).to_string();
    let out = pedra()
        .args(["scale-model", "ratio", "--ram", &ram])
        .output()
        .expect("spawn ratio");
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "ratio failed: {}\n{}",
        s,
        String::from_utf8_lossy(&out.stderr)
    );
    let mut p = LINUX_QUIET_0189_P01;
    p.mem_guard = 0;
    let scales = [100_000u64, 2_000_000, 15_000_000, 25_000_000, 100_000_000];
    let table = ratio_curve_table(&WRITE_FAMILY_ANCHORS_2026_09_10, &scales, 4 << 30, p);
    let curve = table.render();
    let gets = render_get_side_anchors(&GET_SIDE_ANCHORS_2026_09_10);
    assert!(
        s.contains(curve.trim_end()),
        "CLI must contain ratio_curve_table.render(); stdout={s}"
    );
    assert!(
        s.contains(gets.trim_end()),
        "CLI must contain GET-side anchors; stdout={s}"
    );
    assert!(s.contains("ratio_hat_permille=417"));
    assert!(s.contains("measured_ratio_permille=557"));
}
