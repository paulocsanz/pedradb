//! vlog GC decision kernel (RFC-0056 P1.4 — crash dictionary on the
//! value-log MANIFEST swing and the sealed-blob rewrite guard).
//!
//! Pure decision functions only. Production (`db.rs`) calls
//! [`vlog_recover_action`] on `open_with_env` and [`blob_gc_action`] inside
//! `compact_blob_auto`; the durable effects (open handles, rename, rewrite)
//! stay in `db.rs` / `vlog.rs`.
//!
//! Crash contract being decided:
//! - MANIFEST `vlog_use_new == true` means the committed SST inventory points
//!   into `VALUES.vlog.new`. Reopening must resolve to `.new` (or refuse),
//!   never silently fall back to the stale primary.
//! - `vlog_use_new == false` ignores an orphan `.new` (mid-GC before the
//!   MANIFEST commit) so an uncommitted swing cannot hijack reads.
//! - The sealed-blob rewrite guard never rewrites the active append
//!   generation (writers may still be appending into it).

/// What the vlog open path must do, given the recovered MANIFEST flag and
/// what actually exists on disk after a crash.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VlogRecoverAction {
    /// Blob-mode store: open the highest sealed generation.
    OpenBlob,
    /// MANIFEST committed the swing and `VALUES.vlog.new` exists: SST
    /// pointers target `.new` — open it.
    OpenNew,
    /// Primary: never swung, promote-rename already completed (flag not yet
    /// cleared), or orphan `.new` correctly ignored.
    OpenPrimary,
    /// F51: `use_new` with **neither** file on disk. Inventing an empty
    /// primary would make every large value vanish — refuse to open.
    RefuseOpen,
    /// Fresh DB / first large put: no vlog anywhere, no swing — create an
    /// empty primary.
    CreateEmptyPrimary,
    /// No vlog configured and nothing on disk: run without a value log.
    NoVlog,
}

/// Resolve which vlog handle the reopen path must use.
///
/// Mirrors `ValueLog::resolve_path` + `open_with_flag`'s F51 guard + the
/// `Db::open_with_env` blob/none gates, as one pure decision.
#[must_use]
pub fn vlog_recover_action(
    blob_active: bool,
    wants_large: bool,
    primary_exists: bool,
    use_new: bool,
    new_exists: bool,
) -> VlogRecoverAction {
    if blob_active {
        return VlogRecoverAction::OpenBlob;
    }
    // `resolve_path`: the swing target is `.new` only when it exists;
    // a missing `.new` under `use_new` reconciles to primary (rename done).
    let swung_and_staged = use_new && new_exists;
    if wants_large || primary_exists || swung_and_staged {
        if swung_and_staged {
            return VlogRecoverAction::OpenNew;
        }
        if primary_exists {
            return VlogRecoverAction::OpenPrimary;
        }
        // No primary, no `.new`.
        if use_new {
            // SST pointers target `.new` but it is gone: refuse (F51).
            return VlogRecoverAction::RefuseOpen;
        }
        return VlogRecoverAction::CreateEmptyPrimary;
    }
    VlogRecoverAction::NoVlog
}

/// AS-IS mutant: the reopen path ignores the MANIFEST swing flag and always
/// resolves to primary. After a crash mid-GC (MANIFEST committed with
/// `use_new`, SSTs remapped into `.new`), the DB silently serves large
/// values from the stale pre-GC primary — missing or resurrected data.
#[must_use]
pub fn vlog_recover_action_as_is_ignore_swing(
    blob_active: bool,
    wants_large: bool,
    primary_exists: bool,
    _use_new: bool,
    _new_exists: bool,
) -> VlogRecoverAction {
    vlog_recover_action(blob_active, wants_large, primary_exists, false, false)
}

/// Sealed-blob rewrite guard: which files `compact_blob_auto` may rewrite.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlobGcAction {
    /// Sealed, non-empty: eligible for rewrite (θ dead-ratio is policy,
    /// applied by the caller after this guard).
    Rewrite,
    /// Active append generation or empty file: must not be rewritten.
    Skip,
}

/// The structural rewrite guard: never the active generation, never empty.
///
/// Rewriting the active generation while writers append into it loses the
/// concurrent appends (file replaced underneath live offsets).
#[must_use]
pub fn blob_gc_action(is_active: bool, bytes: u64) -> BlobGcAction {
    if !is_active && bytes > 0 {
        BlobGcAction::Rewrite
    } else {
        BlobGcAction::Skip
    }
}

/// AS-IS mutant: rewrites any non-empty file, including the active append
/// generation — concurrent appends into the sealed-active file vanish.
#[must_use]
pub fn blob_gc_action_as_is_rewrite_active(_is_active: bool, bytes: u64) -> BlobGcAction {
    if bytes > 0 {
        BlobGcAction::Rewrite
    } else {
        BlobGcAction::Skip
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swing_committed_opens_new() {
        // MANIFEST committed the swing, `.new` staged, primary still there.
        assert_eq!(
            vlog_recover_action(false, true, true, true, true),
            VlogRecoverAction::OpenNew
        );
    }

    #[test]
    fn no_swing_ignores_orphan_new() {
        // Mid-GC crash BEFORE the MANIFEST commit: orphan `.new` exists but
        // the committed SST inventory still points at primary.
        assert_eq!(
            vlog_recover_action(false, true, true, false, true),
            VlogRecoverAction::OpenPrimary
        );
    }

    #[test]
    fn f51_refuses_when_both_missing_under_swing() {
        assert_eq!(
            vlog_recover_action(false, true, false, true, false),
            VlogRecoverAction::RefuseOpen
        );
    }

    #[test]
    fn promote_rename_done_reconciles_to_primary() {
        // Crash after promote rename, before MANIFEST flag clear.
        assert_eq!(
            vlog_recover_action(false, true, true, true, false),
            VlogRecoverAction::OpenPrimary
        );
    }

    #[test]
    fn fresh_db_creates_empty_primary() {
        assert_eq!(
            vlog_recover_action(false, true, false, false, false),
            VlogRecoverAction::CreateEmptyPrimary
        );
        assert_eq!(
            vlog_recover_action(false, false, false, false, false),
            VlogRecoverAction::NoVlog
        );
    }

    #[test]
    fn blob_mode_wins() {
        assert_eq!(
            vlog_recover_action(true, true, true, true, true),
            VlogRecoverAction::OpenBlob
        );
    }

    #[test]
    fn theorem_vlog_recover_on_finite_domain() {
        // 2^5 input space: fixed never opens the stale primary under a
        // committed swing; the AS-IS mutant does exactly that.
        for bits in 0u32..32 {
            let blob = bits & 1 != 0;
            let wants = bits & 2 != 0;
            let primary = bits & 4 != 0;
            let use_new = bits & 8 != 0;
            let new_exists = bits & 16 != 0;
            let a = vlog_recover_action(blob, wants, primary, use_new, new_exists);
            let m =
                vlog_recover_action_as_is_ignore_swing(blob, wants, primary, use_new, new_exists);
            if blob {
                assert_eq!(a, VlogRecoverAction::OpenBlob);
                continue;
            }
            if use_new && new_exists && !blob {
                // Committed swing, staged: only `.new` is correct.
                assert_eq!(a, VlogRecoverAction::OpenNew);
                if primary {
                    // Mutant divergence: stale primary under committed swing.
                    assert_eq!(m, VlogRecoverAction::OpenPrimary);
                    assert_ne!(a, m);
                }
            }
            if use_new && !new_exists && !primary && wants {
                // Neither file under a committed swing: fixed refuses (F51),
                // mutant invents an empty primary → large values vanish.
                assert_eq!(a, VlogRecoverAction::RefuseOpen);
                assert_eq!(m, VlogRecoverAction::CreateEmptyPrimary);
                assert_ne!(a, m);
            }
            if !use_new && new_exists {
                // Orphan `.new` must never be opened.
                assert_ne!(a, VlogRecoverAction::OpenNew);
            }
        }
    }

    #[test]
    fn theorem_blob_gc_on_finite_domain() {
        for is_active in [false, true] {
            for bytes in [0u64, 1, 4096] {
                let a = blob_gc_action(is_active, bytes);
                let m = blob_gc_action_as_is_rewrite_active(is_active, bytes);
                if is_active && bytes > 0 {
                    // Active append generation: fixed skips, mutant rewrites
                    // (concurrent appends would vanish).
                    assert_eq!(a, BlobGcAction::Skip);
                    assert_eq!(m, BlobGcAction::Rewrite);
                    assert_ne!(a, m);
                } else {
                    assert_eq!(a, m);
                }
                if bytes == 0 {
                    assert_eq!(a, BlobGcAction::Skip);
                }
            }
        }
    }

    #[test]
    fn vlog_recover_action_on_live_swing_is_not_ok() {
        let a = vlog_recover_action(false, true, true, true, true);
        let m = vlog_recover_action_as_is_ignore_swing(false, true, true, true, true);
        assert_ne!(a, m, "AS-IS dente: ignore committed swing");
    }

    #[test]
    fn blob_gc_action_on_live_active_is_not_ok() {
        assert_eq!(blob_gc_action(true, 4096), BlobGcAction::Skip);
        assert_eq!(
            blob_gc_action_as_is_rewrite_active(true, 4096),
            BlobGcAction::Rewrite,
            "AS-IS dente: rewrite the live blob"
        );
    }
}
