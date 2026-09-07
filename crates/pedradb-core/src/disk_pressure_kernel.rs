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
}
