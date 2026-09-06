// Verus twin of the env-crash kernel (RFC-0166 P1.1 —
// crates/pedradb-core/src/env_crash_kernel.rs). Not linked into production.
//
//   ./scripts/verus_env_crash.sh
//
// Theorems: exec == spec for the crash geometry; the barrier floor and
// no-invented-bytes corollaries; honest sync protects the whole log; and
// the two AS-IS teeth (floor-less "legal" crash; lying sync pretending
// it promoted) are witnessed to diverge from the real semantics.

use vstd::prelude::*;

verus! {

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum SyncHonesty {
    Honest,
    Lying,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub struct CrashModel {
    pub written: u64,
    pub synced: u64,
}

// The proved group_commit kernel fact, restated for this crate-local twin.
pub open spec fn fsync_promotes_spec(os_honest: bool) -> bool {
    os_honest
}

pub open spec fn append_spec(m: CrashModel, n: u64) -> CrashModel {
    CrashModel { written: (m.written + n) as u64, synced: m.synced }
}

pub open spec fn sync_spec(m: CrashModel, h: SyncHonesty) -> CrashModel {
    if fsync_promotes_spec(h == SyncHonesty::Honest) {
        CrashModel { written: m.written, synced: m.written }
    } else {
        m
    }
}

pub open spec fn crash_legal_spec(m: CrashModel, cut: u64) -> bool {
    m.synced <= cut && cut <= m.written
}

pub fn append(m: CrashModel, n: u64) -> (r: CrashModel)
    requires
        n <= u64::MAX - m.written,
    ensures
        r == append_spec(m, n),
{
    CrashModel { written: m.written + n, synced: m.synced }
}

pub fn sync(m: CrashModel, h: SyncHonesty) -> (r: CrashModel)
    ensures
        r == sync_spec(m, h),
{
    if fsync_promotes_spec_is_honest(h) {
        CrashModel { written: m.written, synced: m.written }
    } else {
        m
    }
}

fn fsync_promotes_spec_is_honest(h: SyncHonesty) -> (b: bool)
    ensures
        b == fsync_promotes_spec(h == SyncHonesty::Honest),
{
    match h {
        SyncHonesty::Honest => true,
        SyncHonesty::Lying => false,
    }
}

pub fn crash_legal(m: CrashModel, cut: u64) -> (b: bool)
    ensures
        b == crash_legal_spec(m, cut),
{
    m.synced <= cut && cut <= m.written
}

/// Corollary (floor): a legal crash never loses a synced byte.
pub fn barrier_floor_holds(m: CrashModel, cut: u64) -> (b: bool)
    ensures
        b == (!crash_legal_spec(m, cut) || cut >= m.synced),
{
    !crash_legal(m, cut) || cut >= m.synced
}

/// Corollary (ceiling): a legal crash never survives past `written` —
/// recovery can never observe a byte the writer never appended.
pub fn no_invented_bytes_holds(m: CrashModel, cut: u64) -> (b: bool)
    ensures
        b == (!crash_legal_spec(m, cut) || cut <= m.written),
{
    !crash_legal(m, cut) || cut <= m.written
}

/// Honest sync is a real barrier: after it, every legal crash keeps the
/// whole log.
pub fn honest_sync_protects_all(m: CrashModel, cut: u64) -> (b: bool)
    ensures
        b == (!crash_legal_spec(sync_spec(m, SyncHonesty::Honest), cut)
            || cut == m.written),
{
    !crash_legal(sync(m, SyncHonesty::Honest), cut)
        || cut == sync(m, SyncHonesty::Honest).written
}

// --- Corollaries ------------------------------------------------------------

/// Floor: a legal crash never loses a synced byte.
proof fn barrier_floor_theorem(m: CrashModel, cut: u64)
    ensures
        crash_legal_spec(m, cut) ==> cut >= m.synced,
{
}

/// Ceiling: a legal crash never survives past `written` — recovery can
/// never observe a byte the writer never appended.
proof fn no_invented_bytes_theorem(m: CrashModel, cut: u64)
    ensures
        crash_legal_spec(m, cut) ==> cut <= m.written,
{
}

/// Honest sync is a real barrier: afterwards every legal crash keeps the
/// whole log.
proof fn honest_sync_protects_all_theorem(m: CrashModel, cut: u64)
    ensures
        crash_legal_spec(sync_spec(m, SyncHonesty::Honest), cut)
            ==> cut == m.written,
{
}

/// Lying sync promotes nothing: the barrier stays where it was, so the
/// barrier-floor cut stays legal and drops every unsynced byte.
/// (Requires the well-formed log `synced <= written` that
/// `CrashModel::new` clamps in the kernel.)
proof fn lying_sync_promotes_nothing_theorem(m: CrashModel)
    requires
        m.synced <= m.written,
    ensures
        sync_spec(m, SyncHonesty::Lying) == m,
        crash_legal_spec(sync_spec(m, SyncHonesty::Lying), m.synced),
{
}

// --- AS-IS teeth ------------------------------------------------------------

/// AS-IS 1: any cut <= written is "legal" (the floor is ignored).
pub open spec fn crash_legal_as_is_spec(m: CrashModel, cut: u64) -> bool {
    cut <= m.written
}

proof fn as_is_legal_does_not_imply_real_legal()
    ensures
        crash_legal_as_is_spec(CrashModel { written: 10, synced: 5 }, 3),
        !crash_legal_spec(CrashModel { written: 10, synced: 5 }, 3),
        // and the floor theorem keeps holding for the real legality.
        !(crash_legal_spec(CrashModel { written: 10, synced: 5 }, 3)),
{
}

/// AS-IS 2: a lying sync pretends it promoted (RFC-0078 as-is).
pub open spec fn sync_lying_promotes_as_is_spec(m: CrashModel) -> CrashModel {
    CrashModel { written: m.written, synced: m.written }
}

proof fn lying_promote_as_is_diverges()
    ensures
        sync_spec(CrashModel { written: 4, synced: 0 }, SyncHonesty::Lying)
            != sync_lying_promotes_as_is_spec(CrashModel { written: 4, synced: 0 }),
        crash_legal_spec(sync_spec(CrashModel { written: 4, synced: 0 }, SyncHonesty::Lying), 0),
        !crash_legal_spec(sync_lying_promotes_as_is_spec(CrashModel { written: 4, synced: 0 }), 3),
{
}

} // verus!
