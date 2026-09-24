//! Unified engine backpressure and protection kernel (RFC-0274).
//!
//! Provides pure, deterministic, `#![forbid(unsafe_code)]` kernels for:
//! 1. Compaction I/O rate limiting (token bucket protecting foreground `fdatasync`).
//! 2. Smooth dynamic write pacing (micro-delay pacing preventing sawtooth throughput).
//! 3. Global pending compaction bytes debt watermarks (L1..LN protection).
//! 4. Snapshot pinning lifecycle and tombstone accumulation protection.
//! 5. Value Log (VLog/blob) GC debt backpressure.
//! 6. Concurrency queue depth limit (`max_in_flight_writers`).

#![forbid(unsafe_code)]

use std::time::Duration;

/// Default minimum write pacing micro-delay (20µs).
pub const DEFAULT_MIN_PACE_MICROS: u64 = 20;

/// Default maximum write pacing micro-delay (2,000µs = 2ms).
pub const DEFAULT_MAX_PACE_MICROS: u64 = 2_000;

/// Default soft pending compaction bytes (512 MiB).
pub const DEFAULT_PENDING_COMPACTION_SOFT_BYTES: u64 = 512 * 1024 * 1024;

/// Default hard pending compaction bytes (2 GiB).
pub const DEFAULT_PENDING_COMPACTION_HARD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Default maximum snapshot age before expiration (3,600s = 1 hour).
pub const DEFAULT_MAX_SNAPSHOT_AGE_SECS: u64 = 3_600;

/// Default snapshot sequence lag considered hard/critical (10,000,000 commits).
pub const DEFAULT_SNAPSHOT_PIN_HARD_LAG: u64 = 10_000_000;

/// Default VLog dead garbage percentage triggering automatic GC (30%).
pub const DEFAULT_VLOG_GC_SOFT_RATIO_PCT: u8 = 30;

/// Default VLog dead garbage percentage throttling new blob writes (60%).
pub const DEFAULT_VLOG_GC_HARD_RATIO_PCT: u8 = 60;

/// Default maximum in-flight writer threads in ConcurrentDb.
pub const DEFAULT_MAX_IN_FLIGHT_WRITERS: usize = 1024;

/// Configuration bundle for engine-wide backpressure (RFC-0274).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackpressureConfig {
    /// Compaction I/O write rate limit in bytes/sec (0 = unconstrained).
    pub compaction_rate_bytes_per_sec: u64,
    /// Whether smooth dynamic write pacing is active.
    pub smooth_pacing_enabled: bool,
    /// Minimum pacing delay in microseconds when soft limit is crossed.
    pub min_pace_micros: u64,
    /// Maximum pacing delay in microseconds when approaching hard stall.
    pub max_pace_micros: u64,
    /// Soft threshold for pending compaction debt in bytes.
    pub pending_compaction_soft_bytes: u64,
    /// Hard threshold for pending compaction debt in bytes (triggers stall).
    pub pending_compaction_hard_bytes: u64,
    /// Maximum age in seconds before an active snapshot is marked expired.
    pub max_snapshot_age_secs: u64,
    /// Maximum sequence lag an active snapshot can hold before warnings/refusal.
    pub snapshot_pin_hard_lag: u64,
    /// VLog garbage percentage (dead/total) triggering background GC.
    pub vlog_gc_soft_ratio_pct: u8,
    /// VLog garbage percentage (dead/total) throttling new blob ingest.
    pub vlog_gc_hard_ratio_pct: u8,
    /// Maximum active writers allowed simultaneously in ConcurrentDb.
    pub max_in_flight_writers: usize,
}

impl Default for BackpressureConfig {
    fn default() -> Self {
        Self {
            compaction_rate_bytes_per_sec: 0, // unconstrained unless configured
            smooth_pacing_enabled: true,
            min_pace_micros: DEFAULT_MIN_PACE_MICROS,
            max_pace_micros: DEFAULT_MAX_PACE_MICROS,
            pending_compaction_soft_bytes: 0, // unconstrained default (matches RocksDB default 0)
            pending_compaction_hard_bytes: 0, // unconstrained default (matches RocksDB default 0)
            max_snapshot_age_secs: DEFAULT_MAX_SNAPSHOT_AGE_SECS,
            snapshot_pin_hard_lag: DEFAULT_SNAPSHOT_PIN_HARD_LAG,
            vlog_gc_soft_ratio_pct: DEFAULT_VLOG_GC_SOFT_RATIO_PCT,
            vlog_gc_hard_ratio_pct: DEFAULT_VLOG_GC_HARD_RATIO_PCT,
            max_in_flight_writers: DEFAULT_MAX_IN_FLIGHT_WRITERS,
        }
    }
}

/// Smooth pacing admission plan (RFC-0249 / RFC-0269 / RFC-0274).
#[must_use]
pub fn smooth_pacing_due(enabled: bool) -> bool {
    enabled
}

/// Verdict on global pending compaction debt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactionDebtVerdict {
    /// Compaction debt within normal operating bounds.
    Normal,
    /// Pending compaction exceeds soft threshold; writers should be paced smoothly.
    PaceWriter {
        /// Current pending compaction bytes.
        pending_bytes: u64,
        /// Configured soft pending compaction limit.
        soft_bytes: u64,
    },
    /// Pending compaction exceeds hard threshold; writes must stall to allow compactions to catch up.
    StallCompaction {
        /// Current pending compaction bytes.
        pending_bytes: u64,
        /// Configured hard pending compaction limit.
        hard_bytes: u64,
    },
}

/// Verdict on snapshot pinning and tombstone garbage retention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotPinVerdict {
    /// Snapshot is healthy and within age and sequence lag bounds.
    Normal,
    /// Snapshot has exceeded maximum age and is expired.
    Expired {
        /// Current snapshot age in seconds.
        age_secs: u64,
        /// Maximum allowed snapshot age in seconds.
        max_age_secs: u64,
    },
    /// Snapshot has accumulated high sequence lag, blocking tombstone purge.
    PinLagHigh {
        /// Sequence lag behind tip.
        lag: u64,
        /// Configured hard lag threshold.
        hard_lag: u64,
    },
    /// Snapshot lag is critical and filesystem is reclaiming; refuse new writes to prevent disk blowup.
    RefuseWrites {
        /// Sequence lag behind tip.
        lag: u64,
        /// Configured hard lag threshold.
        hard_lag: u64,
    },
}

/// Verdict on Value Log (VLog/blob) garbage accumulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VlogGcVerdict {
    /// VLog garbage ratio is low.
    Normal,
    /// VLog garbage ratio crossed soft watermark; trigger background GC.
    TriggerGc {
        /// Bytes considered dead/unreachable in vlog.
        dead_bytes: u64,
        /// Total bytes in vlog.
        total_bytes: u64,
        /// Dead garbage percentage (0..100).
        ratio_pct: u8,
    },
    /// VLog garbage ratio crossed hard watermark; throttle new blob writes.
    ThrottleNewBlobs {
        /// Bytes considered dead/unreachable in vlog.
        dead_bytes: u64,
        /// Total bytes in vlog.
        total_bytes: u64,
        /// Dead garbage percentage (0..100).
        ratio_pct: u8,
    },
}

/// Verdict on concurrent submission admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConcurrencyVerdict {
    /// Writer admitted into submission queue.
    Admit,
    /// Submission queue depth exceeded; reject or park writer.
    QueueFull {
        /// Active concurrent writers currently in submission.
        active: usize,
        /// Configured maximum concurrent writers limit.
        limit: usize,
    },
}

/// Token bucket I/O rate limiter for background compaction and vlog GC (RFC-0274 Pillar I).
///
/// Ensures background SST writes do not monopolize NVMe/SSD queue depth and starve
/// foreground `fdatasync` commits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompactionIoPacer {
    /// Configured throughput limit in bytes/sec (0 = unconstrained).
    rate_bytes_per_sec: u64,
    /// Tokens currently available in bucket.
    available_tokens: u64,
    /// Maximum bucket capacity in bytes (burst tolerance).
    burst_capacity: u64,
    /// Monotonic timestamp of last refill (nanoseconds).
    last_refill_nanos: u64,
}

impl CompactionIoPacer {
    /// Create a new pacer with the given rate and initial monotonic timestamp.
    #[must_use]
    pub fn new(rate_bytes_per_sec: u64, initial_nanos: u64) -> Self {
        // Default burst capacity: 100ms worth of tokens, or at least 1 MiB.
        let burst = if rate_bytes_per_sec > 0 {
            (rate_bytes_per_sec / 10).max(1024 * 1024)
        } else {
            0
        };
        Self {
            rate_bytes_per_sec,
            available_tokens: burst,
            burst_capacity: burst,
            last_refill_nanos: initial_nanos,
        }
    }

    /// Set a new rate in bytes/sec (0 = unconstrained).
    pub fn set_rate(&mut self, rate_bytes_per_sec: u64) {
        self.rate_bytes_per_sec = rate_bytes_per_sec;
        let burst = if rate_bytes_per_sec > 0 {
            (rate_bytes_per_sec / 10).max(1024 * 1024)
        } else {
            0
        };
        self.burst_capacity = burst;
        if self.available_tokens > burst {
            self.available_tokens = burst;
        }
    }

    /// Current rate limit in bytes/sec.
    #[must_use]
    pub fn rate_bytes_per_sec(&self) -> u64 {
        self.rate_bytes_per_sec
    }

    /// Refill the token bucket based on elapsed monotonic nanoseconds.
    pub fn refill(&mut self, now_nanos: u64) {
        if self.rate_bytes_per_sec == 0 {
            return;
        }
        if now_nanos <= self.last_refill_nanos {
            return;
        }
        let elapsed_nanos = now_nanos.saturating_sub(self.last_refill_nanos);
        // tokens = (elapsed_nanos * rate) / 1_000_000_000
        let new_tokens = (elapsed_nanos as u128)
            .saturating_mul(self.rate_bytes_per_sec as u128)
            / 1_000_000_000u128;
        if new_tokens > 0 {
            self.available_tokens = (self.available_tokens.saturating_add(new_tokens as u64))
                .min(self.burst_capacity);
            self.last_refill_nanos = now_nanos;
        }
    }

    /// Request pacing delay for a write of `bytes`.
    ///
    /// Returns `None` if the write may proceed immediately without throttling.
    /// Returns `Some(delay)` if the caller should sleep to conform to the rate limit.
    pub fn request_pacing_delay(&mut self, bytes: u64, now_nanos: u64) -> Option<Duration> {
        if self.rate_bytes_per_sec == 0 || bytes == 0 {
            return None;
        }
        self.refill(now_nanos);
        if self.available_tokens >= bytes {
            self.available_tokens = self.available_tokens.saturating_sub(bytes);
            None
        } else {
            let deficit = bytes.saturating_sub(self.available_tokens);
            self.available_tokens = 0;
            // sleep_nanos = (deficit * 1_000_000_000) / rate
            let sleep_nanos = (deficit as u128)
                .saturating_mul(1_000_000_000u128)
                .checked_div(self.rate_bytes_per_sec as u128)
                .unwrap_or(0);
            if sleep_nanos > 0 {
                Some(Duration::from_nanos(sleep_nanos as u64))
            } else {
                None
            }
        }
    }
}

/// Calculate dynamic smooth pacing micro-delay (RFC-0274 Pillar II).
///
/// Ingests pressure metrics (L0 file count and pending compaction bytes) and
/// computes a smooth, linear micro-delay between `min_pace_us` and `max_pace_us`.
///
/// Returns `None` if pressure is strictly within normal limits.
#[must_use]
pub fn calculate_smooth_pacing_delay(
    l0: u64,
    l0_soft: u64,
    l0_hard: u64,
    pending_bytes: u64,
    pending_soft: u64,
    pending_hard: u64,
    min_pace_us: u64,
    max_pace_us: u64,
) -> Option<Duration> {
    // 1. Compute L0 pressure fraction [0.0 .. 1.0] scaled to 1_000_000 ppm
    let l0_fraction_ppm: u64 = if l0_soft > 0 && l0_hard > l0_soft && l0 >= l0_soft {
        let span = l0_hard.saturating_sub(l0_soft);
        let excess = l0.saturating_sub(l0_soft).min(span);
        (excess.saturating_mul(1_000_000)) / span
    } else {
        0
    };

    // 2. Compute pending compaction bytes pressure fraction [0.0 .. 1.0] scaled to 1_000_000 ppm
    let pending_fraction_ppm: u64 =
        if pending_soft > 0 && pending_hard > pending_soft && pending_bytes >= pending_soft {
            let span = pending_hard.saturating_sub(pending_soft);
            let excess = pending_bytes.saturating_sub(pending_soft).min(span);
            (excess.saturating_mul(1_000_000)) / span
        } else {
            0
        };

    let in_l0_soft_band = l0_soft > 0 && l0_hard > l0_soft && l0 >= l0_soft;
    let in_pending_soft_band =
        pending_soft > 0 && pending_hard > pending_soft && pending_bytes >= pending_soft;

    if !in_l0_soft_band && !in_pending_soft_band {
        return None;
    }

    // The active pressure is the maximum of the two axes
    let pressure_ppm = l0_fraction_ppm.max(pending_fraction_ppm);

    // Interpolate smoothly between min_pace_us and max_pace_us
    let pace_span = max_pace_us.saturating_sub(min_pace_us);
    let interpolated_us = min_pace_us.saturating_add(
        (pace_span.saturating_mul(pressure_ppm)) / 1_000_000,
    );

    Some(Duration::from_micros(interpolated_us))
}

/// AS-IS legacy comparison: no smooth pacing (always returns `None`).
#[must_use]
pub fn calculate_smooth_pacing_delay_as_is(
    _l0: u64,
    _l0_soft: u64,
    _l0_hard: u64,
    _pending_bytes: u64,
    _pending_soft: u64,
    _pending_hard: u64,
    _min_pace_us: u64,
    _max_pace_us: u64,
) -> Option<Duration> {
    None
}

/// Evaluate pending compaction debt against configured soft/hard watermarks (RFC-0274 Pillar III).
#[must_use]
pub fn evaluate_compaction_debt(
    pending_bytes: u64,
    soft_bytes: u64,
    hard_bytes: u64,
) -> CompactionDebtVerdict {
    if hard_bytes > 0 && pending_bytes >= hard_bytes {
        CompactionDebtVerdict::StallCompaction {
            pending_bytes,
            hard_bytes,
        }
    } else if soft_bytes > 0 && pending_bytes >= soft_bytes {
        CompactionDebtVerdict::PaceWriter {
            pending_bytes,
            soft_bytes,
        }
    } else {
        CompactionDebtVerdict::Normal
    }
}

/// AS-IS legacy comparison: pending compaction debt is ignored (always `Normal`).
#[must_use]
pub fn evaluate_compaction_debt_as_is(
    _pending_bytes: u64,
    _soft_bytes: u64,
    _hard_bytes: u64,
) -> CompactionDebtVerdict {
    CompactionDebtVerdict::Normal
}

/// Evaluate snapshot pin and age status (RFC-0274 Pillar IV).
#[must_use]
pub fn evaluate_snapshot_pin(
    created_at_secs: u64,
    now_secs: u64,
    max_age_secs: u64,
    oldest_seq: u64,
    last_seq: u64,
    hard_lag: u64,
    disk_reclaiming: bool,
) -> SnapshotPinVerdict {
    let age_secs = now_secs.saturating_sub(created_at_secs);
    if max_age_secs > 0 && age_secs >= max_age_secs {
        return SnapshotPinVerdict::Expired {
            age_secs,
            max_age_secs,
        };
    }

    let lag = last_seq.saturating_sub(oldest_seq);
    if hard_lag > 0 && lag >= hard_lag {
        if disk_reclaiming {
            SnapshotPinVerdict::RefuseWrites { lag, hard_lag }
        } else {
            SnapshotPinVerdict::PinLagHigh { lag, hard_lag }
        }
    } else {
        SnapshotPinVerdict::Normal
    }
}

/// AS-IS legacy comparison: snapshot pins never expire or refuse writes.
#[must_use]
pub fn evaluate_snapshot_pin_as_is(
    _created_at_secs: u64,
    _now_secs: u64,
    _max_age_secs: u64,
    _oldest_seq: u64,
    _last_seq: u64,
    _hard_lag: u64,
    _disk_reclaiming: bool,
) -> SnapshotPinVerdict {
    SnapshotPinVerdict::Normal
}

/// Evaluate VLog (blob) garbage accumulation pressure (RFC-0274 Pillar V).
#[must_use]
pub fn evaluate_vlog_gc_pressure(
    dead_bytes: u64,
    total_bytes: u64,
    soft_pct: u8,
    hard_pct: u8,
) -> VlogGcVerdict {
    if total_bytes == 0 || dead_bytes == 0 {
        return VlogGcVerdict::Normal;
    }

    let ratio_pct = ((dead_bytes as u128 * 100u128) / (total_bytes as u128)) as u8;

    if hard_pct > 0 && ratio_pct >= hard_pct {
        VlogGcVerdict::ThrottleNewBlobs {
            dead_bytes,
            total_bytes,
            ratio_pct,
        }
    } else if soft_pct > 0 && ratio_pct >= soft_pct {
        VlogGcVerdict::TriggerGc {
            dead_bytes,
            total_bytes,
            ratio_pct,
        }
    } else {
        VlogGcVerdict::Normal
    }
}

/// AS-IS legacy comparison: VLog garbage pressure is ignored.
#[must_use]
pub fn evaluate_vlog_gc_pressure_as_is(
    _dead_bytes: u64,
    _total_bytes: u64,
    _soft_pct: u8,
    _hard_pct: u8,
) -> VlogGcVerdict {
    VlogGcVerdict::Normal
}

/// Evaluate concurrent submissions admission (RFC-0274 Pillar VI).
#[must_use]
pub fn evaluate_concurrency_admission(
    active: usize,
    max_writers: usize,
) -> ConcurrencyVerdict {
    if max_writers > 0 && active >= max_writers {
        ConcurrencyVerdict::QueueFull {
            active,
            limit: max_writers,
        }
    } else {
        ConcurrencyVerdict::Admit
    }
}

/// AS-IS legacy comparison: unbounded concurrent writers admission.
#[must_use]
pub fn evaluate_concurrency_admission_as_is(
    _active: usize,
    _max_writers: usize,
) -> ConcurrencyVerdict {
    ConcurrencyVerdict::Admit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compaction_io_pacer_unconstrained() {
        let mut pacer = CompactionIoPacer::new(0, 1_000_000_000);
        assert_eq!(pacer.rate_bytes_per_sec(), 0);
        assert_eq!(pacer.request_pacing_delay(10_000_000, 2_000_000_000), None);
    }

    #[test]
    fn test_compaction_io_pacer_rate_limited() {
        // 10 MB / sec
        let rate = 10 * 1024 * 1024;
        let mut pacer = CompactionIoPacer::new(rate, 0);

        // Consume initial burst
        let _ = pacer.request_pacing_delay(pacer.burst_capacity, 0);

        // Next 10 MB request with 0 elapsed nanos must sleep for 1 second
        let delay = pacer.request_pacing_delay(rate, 0);
        assert!(delay.is_some());
        let delay = delay.unwrap();
        assert_eq!(delay.as_secs(), 1);

        // After 1 second elapsed, refill provides tokens up to burst capacity and delay is None
        pacer.refill(1_000_000_000);
        assert_eq!(pacer.available_tokens, pacer.burst_capacity);
        let delay2 = pacer.request_pacing_delay(pacer.burst_capacity, 1_000_000_000);
        assert_eq!(delay2, None);
    }

    #[test]
    fn test_smooth_pacing_delay_curves() {
        // L0 bounds: 4 soft, 8 hard.
        // Pending bounds: 500MB soft, 1000MB hard.
        // Min delay: 20us, max delay: 1000us.
        let min_us = 20;
        let max_us = 1000;

        // Normal zone: no delay
        assert_eq!(
            calculate_smooth_pacing_delay(3, 4, 8, 100, 500, 1000, min_us, max_us),
            None
        );

        // Exactly at soft limit: minimum delay
        let delay_soft = calculate_smooth_pacing_delay(4, 4, 8, 100, 500, 1000, min_us, max_us);
        assert_eq!(delay_soft, Some(Duration::from_micros(20)));

        // Halfway (6 files out of 4..8): ~510us
        let delay_mid = calculate_smooth_pacing_delay(6, 4, 8, 100, 500, 1000, min_us, max_us);
        assert!(delay_mid.is_some());
        let d = delay_mid.unwrap().as_micros() as u64;
        assert!(d >= 500 && d <= 520, "expected ~510us, got {d}us");

        // Exactly at hard limit: maximum delay
        let delay_hard = calculate_smooth_pacing_delay(8, 4, 8, 100, 500, 1000, min_us, max_us);
        assert_eq!(delay_hard, Some(Duration::from_micros(1000)));

        // AS-IS always returns None
        assert_eq!(
            calculate_smooth_pacing_delay_as_is(8, 4, 8, 1000, 500, 1000, min_us, max_us),
            None
        );
    }

    #[test]
    fn test_evaluate_compaction_debt() {
        let soft = 512 * 1024 * 1024;
        let hard = 2 * 1024 * 1024 * 1024;

        assert_eq!(
            evaluate_compaction_debt(100, soft, hard),
            CompactionDebtVerdict::Normal
        );

        assert_eq!(
            evaluate_compaction_debt(600 * 1024 * 1024, soft, hard),
            CompactionDebtVerdict::PaceWriter {
                pending_bytes: 600 * 1024 * 1024,
                soft_bytes: soft,
            }
        );

        assert_eq!(
            evaluate_compaction_debt(3 * 1024 * 1024 * 1024, soft, hard),
            CompactionDebtVerdict::StallCompaction {
                pending_bytes: 3 * 1024 * 1024 * 1024,
                hard_bytes: hard,
            }
        );

        // AS-IS comparison
        assert_eq!(
            evaluate_compaction_debt_as_is(3 * 1024 * 1024 * 1024, soft, hard),
            CompactionDebtVerdict::Normal
        );
    }

    #[test]
    fn test_evaluate_snapshot_pin() {
        let max_age = 3600;
        let hard_lag = 1_000_000;

        // Normal snapshot
        assert_eq!(
            evaluate_snapshot_pin(1000, 1500, max_age, 500, 600, hard_lag, false),
            SnapshotPinVerdict::Normal
        );

        // Expired snapshot (> 3600s)
        assert_eq!(
            evaluate_snapshot_pin(1000, 5000, max_age, 500, 600, hard_lag, false),
            SnapshotPinVerdict::Expired {
                age_secs: 4000,
                max_age_secs: max_age,
            }
        );

        // High sequence lag without disk reclaiming: PinLagHigh
        assert_eq!(
            evaluate_snapshot_pin(1000, 1500, max_age, 100, 2_000_000, hard_lag, false),
            SnapshotPinVerdict::PinLagHigh {
                lag: 1_999_900,
                hard_lag,
            }
        );

        // High sequence lag WITH disk reclaiming: RefuseWrites
        assert_eq!(
            evaluate_snapshot_pin(1000, 1500, max_age, 100, 2_000_000, hard_lag, true),
            SnapshotPinVerdict::RefuseWrites {
                lag: 1_999_900,
                hard_lag,
            }
        );

        // AS-IS comparison
        assert_eq!(
            evaluate_snapshot_pin_as_is(1000, 5000, max_age, 100, 2_000_000, hard_lag, true),
            SnapshotPinVerdict::Normal
        );
    }

    #[test]
    fn test_evaluate_vlog_gc_pressure() {
        assert_eq!(
            evaluate_vlog_gc_pressure(100, 1000, 30, 60),
            VlogGcVerdict::Normal
        );

        assert_eq!(
            evaluate_vlog_gc_pressure(350, 1000, 30, 60),
            VlogGcVerdict::TriggerGc {
                dead_bytes: 350,
                total_bytes: 1000,
                ratio_pct: 35,
            }
        );

        assert_eq!(
            evaluate_vlog_gc_pressure(700, 1000, 30, 60),
            VlogGcVerdict::ThrottleNewBlobs {
                dead_bytes: 700,
                total_bytes: 1000,
                ratio_pct: 70,
            }
        );

        // AS-IS comparison
        assert_eq!(
            evaluate_vlog_gc_pressure_as_is(700, 1000, 30, 60),
            VlogGcVerdict::Normal
        );
    }

    #[test]
    fn test_evaluate_concurrency_admission() {
        assert_eq!(
            evaluate_concurrency_admission(50, 100),
            ConcurrencyVerdict::Admit
        );

        assert_eq!(
            evaluate_concurrency_admission(100, 100),
            ConcurrencyVerdict::QueueFull {
                active: 100,
                limit: 100,
            }
        );

        // AS-IS comparison
        assert_eq!(
            evaluate_concurrency_admission_as_is(100, 100),
            ConcurrencyVerdict::Admit
        );
    }
}
