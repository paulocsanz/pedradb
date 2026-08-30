// Verus proof of the MANIFEST-recovery decision kernel
// (RFC-0056 P0.1 — crash dictionary on the SST-inventory reopen path).
//
// Source of truth for production remains `src/manifest_kernel.rs`
// (called by `recover_ssts` on `Db::open_with_env`). This file is the
// machine-checked theorem: exec == spec, damaged inventory never falls
// back to a silent scan, a committed inventory missing a listed SST
// refuses, absence scans + installs, and F196 tolerates exactly the
// committed-unsynced first install.
//
//   ./scripts/verus_manifest_recover.sh
//
// Do not link this into the production crate — it is a twin of the pure kernel.

use vstd::prelude::*;

verus! {

/// Mirrors `ManifestObs` in manifest_kernel.rs.
pub enum ManifestObs {
    Absent,
    Inventory,
    Corrupt,
}

/// Mirrors `ListedSst` in manifest_kernel.rs.
pub enum ListedSst {
    AllPresent,
    Missing(u64),
}

/// Mirrors `SstRecoverAction` in manifest_kernel.rs.
pub enum SstRecoverAction {
    ServeInventory,
    ScanAndInstall,
    RefuseOpen,
}

/// Closed-form spec — same arms as `manifest_kernel::sst_recover_action`.
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

/// Executable decision — must match `pedradb_core::manifest_kernel::
/// sst_recover_action` bit-for-bit.
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

/// Mirrors `FirstInstallOutcome` in manifest_kernel.rs.
pub enum FirstInstallOutcome {
    Committed,
    CommittedUnsynced,
    Failed,
}

/// Mirrors `FirstInstallAction` in manifest_kernel.rs.
pub enum FirstInstallAction {
    Proceed,
    RefuseOpen,
}

/// Closed-form spec — same arms as `manifest_kernel::first_install_action`.
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

/// P0.1 named lemma (crash spec): a damaged inventory NEVER falls back to a
/// directory scan and never serves — the reopen is refused. A scan would
/// resurrect GC'd / compacted-away files (silent-wrong).
proof fn lemma_damaged_inventory_never_scans(listed: ListedSst)
    ensures
        sst_recover_action(ManifestObs::Corrupt, listed) == SstRecoverAction::RefuseOpen,
{
}

/// P0.1 named lemma (crash spec): the committed inventory is ground truth —
/// a listed SST missing from disk refuses the open instead of silently
/// serving a directory scan that diverges from it.
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
/// to prevent.
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
