//! RFC-0168 P0.2 — per-cell cost accumulators for the parity campaign.
//!
//! Pure atomic counters behind an env gate (`PEDRA_COST_TRACE=1`). When the
//! gate is off every hook is a relaxed atomic load and a not-taken branch —
//! no behavior change, no allocation, no lock. When on, the harness reads
//! deltas around each bench cell and prints one `cost/<cell>/<backend>` line
//! (see `snapshot-bench`), so a campaign leg attributes its latency to
//! probes, block sources (resident pool / TLS raw cache / file pread), and
//! scan windows without a profiler on the gate box.
//!
//! Global (process-wide) on purpose: the benches run one backend per process
//! (`SLIPSTREAM_BENCH_BACKENDS`), so process totals are per-backend totals.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

static ENABLED: OnceLock<bool> = OnceLock::new();

/// `true` when `PEDRA_COST_TRACE` is set to anything but `0`.
#[must_use]
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        std::env::var("PEDRA_COST_TRACE").is_ok_and(|v| v != "0")
    })
}

macro_rules! counters {
    ($($(#[$doc:meta])* $name:ident),+ $(,)?) => {
        static COUNTERS: Counters = Counters {
            $(
                $(#[$doc])*
                $name: AtomicU64::new(0),
            )+
        };

        impl Counters {
            $(
                #[inline]
                pub(crate) fn $name(&self) -> u64 {
                    self.$name.load(Ordering::Relaxed)
                }
            )+
        }
    };
}

struct Counters {
    /// Point-get operations entering the SST fallback path (`Db::lookup`).
    point_ops: AtomicU64,
    /// SSTs considered by a point get (before bounds/bloom gate).
    point_sst_considered: AtomicU64,
    /// SSTs rejected by the bounds/bloom gate without touching a block.
    point_sst_rejected: AtomicU64,
    /// Point blocks served from the resident payload pool.
    point_block_resident: AtomicU64,
    /// Point blocks served from the TLS raw-block cache.
    point_block_tls: AtomicU64,
    /// Point blocks served by a positioned file read.
    point_block_file: AtomicU64,
    /// Bytes moved by those positioned file reads.
    point_file_bytes: AtomicU64,
    /// Nanoseconds spent in positioned file reads (pread stage).
    point_pread_ns: AtomicU64,
    /// Nanoseconds spent decoding/seeking a point block after its bytes are
    /// in hand (CRC + decompress + entry walk, `image` stage).
    point_image_ns: AtomicU64,
    /// Scan operations (one prefix window per iterator creation).
    scan_ops: AtomicU64,
    /// SSTs probed by scans (per-window overlap count).
    scan_sst_probed: AtomicU64,
    /// Scan block loads (block-cache misses that decoded a block).
    scan_block_loads: AtomicU64,
    /// Scan blocks served without a decode (scan cache hit).
    scan_block_hits: AtomicU64,
    /// Bytes of those decoded scan blocks.
    scan_block_bytes: AtomicU64,
}

counters! {
    point_ops,
    point_sst_considered,
    point_sst_rejected,
    point_block_resident,
    point_block_tls,
    point_block_file,
    point_file_bytes,
    point_pread_ns,
    point_image_ns,
    scan_ops,
    scan_sst_probed,
    scan_block_loads,
    scan_block_hits,
    scan_block_bytes,
}

#[inline]
fn bump(counter: &AtomicU64, by: u64) {
    if enabled() {
        counter.fetch_add(by, Ordering::Relaxed);
    }
}

/// A point get entered the SST fallback path.
#[inline]
pub fn point_op() {
    bump(&COUNTERS.point_ops, 1);
}

/// A point get is about to gate one SST (bounds + bloom).
#[inline]
pub fn point_probe() {
    bump(&COUNTERS.point_sst_considered, 1);
}

/// The bounds/bloom gate rejected the SST without a block read.
#[inline]
pub fn point_reject() {
    bump(&COUNTERS.point_sst_rejected, 1);
}

/// A point block was served from the resident payload pool.
#[inline]
pub fn point_block_resident() {
    bump(&COUNTERS.point_block_resident, 1);
}

/// A point block was served from the TLS raw-block cache.
#[inline]
pub fn point_block_tls() {
    bump(&COUNTERS.point_block_tls, 1);
}

/// A point block needed a positioned file read of `bytes`.
#[inline]
pub fn point_block_file(bytes: u64) {
    if enabled() {
        COUNTERS.point_block_file.fetch_add(1, Ordering::Relaxed);
        COUNTERS.point_file_bytes.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// Nanoseconds the positioned file read itself took (pread stage).
#[inline]
pub fn point_pread_ns(ns: u64) {
    bump(&COUNTERS.point_pread_ns, ns);
}

/// Nanoseconds the post-read block work took — CRC + decompress + entry
/// walk (`image` stage), for blocks served from any source.
#[inline]
pub fn point_image_ns(ns: u64) {
    bump(&COUNTERS.point_image_ns, ns);
}

/// One scan window (iterator creation) started.
#[inline]
pub fn scan_op() {
    bump(&COUNTERS.scan_ops, 1);
}

/// A scan probed one SST for its window.
#[inline]
pub fn scan_probe() {
    bump(&COUNTERS.scan_sst_probed, 1);
}

/// A scan block was served without decoding (scan cache hit).
#[inline]
pub fn scan_block_hit() {
    bump(&COUNTERS.scan_block_hits, 1);
}

/// A scan decoded `bytes` of block payload on a cache miss.
#[inline]
pub fn scan_block_load(bytes: u64) {
    if enabled() {
        COUNTERS.scan_block_loads.fetch_add(1, Ordering::Relaxed);
        COUNTERS.scan_block_bytes.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// Point-in-time copy of every counter (values, not deltas).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub point_ops: u64,
    pub point_sst_considered: u64,
    pub point_sst_rejected: u64,
    pub point_block_resident: u64,
    pub point_block_tls: u64,
    pub point_block_file: u64,
    pub point_file_bytes: u64,
    pub point_pread_ns: u64,
    pub point_image_ns: u64,
    pub scan_ops: u64,
    pub scan_sst_probed: u64,
    pub scan_block_loads: u64,
    pub scan_block_hits: u64,
    pub scan_block_bytes: u64,
}

/// Read every counter without clearing.
#[must_use]
pub fn read() -> Snapshot {
    Snapshot {
        point_ops: COUNTERS.point_ops(),
        point_sst_considered: COUNTERS.point_sst_considered(),
        point_sst_rejected: COUNTERS.point_sst_rejected(),
        point_block_resident: COUNTERS.point_block_resident(),
        point_block_tls: COUNTERS.point_block_tls(),
        point_block_file: COUNTERS.point_block_file(),
        point_file_bytes: COUNTERS.point_file_bytes(),
        point_pread_ns: COUNTERS.point_pread_ns(),
        point_image_ns: COUNTERS.point_image_ns(),
        scan_ops: COUNTERS.scan_ops(),
        scan_sst_probed: COUNTERS.scan_sst_probed(),
        scan_block_loads: COUNTERS.scan_block_loads(),
        scan_block_hits: COUNTERS.scan_block_hits(),
        scan_block_bytes: COUNTERS.scan_block_bytes(),
    }
}

impl Snapshot {
    /// `self` as a delta from `earlier` (per-field saturating sub).
    #[must_use]
    pub fn since(&self, earlier: &Snapshot) -> Snapshot {
        Snapshot {
            point_ops: self.point_ops.saturating_sub(earlier.point_ops),
            point_sst_considered: self
                .point_sst_considered
                .saturating_sub(earlier.point_sst_considered),
            point_sst_rejected: self.point_sst_rejected.saturating_sub(earlier.point_sst_rejected),
            point_block_resident: self
                .point_block_resident
                .saturating_sub(earlier.point_block_resident),
            point_block_tls: self.point_block_tls.saturating_sub(earlier.point_block_tls),
            point_block_file: self.point_block_file.saturating_sub(earlier.point_block_file),
            point_file_bytes: self.point_file_bytes.saturating_sub(earlier.point_file_bytes),
            point_pread_ns: self.point_pread_ns.saturating_sub(earlier.point_pread_ns),
            point_image_ns: self.point_image_ns.saturating_sub(earlier.point_image_ns),
            scan_ops: self.scan_ops.saturating_sub(earlier.scan_ops),
            scan_sst_probed: self.scan_sst_probed.saturating_sub(earlier.scan_sst_probed),
            scan_block_loads: self.scan_block_loads.saturating_sub(earlier.scan_block_loads),
            scan_block_hits: self.scan_block_hits.saturating_sub(earlier.scan_block_hits),
            scan_block_bytes: self.scan_block_bytes.saturating_sub(earlier.scan_block_bytes),
        }
    }

    /// One greppable line in the harness' `metric/backend: k=v` house style.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "point ops={} probes={} rejected={} blocks resident={} tls={} file={} file_bytes={} \
             pread_ns={} image_ns={} \
             scan ops={} sst_probed={} block_loads={} block_hits={} block_bytes={}",
            self.point_ops,
            self.point_sst_considered,
            self.point_sst_rejected,
            self.point_block_resident,
            self.point_block_tls,
            self.point_block_file,
            self.point_file_bytes,
            self.point_pread_ns,
            self.point_image_ns,
            self.scan_ops,
            self.scan_sst_probed,
            self.scan_block_loads,
            self.scan_block_hits,
            self.scan_block_bytes
        )
    }
}

/// Decoded byte size of a scan block (user keys + values) for
/// [`scan_block_load`].
#[must_use]
pub fn entries_bytes(entries: &[(crate::key::InternalKey, bytes::Bytes)]) -> u64 {
    entries
        .iter()
        .map(|(k, v)| (k.user_key.len() + v.len()) as u64)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_saturates_and_formats() {
        let a = Snapshot {
            point_ops: 10,
            point_sst_considered: 30,
            point_sst_rejected: 20,
            point_file_bytes: 4096,
            scan_sst_probed: 5,
            ..Snapshot::default()
        };
        let b = Snapshot {
            point_ops: 25,
            point_sst_considered: 70,
            point_sst_rejected: 20,
            point_file_bytes: 12_288,
            scan_sst_probed: 9,
            ..Snapshot::default()
        };
        let d = b.since(&a);
        assert_eq!(d.point_ops, 15);
        assert_eq!(d.point_sst_considered, 40);
        assert_eq!(d.point_sst_rejected, 0);
        assert_eq!(d.point_file_bytes, 8192);
        assert_eq!(d.scan_sst_probed, 4);
        // monotonic readback: a delta against itself is zero
        assert_eq!(a.since(&a), Snapshot::default());
        assert!(d.line().contains("point ops=15"));
        assert!(d.line().contains("sst_probed=4"));
        assert!(d.line().contains("block_hits="));
    }

    #[test]
    fn entries_bytes_sums_user_keys_and_values() {
        let k = crate::key::InternalKey::new("abc", 1, crate::key::ValueType::Value);
        let v = bytes::Bytes::from_static(b"0123456789");
        let entries = vec![(k, v)];
        assert_eq!(entries_bytes(&entries), 3 + 10);
        assert_eq!(entries_bytes(&[]), 0);
    }

    #[test]
    fn bump_hooks_move_counters_when_enabled_in_test() {
        // The env gate defaults off in tests; the hooks must still be callable
        // and (when the test process happens to enable the gate) monotonic.
        let before = read();
        point_op();
        point_probe();
        point_reject();
        point_block_resident();
        point_block_tls();
        point_block_file(128);
        scan_op();
        scan_probe();
        scan_block_hit();
        scan_block_load(256);
        let after = read();
        assert!(after.point_ops >= before.point_ops);
        assert!(after.point_file_bytes >= before.point_file_bytes);
    }
}
