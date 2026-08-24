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
//! [`ConcurrentDb::pin_verified`]): **lone-commit-only** — every commit is
//! a single-writer critical section; the leader/member merge stays off
//! until the group-commit kernel lands (RFC-0057 P2.1 / RFC-0058 P2.1).
//! Product constructors pin `StdEnv` — the `io_uring` ring is out of the
//! mode (P2.2); the full mode keeps its `PosixFallback`.
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
pub const PROFILE_VERSION: &str = "verified-v1";

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
        // --- recovery (fail-closed) ---
        on!("wal_recover", "wal_recover", "fail-closed prefix recovery at open (F4/F14)"),
        on!("manifest_recover", "manifest_recover", "MANIFEST recovery (F196/G8)"),
        on!("reopen_outcome", "reopen_outcome", "reopen state equals the pre-crash visible state (F170/F171/G8)"),
        on!("dictionary_link", "dictionary_link", "crash-dictionary put→get link across flush/crash (G1/G8)"),
        on!("vlog_recover", "vlog_recover", "value-log recovery (F51)"),
        on!("blob_gc_pick", "blob_gc_pick", "blob GC picks the active generation"),
        // --- commit path (single-writer critical section) ---
        on!("apply_step", "apply_step", "batch apply under the single write lock — the verified commit critical section (F10-apply)"),
        on!("grant_persist", "grant_persist", "durability grant persistence (F15)"),
        on!("txn", "txn", "multi-key TX all-or-nothing (F47/F34)"),
        on!("tx_glue", "tx_glue", "OCC glue — validation under the write lock keeps first-committer-wins lone (F47/F34)"),
        on!("isolated", "isolated", "isolated apply (F83)"),
        // --- background decisions ---
        on!("flush_decision", "flush_decision", "when to flush (F2/F43/G1)"),
        on!("compact_decision", "compact_decision", "when to compact (F177/F20)"),
        on!("compact_retention", "compact_retention", "what compaction retains (F177/F20)"),
        on!("compact", "compact", "merge iterator correctness (F27/F28)"),
        // --- reads ---
        on!("snapshot", "snapshot", "snapshot reads (F38/F40/F41)"),
        on!("si_reader", "si_reader", "secondary-index reader construction (F42/F84)"),
        on!("si_read", "si_read", "secondary-index read (F168)"),
        on!("index_val", "index_val", "index entry validation (F80)"),
        on!("fail_closed", "fail_closed", "read APIs fail closed — corruption is never served as a miss (F102/F104/F105)"),
        on!("changelog", "changelog", "changelog decode/replay (F53)"),
        on!("range_covers", "range_covers", "range bounds cover the requested span (F30)"),
        on!("prefix", "prefix", "prefix seek bounds (F57/F58)"),
        on!("stream_cursor", "stream_cursor", "stream cursor resume (F54)"),
        on!("scan_guard", "scan_guard", "scan lifetime guard (F167)"),
        on!("journal_pin", "journal_pin", "pin-aware journal reclaim (H1/F54)"),
        on!("bloom_header", "bloom_header", "bloom header validation (T1)"),
        on!("bloom_insert", "bloom_insert", "bloom insert (T1)"),
        on!("bloom_may_contain", "bloom_may_contain", "bloom probe — no false negatives (T1/T4)"),
        on!("fold_range", "fold_range", "fold Storage range consistency (F169)"),
        // --- montanha (world nodes) ---
        on!("vote", "vote", "raft vote handling (F15)"),
        on!("ae_entry", "ae_entry", "append-entries entry validation (F16)"),
        on!("ae_ack", "ae_ack", "append-entries ack (F48)"),
        on!("commit_raft", "commit_raft", "raft commit advancement (F10/F11/F23)"),
        on!("lease", "lease", "lease grant/expiry persistence (F7/F56)"),
        on!("dcs_apply", "dcs_apply", "DCS command application (F12/F22)"),
        // --- ship / product surface ---
        on!("pack", "pack", "pack encoding validation (F62)"),
        on!("ship_guard", "ship_guard", "ship guard (F165)"),
        on!("bearer", "bearer", "bearer auth (F85/F79)"),
        on!("content_length", "content_length", "content-length handling (F86/F87/F88)"),
        on!("form_plus", "form_plus", "form parsing (F101)"),
        on!("origin_path", "origin_path", "origin path resolution (F91/F92)"),
        on!("children", "children", "node children validation (F59)"),
        on!("fields", "fields", "field validation (F60)"),
        // --- contracts without a theorem (published, DST-exercised) ---
        contract!("wal_barrier", "WAL write + fdatasync before Ok (RFC-0001 O1 / RFC-0036) — enforced in code, exercised by the crash/EIO battery"),
        contract!("disk_env", "StdEnv pinned by the verified constructors (Env seam; FailingEnv drives the DST battery)"),
        // --- deliberately off ---
        off!("write_group_merge", "lone-commit-only: every commit is a single-writer critical section (ConcurrentDb::pin_verified); the merge returns with the RFC-0057 P2.1 kernel (RFC-0058 P2.1)"),
        off!("catchup_window", "moot while the merge is off — the pin forces it to 0"),
        off!("async_group_merge", "verified async writes take the write lock themselves (no leader dependency, the un-merged shape)"),
        off!("io_uring_ring", "no proven ring model (cqe_kernel twin blocked); verified constructors pin StdEnv — the full mode keeps PosixFallback (RFC-0058 P2.2)"),
    ]
}

/// The declared composition (RFC-0058 P0.1).
///
/// Use [`Self::open`] / [`Self::open_with_env`] for the whole profile
/// (file options + lone-commit-only group). [`Self::open_options`] is the
/// file-level half alone.
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
    /// file options + lone-commit-only group.
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
    /// The group-level half (lone-commit-only) is a runtime policy — pin
    /// it with [`ConcurrentDb::pin_verified`](crate::ConcurrentDb::pin_verified)
    /// or open through [`VerifiedProfile::open`] /
    /// [`ConcurrentDb::open_verified`], which do both.
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
            let quoted = after_colon.trim_start().strip_prefix('"').unwrap_or(after_colon);
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
        assert!(
            catalog.len() >= 44,
            "catalog shrank? ids: {catalog:?}"
        );
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
        // The mode's differentiators are explicit.
        for name in ["write_group_merge", "io_uring_ring", "catchup_window"] {
            let c = profile_report()
                .iter()
                .find(|c| c.component == name)
                .unwrap_or_else(|| panic!("missing report row {name}"));
            assert_eq!(c.state, ProfileState::Off, "{name}: {c:?}");
        }
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
}
