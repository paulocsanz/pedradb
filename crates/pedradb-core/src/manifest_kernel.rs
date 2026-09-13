//! Pure MANIFEST-recovery decisions (RFC-0056 P0.1 — crash dictionary on
//! the SST-inventory reopen path).
//!
//! **Term:** this file is what `rustc` links. Aeneas extracts that body
//! (`scripts/aeneas_manifest.sh`). A Verus stand-in of ManifestObs billed as
//! last-wins of a cfg-split file is a model twin (deleted).
//!
//!   ./scripts/aeneas_manifest.sh --required
//!
//! Production `recover_ssts` (called by [`crate::Db::open_with_env`])
//! routes every decision through this kernel: given what `manifest::load`
//! observed (absent inventory / committed inventory / damaged
//! CURRENT-or-MANIFEST) and whether every listed SST file is on disk, the
//! reopen action is decided **here** — never ad hoc at the call site.
//! Bytes, decode, orphan GC, install and fsync are **caller + axiom**
//! (DST / `FailingEnv` drive those).
//!
//! Named decisions (the ones that were `SilentWrong` when inverted):
//! - **Damaged inventory ⇒ refuse** — bad CURRENT contents, dangling
//!   CURRENT, unsupported version, undecodable MANIFEST: never fall back
//!   to a directory scan silently (a scan resurrects GC'd / compacted-away
//!   files and serves an inventory that was never committed).
//! - **Committed inventory listing a missing SST ⇒ refuse** — the
//!   inventory is ground truth; a scan would silently diverge from it.
//! - **Absent inventory (CURRENT missing or torn-empty) ⇒ scan + install**
//!   — first open / legacy dir; nothing acked depends on a committed
//!   inventory, so the scan IS the inventory.
//! - **F196: first install committed-unsynced ⇒ proceed** — during the
//!   first open the installed inventory IS committed (nothing acked is at
//!   risk yet); every other install failure refuses.
//!
//! Aeneas of the rustc body is the term. A Verus stand-in is not last-wins.
//!
//! Spec page: `docs/formal/crash-dictionary.md` (MANIFEST recovery section).

#![forbid(unsafe_code)]

/// What `manifest::load` observed for the SST inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ManifestObs {
    /// CURRENT absent, or torn-empty (unsynced crash) → no committed
    /// inventory exists yet.
    Absent,
    /// CURRENT → MANIFEST decoded: a committed inventory exists.
    Inventory,
    /// Damaged: bad CURRENT contents, CURRENT pointing at a missing
    /// MANIFEST, unsupported version, or undecodable payload.
    Corrupt,
}

/// Do all SST files the committed inventory lists exist on disk?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListedSst {
    /// Every listed `{num:06}.sst` exists.
    AllPresent,
    /// The inventory lists `{num:06}.sst` but the file is gone.
    Missing(u64),
}

/// What the reopen does with the SST inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SstRecoverAction {
    /// Serve the committed inventory (MANIFEST is ground truth).
    ServeInventory,
    /// Absent inventory: scan the directory, then install the first
    /// MANIFEST (F196 tolerates a committed-unsynced first install).
    ScanAndInstall,
    /// Refuse the open (fail closed).
    RefuseOpen,
}

/// Pure rule for one SST-inventory reopen (RFC-0056 P0.1).
///
/// # Post-condition (theorem-ready)
///
/// ```text
/// ensures
///   (action == ServeInventory)  ==> obs == Inventory && listed == AllPresent
///   (action == ScanAndInstall)  ==> obs == Absent
///   obs == Corrupt              ==> action == RefuseOpen   // never scan
///   obs == Inventory && listed is Missing ==> action == RefuseOpen
/// ```
///
/// Finite-domain check: [`tests::theorem_sst_recover_on_finite_domain`].
#[must_use]
pub fn sst_recover_action(obs: ManifestObs, listed: ListedSst) -> SstRecoverAction {
    match (obs, listed) {
        (ManifestObs::Absent, _) => SstRecoverAction::ScanAndInstall,
        (ManifestObs::Inventory, ListedSst::AllPresent) => SstRecoverAction::ServeInventory,
        (ManifestObs::Inventory, ListedSst::Missing(_)) | (ManifestObs::Corrupt, _) => {
            SstRecoverAction::RefuseOpen
        }
    }
}

/// AS-IS swallow: damage or missing listed SST ⇒ silently fall back to a
/// directory scan — the scan resurrects GC'd / compacted-away files and
/// serves an inventory that was never committed (silent-wrong). Mutant
/// must fail every theorem above.
#[must_use]
pub fn sst_recover_action_as_is_scan_on_damage(
    obs: ManifestObs,
    listed: ListedSst,
) -> SstRecoverAction {
    match obs {
        ManifestObs::Inventory if listed == ListedSst::AllPresent => {
            SstRecoverAction::ServeInventory
        }
        _ => SstRecoverAction::ScanAndInstall,
    }
}

/// How the first MANIFEST install (on the `ScanAndInstall` path) ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FirstInstallOutcome {
    /// MANIFEST written + CURRENT swung (synced or not — both fine).
    Committed,
    /// `Err(ManifestCommittedUnsynced)`: the swing happened but the sync
    /// after it failed (F196).
    CommittedUnsynced,
    /// Any other install failure (I/O before the swing, encode failure…).
    Failed,
}

/// What the first open does after installing the initial inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FirstInstallAction {
    /// Serve the scanned inventory.
    Proceed,
    /// Refuse the open.
    RefuseOpen,
}

/// Pure rule for the first-install outcome (F196).
///
/// # Post-condition (theorem-ready)
///
/// ```text
/// ensures
///   out == CommittedUnsynced ==> action == Proceed   // F196: first open,
///     nothing acked is at risk; the inventory IS committed
///   out == Failed             ==> action == RefuseOpen
/// ```
#[must_use]
pub fn first_install_action(out: FirstInstallOutcome) -> FirstInstallAction {
    match out {
        FirstInstallOutcome::Committed | FirstInstallOutcome::CommittedUnsynced => {
            FirstInstallAction::Proceed
        }
        FirstInstallOutcome::Failed => FirstInstallAction::RefuseOpen,
    }
}

/// AS-IS swallow: proceed no matter how the first install ended — serves
/// an inventory that was never persisted (silent-wrong on `Failed`).
#[must_use]
pub fn first_install_action_as_is_proceed_always(_out: FirstInstallOutcome) -> FirstInstallAction {
    FirstInstallAction::Proceed
}

/// Fate of one bulk-run MANIFEST persist (RFC-0219 P0.3). The trampoline
/// `db.rs::persist_bulk_manifest` owns no fate: it matches this plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BulkManifestFate {
    /// Sync mode (`dir_sync_required`): fsync the unsynced SSTs and
    /// publish the MANIFEST inline before returning — the publish window
    /// is closed under the caller's barrier; no debt accrues.
    PersistNow,
    /// Async mode: amortize — clear the unsynced set, accrue
    /// `bulk_manifest_debt`, and hand back a deferred `ManifestPersist`
    /// every `BULK_MANIFEST_EVERY` installs (`force` settles leftover).
    AmortizeDebt,
}

/// Pure rule: the DB sync default alone decides how a bulk install's
/// MANIFEST publish is paid (inline durable vs amortized debt).
///
/// # Post-condition (theorem-ready)
///
/// ```text
/// ensures
///   action == PersistNow  <=> sync == true
///   action == AmortizeDebt <=> sync == false
/// ```
///
/// Finite-domain check: [`tests::bulk_manifest_fate_on_finite_domain`].
#[must_use]
pub fn bulk_manifest_persist_fate(sync: bool) -> BulkManifestFate {
    if sync {
        BulkManifestFate::PersistNow
    } else {
        BulkManifestFate::AmortizeDebt
    }
}

/// AS-IS swallow: amortize forever — even in sync mode the MANIFEST
/// publish window stays open across installs, so a crash reopens an
/// inventory that predates acked installs (silent-wrong in sync mode).
/// Mutant must fail every theorem above.
#[must_use]
pub fn bulk_manifest_persist_fate_as_is(_sync: bool) -> BulkManifestFate {
    BulkManifestFate::AmortizeDebt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_kernel_has_no_verus_cartoon() {
        let src = include_str!("manifest_kernel.rs");
        let block = concat!("verus", "!", " {");
        let cfg = concat!("cfg(", "verus", "_keep", "_ghost)");
        assert!(
            !src.contains(block),
            "stand-in is not last-wins of rustc sst_recover_action"
        );
        assert!(
            !src.contains(cfg),
            "cfg split hides rustc types from the prover"
        );
    }

    #[test]
    fn absent_inventory_scans_and_installs() {
        assert_eq!(
            sst_recover_action(ManifestObs::Absent, ListedSst::AllPresent),
            SstRecoverAction::ScanAndInstall
        );
        // Listed is meaningless without an inventory.
        assert_eq!(
            sst_recover_action(ManifestObs::Absent, ListedSst::Missing(7)),
            SstRecoverAction::ScanAndInstall
        );
    }

    #[test]
    fn committed_inventory_serves_only_when_listed_files_exist() {
        assert_eq!(
            sst_recover_action(ManifestObs::Inventory, ListedSst::AllPresent),
            SstRecoverAction::ServeInventory
        );
        assert_eq!(
            sst_recover_action(ManifestObs::Inventory, ListedSst::Missing(3)),
            SstRecoverAction::RefuseOpen
        );
    }

    #[test]
    fn damaged_inventory_refuses() {
        assert_eq!(
            sst_recover_action(ManifestObs::Corrupt, ListedSst::AllPresent),
            SstRecoverAction::RefuseOpen
        );
        assert_eq!(
            sst_recover_action(ManifestObs::Corrupt, ListedSst::Missing(3)),
            SstRecoverAction::RefuseOpen
        );
    }

    #[test]
    fn f196_first_install_tolerance() {
        assert_eq!(
            first_install_action(FirstInstallOutcome::Committed),
            FirstInstallAction::Proceed
        );
        assert_eq!(
            first_install_action(FirstInstallOutcome::CommittedUnsynced),
            FirstInstallAction::Proceed,
            "F196: committed-unsynced during first open = the inventory IS committed"
        );
        assert_eq!(
            first_install_action(FirstInstallOutcome::Failed),
            FirstInstallAction::RefuseOpen
        );
    }

    /// Finite-domain theorem: damage / missing-listed-SST never scans or
    /// serves silently; absence always scans; the AS-IS mutant swallows
    /// exactly the damage the fixed kernel refuses.
    #[test]
    fn theorem_sst_recover_on_finite_domain() {
        let listed = [ListedSst::AllPresent, ListedSst::Missing(3)];
        for obs in [
            ManifestObs::Absent,
            ManifestObs::Inventory,
            ManifestObs::Corrupt,
        ] {
            for l in listed {
                let a = sst_recover_action(obs, l);
                match a {
                    SstRecoverAction::ServeInventory => {
                        assert_eq!((obs, l), (ManifestObs::Inventory, ListedSst::AllPresent));
                    }
                    SstRecoverAction::ScanAndInstall => {
                        assert_eq!(obs, ManifestObs::Absent, "scan only when absent");
                    }
                    SstRecoverAction::RefuseOpen => {}
                }
                let damaged = obs == ManifestObs::Corrupt
                    || (obs == ManifestObs::Inventory && l != ListedSst::AllPresent);
                if damaged {
                    assert_eq!(a, SstRecoverAction::RefuseOpen, "damage refuses");
                    let m = sst_recover_action_as_is_scan_on_damage(obs, l);
                    assert_eq!(
                        m,
                        SstRecoverAction::ScanAndInstall,
                        "AS-IS must swallow damage into a silent scan"
                    );
                    assert_ne!(m, a, "mutant must differ from fixed on damage");
                }
            }
        }
        for out in [
            FirstInstallOutcome::Committed,
            FirstInstallOutcome::CommittedUnsynced,
            FirstInstallOutcome::Failed,
        ] {
            let a = first_install_action(out);
            assert_eq!(
                a == FirstInstallAction::Proceed,
                out != FirstInstallOutcome::Failed,
                "only Failed refuses the first open"
            );
            if out == FirstInstallOutcome::Failed {
                assert_eq!(
                    first_install_action_as_is_proceed_always(out),
                    FirstInstallAction::Proceed,
                    "AS-IS must swallow the failed first install"
                );
                assert_ne!(
                    first_install_action_as_is_proceed_always(out),
                    a,
                    "mutant must differ from fixed on Failed"
                );
            }
        }
    }

    #[test]
    fn sst_recover_action_on_live_missing_sst_is_not_ok() {
        assert_eq!(
            sst_recover_action(ManifestObs::Inventory, ListedSst::Missing(1)),
            SstRecoverAction::RefuseOpen
        );
        assert_eq!(
            sst_recover_action_as_is_scan_on_damage(ManifestObs::Inventory, ListedSst::Missing(1)),
            SstRecoverAction::ScanAndInstall,
            "AS-IS tooth: missing SST silently scanned"
        );
    }

    /// Balanced-brace slice of one `fn` from a source file (plant lens).
    fn named_fn_src(src: &str, name: &str) -> Option<String> {
        let needle = format!("fn {name}(");
        let start = src.find(&needle)?;
        let rest = &src[start..];
        let bytes = rest.as_bytes();
        let brace = bytes.iter().position(|&b| b == b'{')?;
        let mut depth = 0i32;
        for (i, &b) in bytes[brace..].iter().enumerate() {
            if b == b'{' {
                depth += 1;
            } else if b == b'}' {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[brace..=brace + i].to_string());
                }
            }
        }
        None
    }

    /// Finite-domain theorem: sync pays inline, async amortizes, and the
    /// AS-IS mutant swallows exactly the sync-mode publish window.
    #[test]
    fn bulk_manifest_fate_on_finite_domain() {
        assert_eq!(
            bulk_manifest_persist_fate(true),
            BulkManifestFate::PersistNow
        );
        assert_eq!(
            bulk_manifest_persist_fate(false),
            BulkManifestFate::AmortizeDebt
        );
        assert_eq!(
            bulk_manifest_persist_fate_as_is(true),
            BulkManifestFate::AmortizeDebt,
            "AS-IS tooth: sync mode amortizes forever (publish window open)"
        );
    }

    #[test]
    fn bulk_manifest_persist_fate_on_live_sync_persists_now() {
        // RFC-0219 P0.3: the DB sync default alone picks inline-durable
        // vs amortized; persist_bulk_manifest matches the kernel plan and
        // keeps no inline dir-sync gate of its own.
        let pbm = named_fn_src(include_str!("db.rs"), "persist_bulk_manifest")
            .expect("persist_bulk_manifest");
        assert!(
            pbm.contains("match crate::manifest_kernel::bulk_manifest_persist_fate("),
            "persist_bulk_manifest must match bulk_manifest_persist_fate"
        );
        assert!(
            pbm.contains("BulkManifestFate::PersistNow =>"),
            "the inline durable publish must live in the PersistNow arm"
        );
        assert!(
            !pbm.contains("write_admission_kernel::dir_sync_required(self.sync)"),
            "the dir-sync gate left the trampoline (write-admission family keeps its own calls)"
        );
    }
}
