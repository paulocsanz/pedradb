//! Disk-pressure watermarks (RFC-0179).
//!
//! Unknown probe (`None`) never proactive-refuses: a bad `statvfs` must
//! not take production writes offline. Below the hard floor the write is
//! `Refuse` so WAL append never runs at 0 free bytes. Between hard and
//! soft the handler may compact / drop page cache, then re-check.
//! Compact itself writes a new SST — allowed only while still ≥ hard.
//!
//! AS-IS always admits (the hole: WAL append at ENOSPC).

#![forbid(unsafe_code)]

/// Soft floor: reclaim (compact + `DONTNEED`) then re-check.
pub const DISK_SOFT_FREE_BYTES: u64 = 256 * 1024 * 1024;
/// Hard floor: refuse the write. Reads stay up; not a durability fence.
pub const DISK_HARD_FREE_BYTES: u64 = 64 * 1024 * 1024;

/// Verdict after measuring free bytes (reclaim I/O is glue).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskPressureAdmit {
    /// Write may proceed (plenty of space, or probe unknown).
    Ok,
    /// Below soft, still ≥ hard: compact / drop cache, then re-check.
    Reclaim,
    /// Below hard: do not append WAL.
    Refuse {
        /// Bytes the probe reported free.
        available: u64,
        /// Hard floor that was missed.
        need: u64,
    },
}

/// Map an `Env::available_bytes` result onto the watermark domain.
///
/// `ok = false` is a failed probe (`statvfs` Err, DST inject). That is
/// **unknown**, never 0 free — a bad probe must not take writes offline
/// (RFC-0179). Glue (`admit_disk_write`, `ensure_disk_pressure_admitted`)
/// matches this fn.
#[must_use]
pub fn disk_probe_or_unknown(ok: bool, value: Option<u64>) -> Option<u64> {
    match ok {
        true => value,
        false => None,
    }
}

/// AS-IS hole: probe Err is treated as 0 free bytes (false-refuse).
#[must_use]
pub fn disk_probe_or_unknown_as_is(ok: bool, value: Option<u64>) -> Option<u64> {
    match ok {
        true => value,
        false => Some(0),
    }
}

/// Admit a write given `Env::available_bytes` (`None` = unknown).
#[must_use]
pub fn disk_pressure_admit(available: Option<u64>) -> DiskPressureAdmit {
    match available {
        None => DiskPressureAdmit::Ok,
        Some(n) if n >= DISK_SOFT_FREE_BYTES => DiskPressureAdmit::Ok,
        Some(n) if n >= DISK_HARD_FREE_BYTES => DiskPressureAdmit::Reclaim,
        Some(n) => DiskPressureAdmit::Refuse {
            available: n,
            need: DISK_HARD_FREE_BYTES,
        },
    }
}

/// AS-IS: always admit — WAL append proceeds into ENOSPC.
#[must_use]
pub fn disk_pressure_admit_as_is(_available: Option<u64>) -> DiskPressureAdmit {
    DiskPressureAdmit::Ok
}

/// Compact writes a new SST. Only while the probe is still ≥ hard.
#[must_use]
pub fn compact_allowed_under_pressure(available: Option<u64>) -> bool {
    match available {
        None => true,
        Some(n) => n >= DISK_HARD_FREE_BYTES,
    }
}

/// AS-IS: compact even at 0 free (the SST write is the ENOSPC).
#[must_use]
pub fn compact_allowed_under_pressure_as_is(_available: Option<u64>) -> bool {
    true
}

/// Which reclaim I/O the live engine runs while still ≥ hard (RFC-0179 P1.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskReclaimPlan {
    /// Compact SST levels (P0 already did this).
    pub compact_sst: bool,
    /// Rotate/recycle the current WAL segment.
    pub rotate_wal: bool,
    /// GC the value log / blobs.
    pub compact_vlog: bool,
}

/// Reclaim plan: SST compact **and** WAL recycle **and** vlog GC, or nothing
/// when compact is forbidden (below hard).
#[must_use]
pub fn disk_pressure_reclaim_plan(allowed: bool) -> DiskReclaimPlan {
    DiskReclaimPlan {
        compact_sst: allowed,
        rotate_wal: allowed,
        compact_vlog: allowed,
    }
}

/// AS-IS hole: SST compact only — WAL recycle / vlog GC never run on reclaim.
#[must_use]
pub fn disk_pressure_reclaim_plan_as_is(allowed: bool) -> DiskReclaimPlan {
    DiskReclaimPlan {
        compact_sst: allowed,
        rotate_wal: false,
        compact_vlog: false,
    }
}

/// PITR dest / backup sink / HA replica WAL: same hard floor as live `put`.
///
/// Reclaim is still admitted — those callers have nothing to compact on an
/// empty dest. Soft-floor reclaim stays the live engine's job.
#[must_use]
pub fn external_write_admitted(available: Option<u64>) -> bool {
    !matches!(
        disk_pressure_admit(available),
        DiskPressureAdmit::Refuse { .. }
    )
}

/// AS-IS: external copy/append proceeds into ENOSPC (torn dest / replica WAL).
#[must_use]
pub fn external_write_admitted_as_is(_available: Option<u64>) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_probe_is_ok() {
        assert_eq!(disk_pressure_admit(None), DiskPressureAdmit::Ok);
        assert_eq!(
            disk_pressure_admit_as_is(None),
            DiskPressureAdmit::Ok,
            "AS-IS agrees on unknown"
        );
    }

    #[test]
    fn plenty_is_ok() {
        assert_eq!(
            disk_pressure_admit(Some(DISK_SOFT_FREE_BYTES)),
            DiskPressureAdmit::Ok
        );
        assert_eq!(
            disk_pressure_admit(Some(DISK_SOFT_FREE_BYTES + 1)),
            DiskPressureAdmit::Ok
        );
    }

    #[test]
    fn between_hard_and_soft_is_reclaim() {
        assert_eq!(
            disk_pressure_admit(Some(DISK_HARD_FREE_BYTES)),
            DiskPressureAdmit::Reclaim
        );
        assert_eq!(
            disk_pressure_admit(Some(DISK_SOFT_FREE_BYTES - 1)),
            DiskPressureAdmit::Reclaim
        );
    }

    #[test]
    fn below_hard_is_refuse() {
        let got = disk_pressure_admit(Some(DISK_HARD_FREE_BYTES - 1));
        assert_eq!(
            got,
            DiskPressureAdmit::Refuse {
                available: DISK_HARD_FREE_BYTES - 1,
                need: DISK_HARD_FREE_BYTES,
            }
        );
        assert_eq!(
            disk_pressure_admit(Some(0)),
            DiskPressureAdmit::Refuse {
                available: 0,
                need: DISK_HARD_FREE_BYTES,
            }
        );
        assert_eq!(
            disk_pressure_admit_as_is(Some(0)),
            DiskPressureAdmit::Ok,
            "AS-IS dente: zero free still admits"
        );
    }

    #[test]
    fn disk_probe_or_unknown_on_live_probe_is_not_ok() {
        assert_eq!(disk_probe_or_unknown(true, Some(1024)), Some(1024));
        assert_eq!(disk_probe_or_unknown(true, None), None);
        assert_eq!(disk_probe_or_unknown(false, Some(0)), None);
        assert_eq!(disk_probe_or_unknown(false, None), None);
        assert_eq!(
            disk_probe_or_unknown_as_is(false, None),
            Some(0),
            "AS-IS dente: probe Err is 0 free (false-refuse)"
        );
        assert_eq!(
            disk_pressure_admit(disk_probe_or_unknown(false, None)),
            DiskPressureAdmit::Ok
        );
        assert_eq!(
            disk_pressure_admit(disk_probe_or_unknown_as_is(false, None)),
            DiskPressureAdmit::Refuse {
                available: 0,
                need: DISK_HARD_FREE_BYTES,
            }
        );
        let glue = include_str!("env.rs")
            .split("pub fn probe_available_bytes")
            .nth(1)
            .and_then(|s| s.split("pub fn admit_disk_write").next())
            .expect("probe_available_bytes");
        assert!(
            glue.contains("disk_probe_or_unknown("),
            "probe_available_bytes must match disk_probe_or_unknown"
        );
        let admit = include_str!("db.rs")
            .split("fn ensure_disk_pressure_admitted")
            .nth(1)
            .and_then(|s| s.split("fn reclaim_disk_for_uptime").next())
            .expect("ensure_disk_pressure_admitted");
        assert!(
            admit.contains("probe_available_bytes("),
            "ensure_disk_pressure_admitted must use probe_available_bytes"
        );
    }

    #[test]
    fn external_write_admitted_on_live_admit_disk_write_is_not_ok() {
        assert!(external_write_admitted(None));
        assert!(external_write_admitted(Some(DISK_HARD_FREE_BYTES)));
        assert!(!external_write_admitted(Some(0)));
        assert!(
            external_write_admitted_as_is(Some(0)),
            "AS-IS dente: dest copy proceeds at zero free"
        );
        let glue = include_str!("env.rs")
            .split("pub fn admit_disk_write")
            .nth(1)
            .and_then(|s| s.split("fn note_external_disk_pressure").next())
            .expect("admit_disk_write");
        assert!(
            glue.contains("external_write_admitted("),
            "admit_disk_write must match external_write_admitted"
        );
        assert!(
            glue.contains("probe_available_bytes("),
            "admit_disk_write must probe via disk_probe_or_unknown"
        );
        let copy = include_str!("db.rs")
            .split("pub fn copy_db_directory")
            .nth(1)
            .and_then(|s| s.split("\npub fn ").next())
            .expect("copy_db_directory");
        assert!(
            copy.contains("admit_disk_write("),
            "copy_db_directory must admit before copy"
        );
        let ckpt = include_str!("db.rs")
            .split("pub fn create_checkpoint")
            .nth(1)
            .and_then(|s| s.split("\n    pub fn ").next())
            .expect("create_checkpoint");
        assert!(
            ckpt.contains("admit_disk_write("),
            "create_checkpoint must admit before copy"
        );
        let hist = include_str!("../../pedradb-ops/src/lib.rs")
            .split("pub fn restore_history_from_remote")
            .nth(1)
            .and_then(|s| s.split("\nfn write_warch").next())
            .expect("restore_history_from_remote");
        assert!(
            hist.contains("admit_disk_write("),
            "restore_history_from_remote must admit before writing dest"
        );
        let group = include_str!("db.rs")
            .split("fn group_admit")
            .nth(1)
            .and_then(|s| s.split("fn group_prepare").next())
            .expect("group_admit");
        assert!(
            group.contains("CoreError::DiskPressure"),
            "group_admit must keep DiskPressure, not map it to Internal"
        );
        let submit = include_str!("concurrent.rs")
            .split("fn submit_inner")
            .nth(1)
            .and_then(|s| s.split("fn submit_after_begin").next())
            .expect("submit_inner");
        assert!(
            submit.contains("DiskPressure is not a stall"),
            "write-group must not park/retry DiskPressure"
        );
        assert!(
            submit.contains("WriteStallMem"),
            "only WriteStall/WriteStallMem retry"
        );
        let del = include_str!("db.rs")
            .split("pub fn delete_with(")
            .nth(1)
            .and_then(|s| s.split("pub fn delete_range(").next())
            .expect("delete_with");
        assert!(
            del.contains("apply_batch_with("),
            "delete_with must go through apply_batch (disk admit)"
        );
        let arc = include_str!("../../pedradb-sim/src/failing_arc.rs")
            .split("fn available_bytes")
            .nth(1)
            .and_then(|s| s.split("\n}").next())
            .expect("FailingEnvArc::available_bytes");
        assert!(
            arc.contains("probe_err"),
            "FailingEnvArc must inject probe Err (unknown, not 0-free)"
        );
        let compact = include_str!("db.rs")
            .split("pub fn compact_with(")
            .nth(1)
            .and_then(|s| s.split("pub fn compact_reclaim").next())
            .expect("compact_with");
        assert!(
            compact.contains("compact_allowed_under_pressure("),
            "compact_with must match compact_allowed_under_pressure"
        );
        let flush = include_str!("db.rs")
            .split("pub fn flush(")
            .nth(1)
            .and_then(|s| s.split("pub(crate) fn bulk_family_of_table").next())
            .expect("flush");
        assert!(
            flush.contains("compact_allowed_under_pressure("),
            "flush must match compact_allowed_under_pressure"
        );
    }

    #[test]
    fn disk_pressure_reclaim_plan_on_live_reclaim_is_not_ok() {
        let live = disk_pressure_reclaim_plan(true);
        assert!(live.compact_sst && live.rotate_wal && live.compact_vlog);
        let as_is = disk_pressure_reclaim_plan_as_is(true);
        assert!(as_is.compact_sst);
        assert!(!as_is.rotate_wal, "AS-IS dente: no WAL recycle");
        assert!(!as_is.compact_vlog, "AS-IS dente: no vlog GC");
        let denied = disk_pressure_reclaim_plan(false);
        assert!(!denied.compact_sst && !denied.rotate_wal && !denied.compact_vlog);
        let body = include_str!("db.rs")
            .split("fn reclaim_disk_for_uptime")
            .nth(1)
            .and_then(|s| s.split("fn drop_page_cache_best_effort").next())
            .expect("reclaim_disk_for_uptime");
        assert!(
            body.contains("disk_pressure_reclaim_plan("),
            "reclaim_disk_for_uptime must match disk_pressure_reclaim_plan"
        );
        assert!(body.contains("plan.rotate_wal"), "must recycle WAL");
        assert!(body.contains("plan.compact_vlog"), "must GC vlog");
    }

    #[test]
    fn compact_forbidden_below_hard() {
        assert!(compact_allowed_under_pressure(None));
        assert!(compact_allowed_under_pressure(Some(DISK_HARD_FREE_BYTES)));
        assert!(!compact_allowed_under_pressure(Some(
            DISK_HARD_FREE_BYTES - 1
        )));
        assert!(
            compact_allowed_under_pressure_as_is(Some(0)),
            "AS-IS dente: compact at zero free"
        );
    }

    #[test]
    fn external_write_admits_reclaim_refuses_hard() {
        assert!(external_write_admitted(None));
        assert!(external_write_admitted(Some(DISK_HARD_FREE_BYTES)));
        assert!(!external_write_admitted(Some(DISK_HARD_FREE_BYTES - 1)));
        assert!(
            external_write_admitted_as_is(Some(0)),
            "AS-IS dente: PITR/replica append at zero free"
        );
    }
}
