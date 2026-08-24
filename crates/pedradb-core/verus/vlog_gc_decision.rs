// Verus proof of the vlog GC decision kernel
// (RFC-0056 P1.4 — crash dictionary on the value-log MANIFEST swing and the
// sealed-blob rewrite guard).
//
// Source of truth for production remains `src/vlog_gc_kernel.rs` (called by
// `open_with_env` on the reopen path and by `compact_blob_auto`). This file
// is the machine-checked theorem: a committed swing resolves to `.new`
// (never the stale primary), an orphan `.new` is ignored, F51 refuses to
// invent an empty primary under a committed swing, and the blob rewrite
// guard never touches the active append generation.
//
//   ./scripts/verus_vlog_gc.sh
//
// Do not link this into the production crate — it is a twin of the pure kernel.

use vstd::prelude::*;

verus! {

/// Mirrors `VlogRecoverAction` in vlog_gc_kernel.rs.
pub enum VlogRecoverAction {
    OpenBlob,
    OpenNew,
    OpenPrimary,
    RefuseOpen,
    CreateEmptyPrimary,
    NoVlog,
}

/// Closed-form spec — same arms as `vlog_gc_kernel::vlog_recover_action`.
pub open spec fn vlog_recover_spec(
    blob_active: bool,
    wants_large: bool,
    primary_exists: bool,
    use_new: bool,
    new_exists: bool,
) -> VlogRecoverAction {
    if blob_active {
        VlogRecoverAction::OpenBlob
    } else if wants_large || primary_exists || (use_new && new_exists) {
        if use_new && new_exists {
            VlogRecoverAction::OpenNew
        } else if primary_exists {
            VlogRecoverAction::OpenPrimary
        } else if use_new {
            VlogRecoverAction::RefuseOpen
        } else {
            VlogRecoverAction::CreateEmptyPrimary
        }
    } else {
        VlogRecoverAction::NoVlog
    }
}

/// AS-IS ignore-swing: always resolves as if the MANIFEST flag were false —
/// the stale pre-GC primary is served after a crash mid-GC.
pub open spec fn vlog_recover_as_is(
    blob_active: bool,
    wants_large: bool,
    primary_exists: bool,
    use_new: bool,
    new_exists: bool,
) -> VlogRecoverAction {
    vlog_recover_spec(blob_active, wants_large, primary_exists, false, false)
}

/// Executable decision — must match `pedra_core::vlog_gc_kernel::
/// vlog_recover_action` bit-for-bit.
#[verifier::when_used_as_spec(vlog_recover_spec)]
pub fn vlog_recover_action(
    blob_active: bool,
    wants_large: bool,
    primary_exists: bool,
    use_new: bool,
    new_exists: bool,
) -> (a: VlogRecoverAction)
    ensures
        a == vlog_recover_spec(blob_active, wants_large, primary_exists, use_new, new_exists),
        // Committed swing, staged `.new`: the swing target is the only resolution.
        use_new && new_exists && !blob_active ==> a == VlogRecoverAction::OpenNew,
        // Orphan `.new` (no committed swing) can never hijack reads.
        !use_new ==> a != VlogRecoverAction::OpenNew,
        // F51: never invent an empty primary when the committed SST inventory
        // points into a `.new` that is gone.
        use_new && !primary_exists && !new_exists && wants_large && !blob_active
            ==> a == VlogRecoverAction::RefuseOpen,
        // Fresh DB / first large put only happens without a swing.
        a == VlogRecoverAction::CreateEmptyPrimary ==> !use_new,
{
    if blob_active {
        VlogRecoverAction::OpenBlob
    } else if wants_large || primary_exists || (use_new && new_exists) {
        if use_new && new_exists {
            VlogRecoverAction::OpenNew
        } else if primary_exists {
            VlogRecoverAction::OpenPrimary
        } else if use_new {
            VlogRecoverAction::RefuseOpen
        } else {
            VlogRecoverAction::CreateEmptyPrimary
        }
    } else {
        VlogRecoverAction::NoVlog
    }
}

/// Mirrors `BlobGcAction` in vlog_gc_kernel.rs.
pub enum BlobGcAction {
    Rewrite,
    Skip,
}

/// Closed-form spec — same arms as `vlog_gc_kernel::blob_gc_action`.
pub open spec fn blob_gc_spec(is_active: bool, bytes: u64) -> BlobGcAction {
    if !is_active && bytes > 0 {
        BlobGcAction::Rewrite
    } else {
        BlobGcAction::Skip
    }
}

/// AS-IS rewrite-active: any non-empty file is eligible, including the
/// active append generation.
pub open spec fn blob_gc_as_is(is_active: bool, bytes: u64) -> BlobGcAction {
    if bytes > 0 {
        BlobGcAction::Rewrite
    } else {
        BlobGcAction::Skip
    }
}

/// Executable guard — must match `pedra_core::vlog_gc_kernel::blob_gc_action`
/// bit-for-bit.
#[verifier::when_used_as_spec(blob_gc_spec)]
pub fn blob_gc_action(is_active: bool, bytes: u64) -> (a: BlobGcAction)
    ensures
        a == blob_gc_spec(is_active, bytes),
        // Active append generation: never rewritten, whatever its size.
        is_active ==> a == BlobGcAction::Skip,
        // Empty file: never rewritten.
        bytes == 0 ==> a == BlobGcAction::Skip,
{
    if !is_active && bytes > 0 {
        BlobGcAction::Rewrite
    } else {
        BlobGcAction::Skip
    }
}

// ---------------------------------------------------------------------------
// Named lemmas (crash dictionary — RFC-0056 P1.4)
// ---------------------------------------------------------------------------

/// P1.4 named lemma (crash spec): crash mid-GC after the MANIFEST committed
/// the swing (`.new` staged, primary still on disk) — the reopen resolves to
/// `.new`, so the remapped SST pointers stay readable.
proof fn lemma_swing_opens_staged_new(wants_large: bool)
    ensures
        vlog_recover_action(false, wants_large, true, true, true)
            == VlogRecoverAction::OpenNew,
        vlog_recover_action(false, wants_large, false, true, true)
            == VlogRecoverAction::OpenNew,
{
}

/// P1.4 named lemma (crash spec): crash BEFORE the MANIFEST commit leaves an
/// orphan `.new`; the committed inventory still points at primary, and the
/// orphan is never opened.
proof fn lemma_orphan_new_never_opened(primary_exists: bool, new_exists: bool)
    ensures
        !primary_exists || vlog_recover_action(false, true, primary_exists, false, new_exists)
            == VlogRecoverAction::OpenPrimary,
        vlog_recover_action(false, true, primary_exists, false, new_exists)
            != VlogRecoverAction::OpenNew,
{
}

/// P1.4 named lemma (crash spec): crash after the promote rename but before
/// the MANIFEST flag clear reconciles to primary — `.new` is gone, primary
/// is the committed content.
proof fn lemma_promote_done_reconciles_primary()
    ensures
        vlog_recover_action(false, true, true, true, false)
            == VlogRecoverAction::OpenPrimary,
{
}

/// P1.4 named lemma (F51): committed swing with both files gone refuses to
/// open — inventing an empty primary would make every large value vanish.
proof fn lemma_f51_refuses_both_missing()
    ensures
        vlog_recover_action(false, true, false, true, false)
            == VlogRecoverAction::RefuseOpen,
        vlog_recover_action(false, true, false, true, false)
            != VlogRecoverAction::CreateEmptyPrimary,
{
}

/// P1.4 named lemma (no false refusal): a fresh DB with no vlog anywhere and
/// no swing creates the empty primary on the first large put.
proof fn lemma_fresh_db_creates_empty_primary()
    ensures
        vlog_recover_action(false, true, false, false, false)
            == VlogRecoverAction::CreateEmptyPrimary,
        vlog_recover_action(false, false, false, false, false)
            == VlogRecoverAction::NoVlog,
{
}

/// P1.4 named lemma: blob mode wins over every other flag combination.
proof fn lemma_blob_mode_wins(wants_large: bool, primary_exists: bool, use_new: bool)
    ensures
        vlog_recover_action(true, wants_large, primary_exists, use_new, true)
            == VlogRecoverAction::OpenBlob,
        vlog_recover_action(true, wants_large, primary_exists, use_new, false)
            == VlogRecoverAction::OpenBlob,
{
}

/// Teeth: the ignore-swing AS-IS mutant serves the stale pre-GC primary
/// exactly where the fixed kernel opens the staged `.new` — the silent
/// resurrect/vanish bug the kernel exists to prevent.
proof fn lemma_mutant_serves_stale_primary_after_swing()
    ensures
        vlog_recover_as_is(false, true, true, true, true) == VlogRecoverAction::OpenPrimary,
        vlog_recover_action(false, true, true, true, true) == VlogRecoverAction::OpenNew,
        vlog_recover_as_is(false, true, true, true, true)
            != vlog_recover_action(false, true, true, true, true),
{
}

/// Teeth: the ignore-swing AS-IS mutant invents an empty primary exactly on
/// the F51 input (committed swing, both files gone) where the fixed kernel
/// refuses — every large value vanishes.
proof fn lemma_mutant_invents_empty_primary_on_f51()
    ensures
        vlog_recover_as_is(false, true, false, true, false)
            == VlogRecoverAction::CreateEmptyPrimary,
        vlog_recover_action(false, true, false, true, false)
            == VlogRecoverAction::RefuseOpen,
        vlog_recover_as_is(false, true, false, true, false)
            != vlog_recover_action(false, true, false, true, false),
{
}

/// P1.4 named lemma: the active append generation is never rewritten —
/// concurrent appends into a sealed-active file would be lost.
proof fn lemma_active_generation_never_rewritten(bytes: u64)
    ensures
        blob_gc_action(true, bytes) == BlobGcAction::Skip,
{
}

/// P1.4 named lemma: empty files are never rewritten.
proof fn lemma_empty_file_never_rewritten(is_active: bool)
    ensures
        blob_gc_action(is_active, 0) == BlobGcAction::Skip,
{
}

/// Teeth: the rewrite-active AS-IS mutant rewrites a non-empty active
/// generation exactly where the fixed guard skips it.
proof fn lemma_mutant_rewrites_active_generation()
    ensures
        blob_gc_as_is(true, 4096) == BlobGcAction::Rewrite,
        blob_gc_action(true, 4096) == BlobGcAction::Skip,
        blob_gc_as_is(true, 4096) != blob_gc_action(true, 4096),
{
}

fn main() {}

} // verus!
