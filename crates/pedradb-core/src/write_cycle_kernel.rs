//! Write-cycle forecast (RFC-0192). Integer nanoseconds, no I/O.
//!
//! Twin of [`crate::scale_kernel`] for the **write** path: rank the
//! structural cut from measured WRITEPHASE slices, predict lock-wait
//! under serialized leaders, and forecast QPS if `write()` leaves the
//! wal mutex. The AS-IS tooth is the fire-120 generator: always
//! `LockHold` (500 ns static), independent of the slices.

#![forbid(unsafe_code)]

use std::fmt;

/// Per-op WRITEPHASE slices (nanoseconds). `lock_wait` is **derived**
/// (other leaders' CS) and is not a cut of its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WritePhaseNs {
    /// WAL encode under `wal.lock()`.
    pub wal_encode: u64,
    /// WAL `write()` syscall — still inside `wal.lock()` today (0189 P1.2).
    pub wal_write: u64,
    /// `db.read()` + hint branch (0190 P0.2).
    pub mem_guard: u64,
    /// Memtable / lane write-lock acquire.
    pub mem_lock: u64,
    /// Insert work (BTree / HashMap).
    pub mem_insert: u64,
    /// `published_seq` CAS + epoch RMWs.
    pub publish: u64,
    /// Pipeline walk + complete + settle.
    pub grp: u64,
    /// Time blocked before this leader's CS (measured).
    pub lock_wait: u64,
}

/// Linux p149b quiet overwrite_mc4 (RFC-0189 P0.1), ns/op.
/// Pin for tests and `pedra scale-model write --fixture linux-quiet`.
/// Not a board QPS.
pub const LINUX_QUIET_0189_P01: WritePhaseNs = WritePhaseNs {
    wal_encode: 350,
    wal_write: 890,
    mem_guard: 2_230,
    mem_lock: 380,
    mem_insert: 580,
    publish: 820,
    grp: 270,
    lock_wait: 2_460,
};

/// Structural write cut. `LockHold` exists only as the AS-IS generator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteCut {
    /// Syscall `write()` inside `wal.lock()` (0189 P1.2).
    WalWrite,
    /// Per-group `db.read()` guard (0190 P0.2).
    MemGuard,
    /// Lane / table write lock (0190 P1.1 skiplist iff lanes collapse).
    MemLock,
    /// Insert CPU (0189 P2.2; BTree stays unless this explodes).
    MemInsert,
    /// Publish epoch RMWs (0189 P0.3).
    Publish,
    /// Encode + epilogue (not a structural lock).
    Epilogue,
    /// Fire-120 static diagnose — not a real slice.
    LockHold,
}

impl WriteCut {
    /// Stable token for WRITEPHASE / CLI (`cut=`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WalWrite => "wal_write",
            Self::MemGuard => "mem_guard",
            Self::MemLock => "mem_lock",
            Self::MemInsert => "mem_insert",
            Self::Publish => "publish",
            Self::Epilogue => "epilogue",
            Self::LockHold => "lock_hold",
        }
    }
}

impl fmt::Display for WriteCut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Serial CS the next leader waits on: everything in the pipeline
/// generation except the measured `lock_wait` (that **is** the wait).
#[must_use]
pub fn serial_cs_ns(p: WritePhaseNs) -> u64 {
    p.wal_encode
        .saturating_add(p.wal_write)
        .saturating_add(p.mem_guard)
        .saturating_add(p.mem_lock)
        .saturating_add(p.mem_insert)
        .saturating_add(p.publish)
        .saturating_add(p.grp)
}

/// AS-IS: CS is a constant 2200 ns (the fire-120 `as_is=2200`).
#[must_use]
pub fn serial_cs_ns_as_is(_p: WritePhaseNs) -> u64 {
    2_200
}

/// CS after `write()` leaves `wal.lock()` (0189 P1.2). Encode stays.
#[must_use]
pub fn serial_cs_write_off_lock_ns(p: WritePhaseNs) -> u64 {
    serial_cs_ns(p).saturating_sub(p.wal_write)
}

/// CS after RFC-0193 lands: the pwrite ticket moved **encode + write**
/// off the wal mutex (only the µs-class reserve hop stays serialized —
/// it rides the `grp`/walk slice). This is the as-landed view for the
/// post-P0 forecast: the next ranked cut lives in what is left.
#[must_use]
pub fn serial_cs_ticket_ns(p: WritePhaseNs) -> u64 {
    serial_cs_ns(p)
        .saturating_sub(p.wal_write)
        .saturating_sub(p.wal_encode)
}

/// Upper bound: `L` fully-serialized group leaders share one CS.
/// `L<=1` ⇒ 0. Not board QPS — a ceiling on wait if nobody joins.
#[must_use]
pub fn predicted_lock_wait_ns(cs: u64, leaders: u64) -> u64 {
    match leaders {
        0 | 1 => 0,
        l => cs.saturating_mul(l.saturating_sub(1)) / l,
    }
}

/// AS-IS: lock_wait is the static 500 ns `lock_hold` regardless of L.
#[must_use]
pub fn predicted_lock_wait_ns_as_is(_cs: u64, _leaders: u64) -> u64 {
    500
}

/// Leader cycle = CS + measured (or predicted) lock_wait.
#[must_use]
pub fn cycle_ns(cs: u64, lock_wait: u64) -> u64 {
    cs.saturating_add(lock_wait)
}

/// Ops/s from a per-op cycle in nanoseconds.
#[must_use]
pub fn qps_from_cycle_ns(cycle: u64) -> u64 {
    if crate::write_admission_kernel::batch_is_empty(cycle) {
        0
    } else {
        1_000_000_000 / cycle
    }
}

/// Rank the **structural** slice (max of wr/guard/mlock/mins/publish/enc+grp).
/// `lock_wait` is not a candidate — it is the CS of the others.
#[must_use]
pub fn name_cut(p: WritePhaseNs) -> WriteCut {
    let epilogue = p.wal_encode.saturating_add(p.grp);
    let slices = [
        (p.wal_write, WriteCut::WalWrite),
        (p.mem_guard, WriteCut::MemGuard),
        (p.mem_lock, WriteCut::MemLock),
        (p.mem_insert, WriteCut::MemInsert),
        (p.publish, WriteCut::Publish),
        (epilogue, WriteCut::Epilogue),
    ];
    let mut best = WriteCut::Epilogue;
    let mut best_ns = 0u64;
    for (ns, cut) in slices {
        if ns > best_ns {
            best_ns = ns;
            best = cut;
        }
    }
    best
}

/// AS-IS tooth: always `LockHold` (fire 120), even when guard is 2.23 µs.
#[must_use]
pub fn name_cut_as_is(_p: WritePhaseNs) -> WriteCut {
    WriteCut::LockHold
}

/// Rank the next structural cut **after RFC-0193 lands**: `wal_encode`
/// and `wal_write` left the wal mutex (positional ticket), so they are
/// no longer candidates — the reserve hop rides the epilogue. What
/// remains serialized is what the next fire attacks.
#[must_use]
pub fn name_cut_ticket(p: WritePhaseNs) -> WriteCut {
    let mut q = p;
    q.wal_encode = 0;
    q.wal_write = 0;
    name_cut(q)
}

/// Zipf collapsed onto one lane: max·lanes > 2·total (twice fair share).
#[must_use]
pub fn lane_collapsed(counts: &[u64], lanes: u64) -> bool {
    if lanes <= 1 {
        return false;
    }
    let mut total = 0u64;
    let mut max = 0u64;
    for &c in counts {
        total = total.saturating_add(c);
        if c > max {
            max = c;
        }
    }
    total > 0 && max.saturating_mul(lanes) > total.saturating_mul(2)
}

/// AS-IS: never reports collapse (skiplist never fires).
#[must_use]
pub fn lane_collapsed_as_is(_counts: &[u64], _lanes: u64) -> bool {
    false
}

/// QPS hat if write() is off the wal mutex, using predicted lock_wait.
#[must_use]
pub fn predicted_qps_write_off_lock(p: WritePhaseNs, leaders: u64) -> u64 {
    let cs = serial_cs_write_off_lock_ns(p);
    qps_from_cycle_ns(cycle_ns(cs, predicted_lock_wait_ns(cs, leaders)))
}

// ─────────────────────────────────────────────────────────────────────
// RFC-0192 P0.4 — calibrated forecast tier (2026-09-10 fix,
// `findings/2026-09-10-write-forecast-why-it-missed.md`).
//
// The deterministic `(L−1)/L·CS` wait above is a structural *ranking*
// bound, not a forecast: it assumes deterministic service (Cs²=0) and a
// single serialized pipeline. Measured legs have p99/p50 = 52–65×
// (σ̂ ≈ 1.7–1.8, scv 17–24) plus per-op out-of-phase work, and the
// quoted `qps_hat` landed +1414…+2194‰ off. The calibrated tier below
// predicts the client-visible QPS from the leg's own p50/p99 shape:
// lognormal ⇒ mean = p50·e^{σ̂²/2}, closed loop ⇒ qps = L/mean (Little).
// Integer only: Q=2^20 fixed point, ln via the atanh series, exp via
// Taylor with overflow-checked terms, tail clamped at 1024×p50.

/// Fixed-point scale (2^20) for the integer ln/exp below.
const FP_Q: u64 = 1 << 20;
/// ln 2 × Q (pinned by `rfc0192_ln_fp_pins`).
const FP_LN2_Q: u64 = 726_817;
/// z_{0.99} = 2.32635 × Q — p99 z-score of a normal.
const FP_Z99_Q: u64 = 2_439_353;
/// Tail clamp: p99 ≥ 1024×p50 saturates (misuse guard, declared — not
/// extrapolated). σ² at the clamp ≈ 8.88 keeps `exp_fp_q` in range.
const TAIL_RATIO_CLAMP: u64 = 1024;

/// ln(p99/p50) in Q scale. `0` when p99 ≤ p50 (deterministic) or p50 = 0.
fn ln_ratio_fp_q(p50_ns: u64, p99_ns: u64) -> u64 {
    if p50_ns == 0 || p99_ns <= p50_ns {
        return 0;
    }
    let hi = p50_ns.saturating_mul(TAIL_RATIO_CLAMP);
    let p99 = p99_ns.min(hi);
    let mut m = match p99.checked_mul(FP_Q) {
        Some(v) => v / p50_ns,
        None => TAIL_RATIO_CLAMP * FP_Q,
    };
    let mut e: u64 = 0;
    while m >= 2 * FP_Q {
        m /= 2;
        e += 1;
    }
    // ln m = 2·atanh(x), x = (m−1)/(m+1) ∈ [0, 1/3); 5 odd terms.
    let x = (m - FP_Q) * FP_Q / (m + FP_Q);
    let mut t = x;
    let mut s = x;
    for n in 1..6u64 {
        t = t * x / FP_Q;
        t = t * x / FP_Q;
        s += t / (2 * n + 1);
    }
    e * FP_LN2_Q + 2 * s
}

/// σ̂ = ln(p99/p50)/z_{0.99} in Q scale (lognormal shape from the tail).
fn sigma_fp_q(p50_ns: u64, p99_ns: u64) -> u64 {
    ln_ratio_fp_q(p50_ns, p99_ns) * FP_Q / FP_Z99_Q
}

/// e^y for y in Q scale (y < ~9·Q by construction). Taylor, terms
/// overflow-checked: a term that would overflow is < 1e-4 of the sum.
fn exp_fp_q(y: u64) -> u64 {
    let mut e = FP_Q.saturating_add(y);
    let mut t = y;
    let mut fact: u64 = 1;
    let mut k: u64 = 2;
    while k <= 24 {
        let Some(p) = t.checked_mul(y) else { break };
        t = p / FP_Q;
        let Some(f) = fact.checked_mul(k) else { break };
        fact = f;
        let term = t / fact;
        if term == 0 {
            break;
        }
        e = e.saturating_add(term);
        k += 1;
    }
    e
}

/// σ̂ from the leg's p50/p99, in permille (integer, no floats).
#[must_use]
pub fn sigma_permille_from_p50_p99(p50_ns: u64, p99_ns: u64) -> u64 {
    sigma_fp_q(p50_ns, p99_ns) * 1000 / FP_Q
}

/// Lognormal mean multiplier `e^{σ̂²/2}` × 1000: mean = p50 × this / 1000.
/// Deterministic tail (p99 ≤ p50) ⇒ 1000. Saturates at the 1024× clamp.
#[must_use]
pub fn lognormal_mean_mult_permille(p50_ns: u64, p99_ns: u64) -> u64 {
    let sigma2 = sigma_fp_q(p50_ns, p99_ns).saturating_mul(sigma_fp_q(p50_ns, p99_ns)) / FP_Q;
    exp_fp_q(sigma2 / 2) * 1000 / FP_Q
}

/// scv = e^{σ̂²} − 1 × 1000 — the service-variability term the
/// deterministic `(L−1)/L·CS` wait assumed away (Cs² = 0).
#[must_use]
pub fn scv_permille_from_p50_p99(p50_ns: u64, p99_ns: u64) -> u64 {
    let sigma2 = sigma_fp_q(p50_ns, p99_ns).saturating_mul(sigma_fp_q(p50_ns, p99_ns)) / FP_Q;
    exp_fp_q(sigma2).saturating_sub(FP_Q) * 1000 / FP_Q
}

/// Kingman service-side amplification `(1+scv)/2` × 1000 (Ca² = 1).
/// Explanatory atom for the missed wait; the validated predictor is the
/// lognormal mean above, not a Kingman wait formula.
#[must_use]
pub fn variability_amplification_permille(scv_permille: u64) -> u64 {
    (1000 + scv_permille) / 2
}

/// Calibrated client-visible forecast (RFC-0192 P0.4). This is the tier
/// comparable to a bench QPS; the deterministic tier stays as `tier=ceiling`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteCalibratedForecast {
    /// Closed-loop clients (Little: qps = L/mean).
    pub leaders: u64,
    /// Leg p50 (ns/op, client-visible).
    pub p50_ns: u64,
    /// Leg p99 (ns/op, client-visible).
    pub p99_ns: u64,
    /// σ̂ × 1000.
    pub sigma_permille: u64,
    /// e^{σ̂²} − 1 × 1000.
    pub scv_permille: u64,
    /// (1+scv)/2 × 1000 — the term the deterministic wait missed.
    pub amp_permille: u64,
    /// e^{σ̂²/2} × 1000.
    pub mean_mult_permille: u64,
    /// p50 × mult / 1000.
    pub mean_hat_ns: u64,
    /// leaders·1e9 / mean_hat.
    pub qps_hat: u64,
    /// CS ns removed by a cut view (e.g. 0193: enc+wr off the mutex).
    pub cut_shift_ns: u64,
    /// L·1e9/(mean − cut_shift); `None` when the cut ≥ mean (misuse).
    pub qps_after_cut: Option<u64>,
    /// Measured QPS on the same leg, when there is one.
    pub measured_qps: Option<u64>,
    /// (qps_hat − measured)/measured × 1000 (P1.2; `None` unmeasured).
    pub error_permille: Option<i64>,
}

/// Compose the calibrated tier from the leg's own p50/p99 (integer atoms
/// above; no I/O, no floats).
#[must_use]
pub fn calibrated_forecast(
    leaders: u64,
    p50_ns: u64,
    p99_ns: u64,
    cut_shift_ns: u64,
    measured_qps: Option<u64>,
) -> WriteCalibratedForecast {
    let sigma = sigma_fp_q(p50_ns, p99_ns);
    let sigma2 = sigma.saturating_mul(sigma) / FP_Q;
    let mult = exp_fp_q(sigma2 / 2) * 1000 / FP_Q;
    let scv = exp_fp_q(sigma2).saturating_sub(FP_Q) * 1000 / FP_Q;
    let amp = variability_amplification_permille(scv);
    let mean_hat_ns = p50_ns.saturating_mul(mult) / 1000;
    let qps_hat = if mean_hat_ns == 0 {
        0
    } else {
        leaders.saturating_mul(1_000_000_000) / mean_hat_ns
    };
    let qps_after_cut = if mean_hat_ns > cut_shift_ns {
        Some(leaders.saturating_mul(1_000_000_000) / (mean_hat_ns - cut_shift_ns))
    } else {
        None
    };
    let error_permille = measured_qps.and_then(|m| qps_hat_error_permille(qps_hat, m));
    WriteCalibratedForecast {
        leaders,
        p50_ns,
        p99_ns,
        sigma_permille: sigma * 1000 / FP_Q,
        scv_permille: scv,
        amp_permille: amp,
        mean_mult_permille: mult,
        mean_hat_ns,
        qps_hat,
        cut_shift_ns,
        qps_after_cut,
        measured_qps,
        error_permille,
    }
}

impl WriteCalibratedForecast {
    /// CLI dump. `pedra scale-model write … --p50-ns/--p99-ns` prints
    /// this after the ceiling tier.
    #[must_use]
    pub fn render(self) -> String {
        let after = opt_u64(self.qps_after_cut);
        let measured = opt_u64(self.measured_qps);
        let err = self
            .error_permille
            .map_or_else(|| "-".to_string(), |v| v.to_string());
        format!(
            "tier=forecast (calibrated lognormal p50/p99; qps=L/mean)\n\
             leaders={} p50_ns={} p99_ns={}\n\
             sigma_permille={} scv_permille={} amp_permille={} mean_mult_permille={}\n\
             mean_hat_ns={} qps_hat={}\n\
             cut_shift_ns={} qps_after_cut={}\n\
             measured_qps={} error_permille={}",
            self.leaders,
            self.p50_ns,
            self.p99_ns,
            self.sigma_permille,
            self.scv_permille,
            self.amp_permille,
            self.mean_mult_permille,
            self.mean_hat_ns,
            self.qps_hat,
            self.cut_shift_ns,
            after,
            measured,
            err,
        )
    }
}

fn opt_u64(v: Option<u64>) -> String {
    v.map_or_else(|| "-".to_string(), |x| x.to_string())
}

/// QPS hat after RFC-0193 (encode + write off the wal mutex), predicted
/// wait. The deterministic post-P0 forecast.
#[must_use]
pub fn predicted_qps_ticket(p: WritePhaseNs, leaders: u64) -> u64 {
    let cs = serial_cs_ticket_ns(p);
    qps_from_cycle_ns(cycle_ns(cs, predicted_lock_wait_ns(cs, leaders)))
}

/// Deterministic write-cycle table (RFC-0192). CLI and WRITEPHASE call
/// this; they do not re-rank slices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteCycleForecast {
    /// Ranked structural cut (`name_cut`).
    pub cut: WriteCut,
    /// Fire-120 generator (`name_cut_as_is` — always [`WriteCut::LockHold`]).
    pub cut_as_is: WriteCut,
    /// Serial CS from the measured slices.
    pub cs_ns: u64,
    /// Fire-120 constant CS (2200 ns).
    pub cs_as_is_ns: u64,
    /// Predicted lock_wait under `leaders` fully-serialized group leaders.
    pub lock_wait_hat_ns: u64,
    /// Fire-120 constant lock_hold (500 ns).
    pub lock_wait_as_is_ns: u64,
    /// Leader cycle using **measured** `lock_wait`.
    pub cycle_ns: u64,
    /// `1e9 / cycle_ns`.
    pub qps_hat: u64,
    /// QPS if `write()` leaves `wal.lock()` (0189 P1.2), predicted wait.
    pub off_wr_qps_hat: u64,
    /// RFC-0193 as-landed: CS with encode + write off the wal mutex.
    pub ticket_cs_ns: u64,
    /// RFC-0193 as-landed QPS hat, predicted wait.
    pub ticket_qps_hat: u64,
    /// Next ranked structural cut once 0193 landed (ticket view).
    pub ticket_cut: WriteCut,
    /// Group-leader count used for the wait hat (`L=1` ⇒ wait 0).
    pub leaders: u64,
}

/// Model-vs-measured error in per-mille (P1.2): how far a QPS hat is from
/// the measured ops/s on the same leg. Positive = model over-predicts.
/// `None` when there is no measurement (measured 0) — the error is named,
/// never hidden behind a clamp. Integer permille; floating-point-free.
#[must_use]
pub fn qps_hat_error_permille(qps_hat: u64, measured_qps: u64) -> Option<i64> {
    if crate::write_admission_kernel::batch_is_empty(measured_qps) {
        return None;
    }
    let diff = qps_hat as i64 - measured_qps as i64;
    Some(diff.saturating_mul(1000) / measured_qps as i64)
}

/// Compose the RFC-0192 table from the atomic kernel fns.
#[must_use]
pub fn write_cycle_forecast(p: WritePhaseNs, leaders: u64) -> WriteCycleForecast {
    let cs_ns = serial_cs_ns(p);
    let cycle = cycle_ns(cs_ns, p.lock_wait);
    WriteCycleForecast {
        cut: name_cut(p),
        cut_as_is: name_cut_as_is(p),
        cs_ns,
        cs_as_is_ns: serial_cs_ns_as_is(p),
        lock_wait_hat_ns: predicted_lock_wait_ns(cs_ns, leaders),
        lock_wait_as_is_ns: predicted_lock_wait_ns_as_is(cs_ns, leaders),
        cycle_ns: cycle,
        qps_hat: qps_from_cycle_ns(cycle),
        off_wr_qps_hat: predicted_qps_write_off_lock(p, leaders),
        ticket_cs_ns: serial_cs_ticket_ns(p),
        ticket_qps_hat: predicted_qps_ticket(p, leaders),
        ticket_cut: name_cut_ticket(p),
        leaders,
    }
}

impl WriteCycleForecast {
    /// CLI dump. `pedra scale-model write` prints this verbatim. The
    /// `tier=ceiling` label is load-bearing (P0.4): this number is a
    /// deterministic structural bound for ranking cuts — NOT a board
    /// QPS forecast and not even an upper bound under L>1 clients.
    #[must_use]
    pub fn render(self) -> String {
        format!(
            "tier=ceiling (deterministic; structural ranking bound, not a board qps forecast)\n\
             write-cycle leaders={}\n\
             cut={} cut_as_is={}\n\
             cs_ns={} cs_as_is_ns={}\n\
             lock_wait_hat_ns={} lock_wait_as_is_ns={}\n\
             cycle_ns={} qps_hat={} off_wr_qps_hat={}\n\
             ticket_cut={} ticket_cs_ns={} ticket_qps_hat={}",
            self.leaders,
            self.cut.as_str(),
            self.cut_as_is.as_str(),
            self.cs_ns,
            self.cs_as_is_ns,
            self.lock_wait_hat_ns,
            self.lock_wait_as_is_ns,
            self.cycle_ns,
            self.qps_hat,
            self.off_wr_qps_hat,
            self.ticket_cut.as_str(),
            self.ticket_cs_ns,
            self.ticket_qps_hat,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc0192_qps_error_over_predict_is_positive_permille() {
        // Ticket view hat 278 784 vs a hypothetical measured 250 000 on the
        // same leg: +115‰ (model over-predicts — the honest P1.2 shape).
        assert_eq!(qps_hat_error_permille(278_784, 250_000), Some(115));
    }

    #[test]
    fn rfc0192_qps_error_exact_is_zero_under_is_negative() {
        assert_eq!(qps_hat_error_permille(250_000, 250_000), Some(0));
        // Under-predict by a quarter: −250‰.
        assert_eq!(qps_hat_error_permille(75_000, 100_000), Some(-250));
    }

    #[test]
    fn rfc0192_qps_error_without_measurement_is_none() {
        assert_eq!(qps_hat_error_permille(278_784, 0), None);
    }

    #[test]
    fn rfc0192_linux_quiet_names_guard() {
        let p = LINUX_QUIET_0189_P01;
        assert_eq!(name_cut(p), WriteCut::MemGuard);
        assert_eq!(name_cut_as_is(p), WriteCut::LockHold);
        assert_eq!(name_cut(p).as_str(), "mem_guard");
        assert_eq!(name_cut_as_is(p).as_str(), "lock_hold");
    }

    #[test]
    fn rfc0192_after_guard_names_wal_write() {
        let mut p = LINUX_QUIET_0189_P01;
        p.mem_guard = 0;
        assert_eq!(name_cut(p), WriteCut::WalWrite);
        assert_eq!(name_cut_as_is(p), WriteCut::LockHold);
    }

    #[test]
    fn rfc0192_one_leader_lock_wait_is_zero() {
        let cs = serial_cs_ns(LINUX_QUIET_0189_P01);
        assert_eq!(predicted_lock_wait_ns(cs, 1), 0);
        assert_eq!(predicted_lock_wait_ns(cs, 0), 0);
        assert!(predicted_lock_wait_ns(cs, 4) > 0);
        assert_eq!(predicted_lock_wait_ns(cs, 2), cs / 2);
    }

    #[test]
    fn rfc0192_as_is_always_lock_hold() {
        let p = LINUX_QUIET_0189_P01;
        assert_eq!(name_cut_as_is(p), WriteCut::LockHold);
        assert_eq!(predicted_lock_wait_ns_as_is(serial_cs_ns(p), 1), 500);
        assert_eq!(serial_cs_ns_as_is(p), 2_200);
        assert!(serial_cs_ns(p) > serial_cs_ns_as_is(p));
    }

    #[test]
    fn rfc0192_forecast_is_the_cli_table() {
        let p = LINUX_QUIET_0189_P01;
        let f = write_cycle_forecast(p, 4);
        assert_eq!(f.cut, name_cut(p));
        assert_eq!(f.cut_as_is, WriteCut::LockHold);
        assert_eq!(f.cs_ns, serial_cs_ns(p));
        assert_eq!(f.cs_as_is_ns, serial_cs_ns_as_is(p));
        assert_eq!(f.qps_hat, qps_from_cycle_ns(cycle_ns(f.cs_ns, p.lock_wait)));
        assert_eq!(f.off_wr_qps_hat, predicted_qps_write_off_lock(p, 4));
        assert_eq!(f.lock_wait_as_is_ns, 500);
        assert_eq!(f.leaders, 4);
        let rendered = f.render();
        assert!(
            rendered.contains(&format!("cut={}", f.cut.as_str())),
            "{rendered}"
        );
        assert!(rendered.contains("cut_as_is=lock_hold"), "{rendered}");
    }

    #[test]
    fn rfc0192_lane_collapse_is_max_gt_twice_fair() {
        let zipf = [100u64, 0, 0, 0, 0, 0, 0, 0];
        let fair = [12u64, 12, 12, 12, 12, 12, 12, 12];
        assert!(lane_collapsed(&zipf, 8));
        assert!(!lane_collapsed(&fair, 8));
        assert!(!lane_collapsed(&zipf, 1));
        assert!(!lane_collapsed_as_is(&zipf, 8));
    }

    #[test]
    fn rfc0192_write_off_lock_drops_cs_by_wr() {
        let p = LINUX_QUIET_0189_P01;
        let cs = serial_cs_ns(p);
        let off = serial_cs_write_off_lock_ns(p);
        assert_eq!(cs.saturating_sub(off), p.wal_write);
        let q_now = qps_from_cycle_ns(cycle_ns(cs, predicted_lock_wait_ns(cs, 4)));
        let q_off = predicted_qps_write_off_lock(p, 4);
        assert!(q_off > q_now);
    }

    /// RFC-0193 as-landed: the ticket moved encode + write off the wal
    /// mutex, so the post-P0 forecast serializes strictly less than the
    /// 0189 write-off-lock view and ranks the next cut among the rest.
    #[test]
    fn rfc0193_ticket_view_drops_encode_and_write() {
        let p = LINUX_QUIET_0189_P01;
        let cs = serial_cs_ns(p);
        let ticket = serial_cs_ticket_ns(p);
        assert_eq!(
            cs.saturating_sub(ticket),
            p.wal_write.saturating_add(p.wal_encode)
        );
        assert!(ticket < serial_cs_write_off_lock_ns(p));
        // Ranking: with the two off-lock slices zeroed, guard still leads
        // on the pinned fixture; with guard paid (0190 P0.2 fast path),
        // publish is the next structural cut.
        assert_eq!(name_cut_ticket(p), WriteCut::MemGuard);
        let mut post_guard = p;
        post_guard.mem_guard = 0;
        assert_eq!(name_cut_ticket(post_guard), WriteCut::Publish);
        // Deterministic ordering: as-is < off-lock < ticket QPS hat.
        let f = write_cycle_forecast(post_guard, 4);
        assert!(f.ticket_qps_hat > f.off_wr_qps_hat);
        assert!(f.off_wr_qps_hat > f.qps_hat);
        assert_eq!(f.ticket_cs_ns, serial_cs_ticket_ns(post_guard));
        assert!(f.render().contains(&format!(
            "ticket_cut={} ticket_cs_ns={} ticket_qps_hat={}",
            f.ticket_cut.as_str(),
            f.ticket_cs_ns,
            f.ticket_qps_hat
        )));
    }

    // ── RFC-0192 P0.4: calibrated tier, pinned on the dated legs
    // (`findings/2026-09-09-overwrite-mc4-linux-p149b/serial.md`,
    // 2026-09-10 03:32Z; kernel is the truth for the exact integers).

    /// The 03:32Z isolated-leg trio: the calibrated tier lands within
    /// ±103‰ where the old deterministic hat was off by +1414…+2194‰.
    #[test]
    fn rfc0192_calibrated_forecast_pins_serial_md_legs() {
        // (qps, p50_ns, p99_ns) — verbatim from the finding.
        let legs: [(u64, u64, u64); 3] = [
            (87_262, 8_500, 551_000),
            (115_452, 7_700, 502_000),
            (103_096, 8_500, 446_000),
        ];
        let want: [(u64, i64); 3] = [(94_288, 80), (103_626, -102), (110_518, 71)];
        for ((qps, p50, p99), (want_qps, want_err)) in legs.iter().zip(want) {
            let f = calibrated_forecast(4, *p50, *p99, 0, Some(*qps));
            assert_eq!(f.qps_hat, want_qps, "p50={p50}");
            assert_eq!(f.error_permille, Some(want_err), "p50={p50}");
            assert_eq!(f.leaders, 4);
            assert!(f.sigma_permille >= 1_700 && f.sigma_permille <= 1_800);
            assert!(f.scv_permille >= 17_100 && f.scv_permille <= 24_200);
            assert!(f.mean_mult_permille > 4_000, "mean/p50 must exceed 4x");
            // Little: hat is L·1e9/mean_hat, not 1e9/cycle.
            assert_eq!(f.qps_hat, 4_000_000_000 / f.mean_hat_ns);
        }
    }

    /// The conflation, quantified: the old ceiling error is ≥ 13× the
    /// calibrated error on every leg (P1.2 honesty, not a clamp).
    #[test]
    fn rfc0192_ceiling_error_was_an_order_of_magnitude_worse() {
        let ticket_hat = 278_784u64; // CLI 2026-09-10, --guard 0, L=4.
        let legs: [(u64, u64, u64); 3] = [
            (87_262, 8_500, 551_000),
            (115_452, 7_700, 502_000),
            (103_096, 8_500, 446_000),
        ];
        for (qps, p50, p99) in legs {
            let old = qps_hat_error_permille(ticket_hat, qps).expect("measured");
            let new = calibrated_forecast(4, p50, p99, 0, Some(qps))
                .error_permille
                .expect("measured");
            assert!(old >= 13 * new.abs(), "old={old} new={new}");
            assert!(old > 1_400 && old < 2_200, "old={old}");
            assert!(new.abs() <= 102, "new={new}");
        }
    }

    /// The label bug: on the quiet 06:09Z leg the deterministic
    /// 1e9/(CS+wait) is NOT a bound — it under-predicts the measured QPS
    /// because 4 clients overlap out-of-phase work. −436‰, named.
    #[test]
    fn rfc0192_ceiling_is_not_a_bound_on_quiet_leg() {
        let quiet = qps_from_cycle_ns(cycle_ns(serial_cs_ns(LINUX_QUIET_0189_P01), 2_460));
        assert_eq!(quiet, 125_313);
        assert_eq!(
            qps_hat_error_permille(quiet, 222_125),
            Some(-435),
            "quiet 06:09Z leg: qps_hat under the measured, not a ceiling"
        );
        // The render must now carry the tier label that says exactly this.
        let r = write_cycle_forecast(LINUX_QUIET_0189_P01, 4).render();
        assert!(r.starts_with("tier=ceiling"), "{r}");
        assert!(r.contains("not a board qps forecast"), "{r}");
    }

    /// Deterministic tail is the identity: p99 == p50 ⇒ σ̂ = 0, scv = 0,
    /// mult = 1000, and the forecast degenerates to L/p50.
    #[test]
    fn rfc0192_deterministic_tail_is_identity() {
        assert_eq!(lognormal_mean_mult_permille(8_000, 8_000), 1000);
        assert_eq!(scv_permille_from_p50_p99(8_000, 8_000), 0);
        assert_eq!(variability_amplification_permille(0), 500);
        let f = calibrated_forecast(4, 8_000, 8_000, 0, None);
        assert_eq!(f.mean_hat_ns, 8_000);
        assert_eq!(f.qps_hat, 500_000);
        assert_eq!(f.error_permille, None);
        // p99 < p50 (garbage) also degenerates instead of panicking.
        assert_eq!(lognormal_mean_mult_permille(8_000, 7_000), 1000);
        // Kingman atom: (1+scv)/2 with the r2 scv = 24139 ⇒ 12569.
        assert_eq!(variability_amplification_permille(24_139), 12_569);
    }

    /// Tail clamp: p99 ≥ 1024×p50 saturates instead of extrapolating —
    /// a 2000× and an absurd tail give the same mult, no panic.
    #[test]
    fn rfc0192_tail_clamps_at_1024x() {
        let clamped = lognormal_mean_mult_permille(1_000, 2_000_000);
        assert_eq!(clamped, lognormal_mean_mult_permille(1_000, u64::MAX / 2));
        assert_eq!(clamped, 84_495);
        assert!(sigma_permille_from_p50_p99(1_000, u64::MAX / 2) > 2_900);
    }

    /// Integer ln pins (Q scale): ln 2, ln 10, ln 64.8 — the atanh-series
    /// residue is ≤ 5e-6 on the operating range.
    #[test]
    fn rfc0192_ln_fp_pins() {
        assert_eq!(ln_ratio_fp_q(1_000, 2_000), FP_LN2_Q);
        assert_eq!(ln_ratio_fp_q(1_000, 10_000), 2_414_431); // ln 10 × Q
        assert_eq!(ln_ratio_fp_q(1_000, 64_800), 4_373_926); // ln 64.8 × Q
        assert_eq!(ln_ratio_fp_q(1_000, 1_000), 0);
    }

    /// Cut gains project on the calibrated mean, not the ceiling cycle:
    /// the 0193 cut (enc+wr = 1240 ns off the mutex) is +33‰ on the r2
    /// distribution — not the +60% of ceiling-vs-ceiling arithmetic.
    #[test]
    fn rfc0192_cut_gain_forecasts_on_calibrated_mean() {
        let f = calibrated_forecast(4, 7_700, 502_000, 1_240, Some(115_452));
        assert_eq!(f.qps_hat, 103_626);
        let after = f.qps_after_cut.expect("cut < mean");
        assert_eq!(after, 107_066);
        let gain = (after.saturating_sub(f.qps_hat) * 1000) / f.qps_hat;
        assert_eq!(gain, 33, "3.3%, not the 60% ceiling-view gain");
        // A cut as large as the whole mean is named misuse, not a negative.
        assert_eq!(
            calibrated_forecast(4, 7_700, 502_000, 40_000, None).qps_after_cut,
            None
        );
    }

    /// The calibrated render carries the tier, the atoms and the error.
    #[test]
    fn rfc0192_calibrated_render_carries_tier_and_error() {
        let f = calibrated_forecast(4, 7_700, 502_000, 1_240, Some(115_452));
        let r = f.render();
        assert!(r.starts_with("tier=forecast"), "{r}");
        assert!(r.contains("qps_hat=103626"), "{r}");
        assert!(r.contains("qps_after_cut=107066"), "{r}");
        assert!(r.contains("measured_qps=115452"), "{r}");
        assert!(r.contains("error_permille=-102"), "{r}");
        assert!(r.contains("scv_permille=24139"), "{r}");
        assert!(r.contains("amp_permille=12569"), "{r}");
        let unmeasured = calibrated_forecast(4, 8_500, 551_000, 0, None).render();
        assert!(
            unmeasured.contains("measured_qps=- error_permille=-"),
            "{unmeasured}"
        );
    }
}
