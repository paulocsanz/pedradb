//! Pure MANIFEST-recovery decisions (RFC-0056 P0.1 — crash dictionary on
//! the SST-inventory reopen path).
//!
//! **Single artifact (pair `manifest_recover`):** this file is what `rustc`
//! links *and* what Verus proves (`cfg(verus_keep_ghost)`). Pair
//! `first_install` still has a twin-cópia until its turn.
//!
//!   ./scripts/verus_manifest_recover.sh
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
//! The rustc bodies stay byte-stable so non-`single_artifact` twins still
//! token-match. Verus proofs sit in the `cfg(verus_keep_ghost)` block
//! above them (last-wins for lint is the rustc body).
//!
//! Spec page: `docs/formal/crash-dictionary.md` (MANIFEST recovery section).

#![forbid(unsafe_code)]

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

/// Mirrors the rustc `ManifestObs` below (cfg-split so Verus does not see Debug).
pub enum ManifestObs {
    Absent,
    Inventory,
    Corrupt,
}

/// Mirrors the rustc `ListedSst` below.
pub enum ListedSst {
    AllPresent,
    Missing(u64),
}

/// Mirrors the rustc `SstRecoverAction` below.
pub enum SstRecoverAction {
    ServeInventory,
    ScanAndInstall,
    RefuseOpen,
}

/// Closed-form spec — same arms as production `sst_recover_action`.
pub open spec fn sst_recover_spec(obs: ManifestObs, listed: ListedSst) -> SstRecoverAction {
    match (obs, listed) {
        (ManifestObs::Absent, _) => SstRecoverAction::ScanAndInstall,
        (ManifestObs::Inventory, ListedSst::AllPresent) => SstRecoverAction::ServeInventory,
        (ManifestObs::Inventory, ListedSst::Missing(_)) => SstRecoverAction::RefuseOpen,
        (ManifestObs::Corrupt, _) => SstRecoverAction::RefuseOpen,
    }
}

/// AS-IS swallow: damage / missing listed SST ⇒ silent directory scan
/// (resurrects GC'd files, serves an inventory that was never committed).
pub open spec fn sst_recover_as_is(obs: ManifestObs, listed: ListedSst) -> SstRecoverAction {
    if obs is Inventory && listed is AllPresent {
        SstRecoverAction::ServeInventory
    } else {
        SstRecoverAction::ScanAndInstall
    }
}

/// Executable decision — must match the rustc `sst_recover_action` bit-for-bit.
#[verifier::when_used_as_spec(sst_recover_spec)]
pub fn sst_recover_action(obs: ManifestObs, listed: ListedSst) -> (a: SstRecoverAction)
    ensures
        a == sst_recover_spec(obs, listed),
        (a == SstRecoverAction::ServeInventory)
            ==> (obs == ManifestObs::Inventory && listed == ListedSst::AllPresent),
        (a == SstRecoverAction::ScanAndInstall) ==> obs == ManifestObs::Absent,
{
    match (obs, listed) {
        (ManifestObs::Absent, _) => SstRecoverAction::ScanAndInstall,
        (ManifestObs::Inventory, ListedSst::AllPresent) => SstRecoverAction::ServeInventory,
        (ManifestObs::Inventory, ListedSst::Missing(_)) => SstRecoverAction::RefuseOpen,
        (ManifestObs::Corrupt, _) => SstRecoverAction::RefuseOpen,
    }
}

/// Mirrors the rustc `FirstInstallOutcome` below.
pub enum FirstInstallOutcome {
    Committed,
    CommittedUnsynced,
    Failed,
}

/// Mirrors the rustc `FirstInstallAction` below.
pub enum FirstInstallAction {
    Proceed,
    RefuseOpen,
}

/// Closed-form spec — same arms as production `first_install_action`.
pub open spec fn first_install_spec(out: FirstInstallOutcome) -> FirstInstallAction {
    match out {
        FirstInstallOutcome::Committed => FirstInstallAction::Proceed,
        FirstInstallOutcome::CommittedUnsynced => FirstInstallAction::Proceed,
        FirstInstallOutcome::Failed => FirstInstallAction::RefuseOpen,
    }
}

/// AS-IS swallow: proceed no matter how the first install ended.
pub open spec fn first_install_as_is(_out: FirstInstallOutcome) -> FirstInstallAction {
    FirstInstallAction::Proceed
}

#[verifier::when_used_as_spec(first_install_spec)]
pub fn first_install_action(out: FirstInstallOutcome) -> (a: FirstInstallAction)
    ensures
        a == first_install_spec(out),
        (out == FirstInstallOutcome::CommittedUnsynced) ==> a == FirstInstallAction::Proceed,
        (out == FirstInstallOutcome::Failed) ==> a == FirstInstallAction::RefuseOpen,
{
    match out {
        FirstInstallOutcome::Committed => FirstInstallAction::Proceed,
        FirstInstallOutcome::CommittedUnsynced => FirstInstallAction::Proceed,
        FirstInstallOutcome::Failed => FirstInstallAction::RefuseOpen,
    }
}

/// P0.1 named lemma (crash spec / Memgraph #3785): a damaged inventory NEVER
/// falls back to a directory scan and never serves — the reopen is refused.
/// A scan would resurrect GC'd / compacted-away files (silent-wrong).
proof fn lemma_damaged_inventory_never_scans(listed: ListedSst)
    ensures
        sst_recover_action(ManifestObs::Corrupt, listed) == SstRecoverAction::RefuseOpen,
{
}

/// P0.1 named lemma (crash spec / RocksDB #13624 VersionBuilder Corruption):
/// the committed inventory is ground truth — a listed SST missing from disk
/// refuses the open instead of silently serving a directory scan that
/// diverges from it.
proof fn lemma_missing_listed_sst_refuses(num: u64)
    ensures
        sst_recover_action(ManifestObs::Inventory, ListedSst::Missing(num))
            == SstRecoverAction::RefuseOpen,
{
}

/// P0.1 named lemma (no false refusal): absent inventory (CURRENT missing
/// or torn-empty) is a first open / legacy dir — scan + install proceeds.
proof fn lemma_absent_inventory_scans(listed: ListedSst)
    ensures
        sst_recover_action(ManifestObs::Absent, listed) == SstRecoverAction::ScanAndInstall,
{
}

/// P0.1 named lemma (liveness of the healthy path): a decoded inventory
/// whose listed files are all present is served as-is.
proof fn lemma_committed_inventory_serves()
    ensures
        sst_recover_action(ManifestObs::Inventory, ListedSst::AllPresent)
            == SstRecoverAction::ServeInventory,
{
}

/// P0.1 named lemma (F196): the committed-unsynced first install is
/// tolerated — during the first open the installed inventory IS committed
/// (nothing acked is at risk yet).
proof fn lemma_first_install_unsynced_tolerated()
    ensures
        first_install_action(FirstInstallOutcome::CommittedUnsynced)
            == FirstInstallAction::Proceed,
        first_install_action(FirstInstallOutcome::Failed)
            == FirstInstallAction::RefuseOpen,
{
}

/// Teeth: the AS-IS swallow mutant answers a silent directory scan exactly
/// where the fixed kernel refuses — the resurrection bug the kernel exists
/// to prevent (Memgraph #3785 class: torn MANIFEST must not become a scan).
proof fn lemma_mutant_scan_resurrects_gc(listed: ListedSst)
    requires
        listed == ListedSst::AllPresent || listed is Missing,
    ensures
        sst_recover_action(ManifestObs::Corrupt, listed) == SstRecoverAction::RefuseOpen,
        sst_recover_as_is(ManifestObs::Corrupt, listed) == SstRecoverAction::ScanAndInstall,
{
}

/// Teeth: the AS-IS swallow mutant serves the scanned inventory after a
/// FAILED first install — serving an inventory that was never persisted.
proof fn lemma_mutant_first_install_serve_failed()
    ensures
        first_install_action(FirstInstallOutcome::Failed) == FirstInstallAction::RefuseOpen,
        first_install_as_is(FirstInstallOutcome::Failed) == FirstInstallAction::Proceed,
{
}

} // verus!

/// What `manifest::load` observed for the SST inventory.
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListedSst {
    /// Every listed `{num:06}.sst` exists.
    AllPresent,
    /// The inventory lists `{num:06}.sst` but the file is gone.
    Missing(u64),
}

/// What the reopen does with the SST inventory.
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn first_install_action_as_is_proceed_always(_out: FirstInstallOutcome) -> FirstInstallAction {
    FirstInstallAction::Proceed
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "AS-IS dente: missing SST silently scanned"
        );
    }
}
