//! RFC-0058 P0.1: the verified profile — a **declared** composition of
//! proven kernels instead of an accident of platform defaults.
//!
//! What "verified" means here (and no more): every critical section the
//! mode executes is either (a) a kernel proved in the formal pipeline and
//! cataloged in `scripts/formal/catalog.json`, or (b) a published contract
//! enforced in code and exercised by the DST battery (crash / reopen /
//! EIO). It does **not** mean code extracted from theorems (rejected,
//! LEDGER L46 / `VeriBetrKV` 8×) and it does not mean "no bugs" — the
//! residual (OS lying on fsync, hardware, field) stays published in
//! `research/DST-VS-FDB-SIM.md`.
//!
//! Composition (file level, [`OpenOptions::verified`]):
//! `sync=true`, `wal_full_fsync=true` (strongest data class),
//! `wal_recovery=FailClosed`. Composition (group level,
//! [`ConcurrentDb::pin_verified`]), reactivated by RFC-0058 P2.1 with the
//! proved group-commit kernel (RFC-0057 P2.1): the leader/member merge
//! runs (`group_commit_kernel` decides first-committer-wins and group
//! atomicity; `fence_publish_seq` publishes the group at one watermark),
//! the catch-up window is pinned to 0 (merging by natural queuing, never
//! by a delay window), and async writers keep the un-merged bypass (no
//! leader dependency). Product constructors pin `StdEnv` — the `io_uring`
//! ring is out of the mode (P2.2); the full mode keeps its
//! `PosixFallback`.
//!
//! [`profile_report`] is the machine-checked tie to the catalog: the set
//! of ON kernels must equal the catalog pair ids exactly. Adding a kernel
//! to the catalog without claiming it here (On, or flipping an Off entry
//! to On) fails `verified_report_matches_catalog` — the report is a
//! living ritual, not documentation.

use crate::concurrent::ConcurrentDb;
use crate::db::{OpenOptions, WalRecovery};
use crate::env::{Env, StdEnv};
use crate::Result;
use std::path::Path;

/// Version tag of the declared composition (report format, not semver).
/// v2 = RFC-0058 P2.1: the merge is back with the proved kernel.
pub const PROFILE_VERSION: &str = "verified-v2";

/// Whether a component of the mode is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileState {
    /// Active in the verified mode.
    On,
    /// Deliberately off in the verified mode (see `note` for the gate).
    Off,
}

/// One row of [`profile_report`]: component ⇒ state ⇒ kernel.
///
/// `kernel` is the id of a `pairs` entry in `scripts/formal/catalog.json`
/// (`None` = a published contract without a theorem — see the note).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileComponent {
    /// Component name (stable string; mirrors the catalog id when one
    /// exists).
    pub component: &'static str,
    /// On / Off in this mode.
    pub state: ProfileState,
    /// Catalog kernel id backing the component, when one exists.
    pub kernel: Option<&'static str>,
    /// Why this state holds / what gates a change.
    pub note: &'static str,
}

macro_rules! on {
    ($c:expr, $k:expr, $n:expr) => {
        ProfileComponent {
            component: $c,
            state: ProfileState::On,
            kernel: Some($k),
            note: $n,
        }
    };
}

macro_rules! off {
    ($c:expr, $n:expr) => {
        ProfileComponent {
            component: $c,
            state: ProfileState::Off,
            kernel: None,
            note: $n,
        }
    };
}

macro_rules! contract {
    ($c:expr, $n:expr) => {
        ProfileComponent {
            component: $c,
            state: ProfileState::On,
            kernel: None,
            note: $n,
        }
    };
}

/// Component ⇒ state ⇒ kernel for the verified mode (RFC-0058 P0.1).
///
/// Invariant (machine-checked by `verified_report_matches_catalog`): the
/// set of `kernel` ids over ON entries equals the catalog `pairs` ids —
/// every proved kernel is claimed by the mode and nothing else is.
#[must_use]
pub fn profile_report() -> &'static [ProfileComponent] {
    &[
        on!("wal_recover", "wal_recover", "fail-closed prefix recovery at open (F4/F14)"),
        on!("manifest_recover", "manifest_recover", "MANIFEST recovery (F196/G8)"),
        on!("reopen_outcome", "reopen_outcome", "reopen state equals the pre-crash visible state (F170/F171/G8)"),
        on!("dictionary_link", "dictionary_link", "crash-dictionary put→get link across flush/crash (G1/G8)"),
        on!("vlog_recover", "vlog_recover", "value-log recovery (F51)"),
        on!("blob_gc_pick", "blob_gc_pick", "blob GC picks the active generation"),
        on!("write_group_merge", "group_commit", "leader/member merge with the proved kernel: first-committer-wins cross-group, group atomicity intra-group (RFC-0057 P2.1 / RFC-0051 P1.3)"),
        on!("group_fence", "group_fence", "one publish watermark per group = max appended member sequence, after WAL durability (RFC-0057 P2.1)"),
        on!("flush_decision", "flush_decision", "when to flush (F2/F43/G1)"),
        on!("flush_publish", "flush_publish", "MANIFEST after durable SST (RFC-0151 P1)"),
        on!("compact_decision", "compact_decision", "when to compact (F177/F20)"),
        on!("compact_retention", "compact_retention", "what compaction retains (F177/F20)"),
        on!("leveling", "leveling", "level-size ladder of the leveled scheduler (F-leveling-sweep)"),
        on!("leveling_pick", "leveling_pick", "leveled job selection: overlap slice, input cap, disjoint gate (F-leveling-sweep)"),
        on!("leveling_pushdown", "leveling_pushdown", "leveled pushdown: disjoint-gated oldest-chunk demotion (F-leveling-sweep)"),
        on!("changelog", "changelog", "changelog decode/replay (F53)"),
        on!("range_covers", "range_covers", "range bounds cover the requested span (F30)"),
        on!("prefix", "prefix", "prefix seek bounds (F57/F58)"),
        on!("scan_guard", "scan_guard", "scan lifetime guard (F167)"),
        on!("bloom_header", "bloom_header", "bloom header validation (T1)"),
        on!("bloom_insert", "bloom_insert", "bloom insert (T1)"),
        on!("bloom_may_contain", "bloom_may_contain", "bloom probe — no false negatives (T1/T4)"),
        on!("group_publish", "group_publish", "group publish after WAL durable"),
        on!("forall_schedules", "forall_schedules", "PCT depth is not ∀ schedules"),
        on!("fsync_promote", "fsync_promote", "fsync promotes pending"),
        on!("media_durable", "media_durable", "fsync Ok is not media proof"),
        on!("fdatasync_rc", "fdatasync_rc", "fdatasync nonzero rc is not Ok"),
        on!("cqe_res", "cqe_res", "negative CQE res is not Ok"),
        on!("crc_match", "crc_match", "CRC mismatch is not Ok"),
        on!("sst_crc", "sst_crc", "SST CRC fate fail-closed"),
        on!("cf_family", "cf_family", "CF family membership / encode (scan leak fail-closed)"),
        on!("visible_at", "visible_at", "snapshot merge visibility + F30 range tombstone"),
        on!("ikey_pack", "ikey_pack", "InternalKey packed trailer + seq-desc Ord"),
        on!("write_record_count", "write_record_count", "WriteRecord count is atomic (no silent prefix)"),
        on!("pin_gc", "pin_gc", "SnapshotPin is oldest_snapshot for point_version_fate"),
        on!("wait_for_deadlock", "wait_for_deadlock", "TransactionDB 2PL wait-for cycle is Deadlock"),
        on!("iter_window", "iter_window", "compat iterator window vs visible_at (RFC-0151 P1)"),
        on!("zero_glue", "zero_glue", "zero remaining glue is not a theorem"),
        on!("lock_interleavings", "lock_interleavings", "lock/OS-scheduler interleavings are not forall"),
        contract!("wal_barrier", "WAL write + fdatasync before Ok (RFC-0001 O1 / RFC-0036) — enforced in code, exercised by the crash/EIO battery"),
        contract!("disk_env", "StdEnv pinned by the verified constructors (Env seam; FailingEnv drives the DST battery)"),
        off!("catchup_window", "pinned to 0 by the verified pin — the merge happens by natural queuing, never by a delay window"),
        off!("async_group_merge", "verified async writes take the write lock themselves (no leader dependency — the pin forces the bypass even under PEDRA_ASYNC_GROUP=1)"),
        off!("io_uring_ring", "no proven ring model (cqe_kernel twin blocked); verified constructors pin StdEnv — the full mode keeps PosixFallback (RFC-0058 P2.2 / RFC-0080)"),
    ]
}

/// Admit a proven io_uring ring model (RFC-0080 / R-uring).
///
/// Always false: there is no probable ring model. Verified constructors
/// pin `StdEnv` / POSIX fallback. AS-IS treats the ring as proven.
#[must_use]
pub fn ring_model_admitted() -> bool {
    false
}

/// AS-IS: a green verified open is rounded to a proven ring (the 0080 hole).
#[must_use]
pub fn ring_model_admitted_as_is() -> bool {
    true
}

/// Admit a live ring backend inside the verified profile.
///
/// Requires both a request for the ring **and** a proven model. Today
/// that is never. AS-IS admits whenever the caller wants the ring.
#[must_use]
pub fn verified_admits_ring(want_ring: bool) -> bool {
    want_ring && ring_model_admitted()
}

/// AS-IS: verified + live ring is fine (WAL back on SQE — the 0080 hole).
#[must_use]
pub fn verified_admits_ring_as_is(want_ring: bool) -> bool {
    want_ring
}

/// RFC-0080 P2.1: a Verus twin of the io_uring ring is not admitted.
/// Always false. RFC-0074 twins `cqe_res_ok` only; no ring model twin.
#[must_use]
pub fn ring_twin_admitted() -> bool {
    false
}

/// AS-IS: the ring looks twin-proven (the 0080 P2.1 hole).
#[must_use]
pub fn ring_twin_admitted_as_is() -> bool {
    true
}

/// RFC-0080 P2.2: production WAL/SST write+sync on SQE submit.
/// Always false. G1 stays POSIX `pwrite` / `fdatasync`.
#[must_use]
pub fn wal_on_sqe_admitted() -> bool {
    false
}

/// AS-IS: WAL is rounded back onto the ring (the 0062 / 0080 hole).
#[must_use]
pub fn wal_on_sqe_admitted_as_is() -> bool {
    true
}

/// The declared composition (RFC-0058 P0.1 + P2.1).
///
/// Use [`Self::open`] / [`Self::open_with_env`] for the whole profile
/// (file options + the verified group pin: merge decided by the proved
/// `group_commit_kernel`, catch-up window 0, async bypass).
/// [`Self::open_options`] is the file-level half alone.
pub struct VerifiedProfile;

impl VerifiedProfile {
    /// File-level composition: `sync=true`, strongest WAL data class,
    /// fail-closed recovery.
    #[must_use]
    pub fn open_options() -> OpenOptions {
        OpenOptions {
            sync: true,
            wal_full_fsync: true,
            wal_recovery: WalRecovery::FailClosed,
            ..OpenOptions::default()
        }
    }

    /// Open on the real filesystem (`StdEnv`) with the full profile:
    /// file options + the verified group pin.
    ///
    /// # Errors
    /// Same as [`ConcurrentDb::open_with`].
    pub fn open(path: impl AsRef<Path>) -> Result<ConcurrentDb<StdEnv>> {
        ConcurrentDb::open_verified(path)
    }

    /// Open with an explicit [`Env`] (DST wraps `FailingEnv` here) and the
    /// full profile.
    ///
    /// # Errors
    /// Same as [`ConcurrentDb::open_with_env`].
    pub fn open_with_env<E: Env>(path: impl AsRef<Path>, env: E) -> Result<ConcurrentDb<E>> {
        let db = ConcurrentDb::open_with_env(path, Self::open_options(), env)?;
        db.pin_verified();
        Ok(db)
    }
}

impl OpenOptions {
    /// File-level composition of the verified profile (RFC-0058 P0.1):
    /// `sync=true`, `wal_full_fsync=true`, `wal_recovery=FailClosed`.
    ///
    /// The group-level half (RFC-0058 P2.1: merge decided by the proved
    /// `group_commit_kernel`, catch-up window 0, async bypass) is a
    /// runtime policy — pin it with
    /// [`ConcurrentDb::pin_verified`](crate::ConcurrentDb::pin_verified)
    /// or open through [`VerifiedProfile::open`] /
    /// [`ConcurrentDb::open_verified`], which do both. The io_uring ring
    /// stays outside the mode (P2.2 gate: no proven ring model — open
    /// with [`StdEnv`](crate::StdEnv), as `PEDRA_VERIFIED=1` does).
    #[must_use]
    pub fn verified() -> Self {
        VerifiedProfile::open_options()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Extract the string values of every `"id":` key inside the `pairs`
    /// array of the catalog JSON (tolerant of whitespace; no JSON
    /// dependency in the core dev graph).
    fn catalog_ids() -> Vec<String> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/formal/catalog.json"
        );
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("catalog.json unreadable ({e}) — run from the repo"));
        let pairs_body = text
            .split("\"pairs\"")
            .nth(1)
            .and_then(|rest| rest.split("\"clones\"").next())
            .expect("catalog.json must have a pairs array before clones");
        let mut ids = Vec::new();
        let mut rest = pairs_body;
        while let Some(pos) = rest.find("\"id\"") {
            rest = &rest[pos + 4..];
            let after_colon = rest.trim_start().strip_prefix(':').unwrap_or(rest);
            let quoted = after_colon
                .trim_start()
                .strip_prefix('"')
                .unwrap_or(after_colon);
            if let Some(end) = quoted.find('"') {
                ids.push(quoted[..end].to_string());
                rest = &quoted[end..];
            }
        }
        ids
    }

    /// RFC-0058 P0.1 invariant: the ON kernels of the report are exactly
    /// the catalog `pairs` ids (well — every `"id"` in the file; clones
    /// and models are supersets that keep the existence check honest).
    /// A new catalog kernel must be claimed here in the same change.
    #[test]
    fn verified_report_matches_catalog() {
        let catalog = catalog_ids();
        // 39 pairs: the pre-088f2f9 count minus the kernels of crates this
        // repo does not ship (088f2f9 dropped the mirrors; the floor follows).
        assert!(catalog.len() >= 39, "catalog shrank? ids: {catalog:?}");
        let reported: std::collections::HashSet<&str> = profile_report()
            .iter()
            .filter(|c| c.state == ProfileState::On)
            .filter_map(|c| c.kernel)
            .collect();
        for id in &catalog {
            assert!(
                reported.contains(id.as_str()),
                "catalog kernel {id} not claimed by the verified report — claim it (On) or gate it (Off) in the same change"
            );
        }
        let catalog_set: std::collections::HashSet<&str> =
            catalog.iter().map(String::as_str).collect();
        for k in &reported {
            assert!(
                catalog_set.contains(k),
                "report cites kernel {k} that is not in the catalog"
            );
        }
        // The mode's differentiators are explicit: the merge is ON with
        // the proved kernel; the delay window, the async leader
        // dependency and the io_uring ring stay OFF.
        let c = profile_report()
            .iter()
            .find(|c| c.component == "write_group_merge")
            .unwrap_or_else(|| panic!("missing report row write_group_merge"));
        assert_eq!(c.state, ProfileState::On, "{c:?}");
        assert_eq!(c.kernel, Some("group_commit"));
        for name in ["io_uring_ring", "catchup_window", "async_group_merge"] {
            let c = profile_report()
                .iter()
                .find(|c| c.component == name)
                .unwrap_or_else(|| panic!("missing report row {name}"));
            assert_eq!(c.state, ProfileState::Off, "{name}: {c:?}");
        }
        let ring = profile_report()
            .iter()
            .find(|c| c.component == "io_uring_ring")
            .unwrap();
        assert_eq!(
            ring.state == ProfileState::On,
            ring_model_admitted(),
            "io_uring_ring On/Off must track ring_model_admitted"
        );
    }

    #[test]
    fn ring_model_is_not_admitted() {
        assert!(!ring_model_admitted());
        assert!(
            ring_model_admitted_as_is(),
            "AS-IS dente: ring looks proven"
        );
        assert!(!verified_admits_ring(true));
        assert!(!verified_admits_ring(false));
        assert!(
            verified_admits_ring_as_is(true),
            "AS-IS dente: verified would take a live ring"
        );
        assert!(!verified_admits_ring_as_is(false));
        assert!(!ring_twin_admitted());
        assert!(
            ring_twin_admitted_as_is(),
            "AS-IS dente: ring twin looks proven"
        );
        assert!(!wal_on_sqe_admitted());
        assert!(wal_on_sqe_admitted_as_is(), "AS-IS dente: WAL back on SQE");
        let twin = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("verus/ring_model.rs");
        assert!(
            !twin.exists(),
            "RFC-0080 P2.1: ring Verus twin must stay absent ({})",
            twin.display()
        );
    }

    /// P0.1: the options half of the composition is fixed and fail-closed.
    #[test]
    fn verified_open_options_are_fixed() {
        let o = OpenOptions::verified();
        assert!(o.sync);
        assert!(o.wal_full_fsync);
        assert_eq!(o.wal_recovery, WalRecovery::FailClosed);
        let p = VerifiedProfile::open_options();
        assert!(p.sync && p.wal_full_fsync);
        assert_eq!(p.wal_recovery, WalRecovery::FailClosed);
    }

    /// Same TX the CLI `demo` runs under `PEDRA_VERIFIED=1` (`StdEnv` +
    /// verified options). Pins the shipped commit path, not a copy.
    #[test]
    fn verified_std_env_demo_tx_roundtrip() {
        use crate::db::Db;
        use crate::StdEnv;
        let dir =
            std::env::temp_dir().join(format!("pedradb-verified-demo-tx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut db = Db::open_with_env(&dir, OpenOptions::verified(), StdEnv).unwrap();
        {
            let mut tx = db.begin();
            tx.put(b"u/1", br#"{"name":"ada"}"#).unwrap();
            tx.put(b"idx/name/ada", b"1").unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(
            db.get(b"u/1").as_deref(),
            Some(br#"{"name":"ada"}"#.as_ref())
        );
        db.close().unwrap();
        let db2 = Db::open_with_env(&dir, OpenOptions::verified(), StdEnv).unwrap();
        assert_eq!(
            db2.get(b"u/1").as_deref(),
            Some(br#"{"name":"ada"}"#.as_ref())
        );
        db2.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
