//! `snapshot_backends` benchmark harness: the sorted-ingest route-fold
//! workload used for the published Pedra vs RocksDB vs fjall tables.
//!
//! Ported from [beyondoss/slipstream](https://github.com/beyondoss/slipstream)
//! (MIT license), PR #19 / branch `cursor/pedradb-snapshot-adapter-cb1b`
//! @ `d3bc6a4`, trimmed to the surface the bench exercises:
//!
//! - `kv.rs` — `KvEntry`/`KvUpdate`/`VersionToken`/`WatchCursor` (verbatim).
//! - `snapshot.rs` — the `SnapshotStore` trait and `SnapshotError`, minus the
//!   artifact export/import methods (no `object_store`/`tar` here).
//! - `snapshot_record.rs` — the shared `[ver_len][version][value]` record
//!   codec (verbatim).
//! - `snapshot_{fjall,rocksdb,pedradb}.rs` — the three on-disk backends with
//!   the same tuning constants as upstream; `export_to`/`import` omitted.
//!
//! The append-log backend, NATS watch machinery, and artifact transport of
//! the upstream crate are intentionally absent — this crate exists so the
//! benchmark is reproducible from this repository alone, with Pedra pinned to
//! the in-tree `rocksdb-compat`.

#![deny(unsafe_code)]

mod kv;
pub mod snapshot;
#[cfg(feature = "fjall")]
mod snapshot_fjall;
#[cfg(feature = "pedradb")]
mod snapshot_pedradb;
#[cfg(any(feature = "fjall", feature = "rocksdb", feature = "pedradb"))]
mod snapshot_record;
#[cfg(feature = "rocksdb")]
mod snapshot_rocksdb;

pub use kv::{KvEntry, KvUpdate, VersionToken, WatchCursor};
#[cfg(feature = "fjall")]
pub use snapshot_fjall::{FjallConfig, FjallReader, FjallSnapshot};
#[cfg(feature = "pedradb")]
pub use snapshot_pedradb::{PedraDbConfig, PedraDbReader, PedraDbSnapshot};
#[cfg(feature = "rocksdb")]
pub use snapshot_rocksdb::{RocksDbConfig, RocksDbReader, RocksDbSnapshot};
pub use snapshot::SnapshotStore;

/// RFC-0168 P0.2/P0.3 — per-cell cost lines and the machine-readable
/// per-cell artifact.
///
/// `Guard::new` + Drop wraps one criterion cell: after sampling, the guard
/// prints `cost/<group>/<id>: …` (engine cost counters, only when the engine
/// gate `PEDRA_COST_TRACE` is on) and registers the cell. `flush_group`,
/// called right after the group's `finish()`, appends one CSV row per cell —
/// criterion median (ns, from `estimates.json`) plus the cost columns — to
/// `$SLIPSTREAM_BENCH_ARTIFACT` when that env is set. Non-Pedra backends and
/// gate-off processes still get median rows with zero cost columns.
///
/// The `apply_hydrate` group is out of scope: its criterion ids embed the
/// entry count (`fjall/100000`), and its campaign numbers come from the
/// one-shot `hydrate/<backend>` progress lines instead.
#[cfg(feature = "pedradb")]
pub mod cellcost {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use rocksdb_compat::cost::Snapshot as CostSnapshot;

    type Cell = (String, String, CostSnapshot);

    static CELLS: Mutex<Vec<Cell>> = Mutex::new(Vec::new());

    /// Per-cell guard: create at the top of a `bench_function` closure.
    pub struct Guard {
        group: String,
        id: String,
        start: Option<CostSnapshot>,
    }

    impl Guard {
        /// `group` is the criterion group name, `id` the bench id
        /// (usually the backend name, or `<backend>_<variant>`).
        #[must_use]
        pub fn new(group: &str, id: &str) -> Self {
            Self {
                group: group.to_string(),
                id: id.to_string(),
                start: rocksdb_compat::cost::enabled().then(rocksdb_compat::cost::read),
            }
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            let delta = match self.start {
                Some(start) => {
                    let d = rocksdb_compat::cost::read().since(&start);
                    println!("cost/{}/{}: {}", self.group, self.id, d.line());
                    d
                }
                None => CostSnapshot::default(),
            };
            CELLS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((self.group.clone(), self.id.clone(), delta));
        }
    }

    fn criterion_home() -> PathBuf {
        if let Some(home) = std::env::var_os("CRITERION_HOME") {
            return PathBuf::from(home);
        }
        let target =
            std::env::var_os("CARGO_TARGET_DIR").unwrap_or_else(|| "target".into());
        PathBuf::from(target).join("criterion")
    }

    /// Append one CSV row per registered cell of `group`.
    pub fn flush_group(group: &str) {
        let Some(artifact) = std::env::var_os("SLIPSTREAM_BENCH_ARTIFACT") else {
            // Still drain the registry so long runs do not accumulate.
            drain(group);
            return;
        };
        flush_group_to(&criterion_home(), Path::new(&artifact), group);
    }

    fn drain(group: &str) -> Vec<Cell> {
        let mut all = CELLS.lock().unwrap_or_else(|e| e.into_inner());
        let (matched, rest): (Vec<Cell>, Vec<Cell>) =
            std::mem::take(&mut *all).into_iter().partition(|(g, _, _)| g == group);
        *all = rest;
        matched
    }

    pub(crate) fn flush_group_to(home: &Path, artifact: &Path, group: &str) {        let cells = drain(group);
        if cells.is_empty() {
            return;
        }
        let run = std::env::var("SLIPSTREAM_BENCH_RUN").unwrap_or_default();
        let mut row = String::new();
        if !artifact.exists() {
            row.push_str(
                "run,cell,backend,median_ns,point_ops,point_probes,point_rejected,\
                 point_block_resident,point_block_tls,point_block_file,point_file_bytes,\
                 point_pread_ns,point_image_ns,\
                 scan_ops,scan_sst_probed,scan_block_loads,scan_block_hits,scan_block_bytes\n",
            );
        }
        for (_, id, d) in cells {
            let backend = id.split('_').next().unwrap_or(id.as_str()).to_string();
            let median = criterion_median_ns(home, group, &id)
                .map(|m| format!("{m:.3}"))
                .unwrap_or_else(|| "NA".to_string());
            row.push_str(&format!(
                "{run},{group}/{id},{backend},{median},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                d.point_ops,
                d.point_sst_considered,
                d.point_sst_rejected,
                d.point_block_resident,
                d.point_block_tls,
                d.point_block_file,
                d.point_file_bytes,
                d.point_pread_ns,
                d.point_image_ns,
                d.scan_ops,
                d.scan_sst_probed,
                d.scan_block_loads,
                d.scan_block_hits,
                d.scan_block_bytes,
            ));
        }
        if let Some(parent) = artifact.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut out = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(artifact)
            .expect("open SLIPSTREAM_BENCH_ARTIFACT");
        use std::io::Write as _;
        let _ = out.write_all(row.as_bytes());
    }

    fn criterion_median_ns(home: &Path, group: &str, id: &str) -> Option<f64> {
        let text = std::fs::read_to_string(
            home.join(group).join(id).join("new").join("estimates.json"),
        )
        .ok()?;
        extract_median_ns(&text)
    }

    /// Pull the `median` estimate (nanoseconds) out of criterion's compact
    /// `estimates.json`. Values may use ryu exponent form (`1.2e9`).
    pub(crate) fn extract_median_ns(text: &str) -> Option<f64> {
        let median_at = text.find("\"median\"")?;
        let est_at = text[median_at..].find("\"point_estimate\"")? + median_at;
        let rest = &text[est_at..];
        let colon = rest.find(':')? + 1;
        let rest = rest[colon..].trim_start();
        let end = rest.find(|c: char| {
            !(c.is_ascii_digit() || c == '.' || c == 'e' || c == '+' || c == '-')
        })?;
        rest[..end]
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v > 0.0)
    }
}

#[cfg(test)]
#[cfg(feature = "pedradb")]
mod cellcost_tests {
    use super::cellcost::{extract_median_ns, flush_group_to, Guard};

    #[test]
    fn median_extraction_handles_compact_and_exponent_forms() {
        let compact = r#"{"slope":{"confidence_interval":{"confidence_level":0.95,"lower_bound":1.0,"upper_bound":2.0},"point_estimate":1.5,"standard_error":0.1},"mean":{"confidence_interval":{"confidence_level":0.95,"lower_bound":1.0,"upper_bound":2.0},"point_estimate":1.4,"standard_error":0.1},"median":{"confidence_interval":{"confidence_level":0.95,"lower_bound":1023.1,"upper_bound":1024.9},"point_estimate":1023.4,"standard_error":0.2},"std_dev":{},"median_abs_dev":{"point_estimate":9.5}}"#;
        assert_eq!(extract_median_ns(compact), Some(1023.4));
        let exponent = r#"{"slope":{"point_estimate":1.2e9},"median":{"point_estimate":1.0234e3},"median_abs_dev":{"point_estimate":9.5}}"#;
        assert_eq!(extract_median_ns(exponent), Some(1023.4));
        assert_eq!(extract_median_ns(""), None);
        assert_eq!(extract_median_ns(r#"{"median":{}}"#), None);
    }

    #[test]
    fn flush_writes_header_once_and_joins_median_from_estimates() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().join("criterion");
        let est = home.join("get_hit").join("pedradb").join("new");
        std::fs::create_dir_all(&est).expect("mkdir");
        std::fs::write(
            est.join("estimates.json"),
            r#"{"median":{"point_estimate":1023.4}}"#,
        )
        .expect("write");
        {
            let _g = Guard::new("get_hit", "pedradb");
        } // Drop registers the cell with zero cost (gate off in tests).
        let artifact = tmp.path().join("cells.csv");
        flush_group_to(&home, &artifact, "get_hit");
        flush_group_to(&home, &artifact, "get_hit"); // second flush: nothing new
        let csv = std::fs::read_to_string(&artifact).expect("read");
        let mut lines = csv.lines();
        let header = lines.next().expect("header");
        assert!(header.starts_with("run,cell,backend,median_ns,"));
        let row = lines.next().expect("row");
        assert!(row.starts_with(",get_hit/pedradb,pedradb,1023.400,"));
        assert!(row.ends_with("0")); // zeroed cost columns
        assert!(lines.next().is_none());
    }
}
