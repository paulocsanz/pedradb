// Verus proof of the group-commit kernel
// (RFC-0057 P2.1 / RFC-0058 P2.1 — RFC-0051 P1.3 semantics as a theorem).
//
// Source of truth for production remains `src/group_commit_kernel.rs`
// (called by `concurrent.rs::validate_occ_batch` / `lone_commit` and
// `db.rs::GroupInFlight::max_appended_seq`). This file is the
// machine-checked theorem:
//   - first-committer-wins exec == spec, and the `last_seq == snap`
//     fast path never conflicts (soundness of skipping the scan);
//   - GROUP ATOMICITY: `group_validate` decides every member against
//     the same `last_seq` — member i's outcome in ANY group equals its
//     solo outcome (simultaneity, no serialization order between
//     members of the same group);
//   - FENCE: `fence_publish_seq` is the max member sequence (one publish
//     watermark for the whole group) and never below any member;
//   - teeth: the serialized mutant (`occ_conflict_as_is_serialized`)
//     aborts a same-group writer where the group form commits — exactly
//     the RFC-0051 P1.3 planted-bug divergence.
//
//   ./scripts/verus_group_commit.sh
//
// Do not link this into the production crate — it is a twin of the pure kernel.

use vstd::prelude::*;

verus! {

/// Mirrors `OccRead` in group_commit_kernel.rs.
pub struct OccRead {
    pub snap: u64,
    pub touched_key_written_after: bool,
}

// ---------------------------------------------------------------------
// Specs
// ---------------------------------------------------------------------

/// Conflict iff the window `(snap, last_seq]` is non-empty AND some
/// touched key was written inside it (first-committer-wins).
pub open spec fn occ_conflict_spec(
    snap: u64,
    last_seq: u64,
    touched_key_written_after: bool,
) -> bool {
    last_seq > snap && touched_key_written_after
}

/// Max of the first `i` elements of `s` (0 for the empty prefix).
pub open spec fn max_prefix(s: Seq<u64>, i: int) -> u64
    recommends 0 <= i <= s.len(),
    decreases i,
{
    if 0 < i && i <= s.len() {
        let m = max_prefix(s, i - 1);
        if s[i - 1] > m {
            s[i - 1]
        } else {
            m
        }
    } else {
        0
    }
}

pub open spec fn fence_publish_seq_spec(member_seqs: &[u64]) -> u64 {
    max_prefix(member_seqs@, member_seqs@.len() as int)
}

/// Pointwise meaning of the group decision: position i is member i's
/// solo predicate — nothing else enters.
pub open spec fn group_validate_spec(reads: &[OccRead], last_seq: u64) -> Seq<bool> {
    Seq::new(
        reads@.len(),
        |i: int|
            if 0 <= i < reads@.len() as int {
                occ_conflict_spec(reads[i].snap, last_seq, reads[i].touched_key_written_after)
            } else {
                false
            },
    )
}

/// The serialized mutant (TEST-ONLY): later members validate against
/// `last_seq + writes_before`.
pub open spec fn as_is_serialized_spec(
    snap: u64,
    last_seq: u64,
    writes_before: u64,
    touched_key_written_after: bool,
) -> bool {
    last_seq + writes_before > snap && touched_key_written_after
}

pub open spec fn solo_outcome_spec(r: OccRead, last_seq: u64) -> bool {
    occ_conflict_spec(r.snap, last_seq, r.touched_key_written_after)
}

// ---------------------------------------------------------------------
// Exec twins (must match production bit-for-bit; index loops so the
// Aeneas extract stays axiom-free)
// ---------------------------------------------------------------------

#[verifier::when_used_as_spec(occ_conflict_spec)]
pub fn occ_conflict(snap: u64, last_seq: u64, touched_key_written_after: bool) -> (c: bool)
    ensures
        c == occ_conflict_spec(snap, last_seq, touched_key_written_after),
{
    last_seq > snap && touched_key_written_after
}

pub fn group_validate(reads: &[OccRead], last_seq: u64) -> (out: Vec<bool>)
    ensures
        out.len() == reads.len(),
        out@ == group_validate_spec(reads, last_seq),
        forall|i: int|
            0 <= i < reads.len() ==> out[i] == occ_conflict_spec(
                reads[i].snap,
                last_seq,
                reads[i].touched_key_written_after,
            ),
{
    let mut out: Vec<bool> = Vec::new();
    let mut i: usize = 0;
    while i < reads.len()
        invariant
            0 <= i <= reads.len(),
            out.len() == i,
            forall|j: int|
                0 <= j < i ==> out[j] == occ_conflict_spec(
                    reads[j].snap,
                    last_seq,
                    reads[j].touched_key_written_after,
                ),
        decreases reads.len() - i,
    {
        out.push(occ_conflict(reads[i].snap, last_seq, reads[i].touched_key_written_after));
        i += 1;
    }
    out
}

#[verifier::when_used_as_spec(fence_publish_seq_spec)]
pub fn fence_publish_seq(member_seqs: &[u64]) -> (p: u64)
    ensures
        p == fence_publish_seq_spec(member_seqs),
        forall|i: int| 0 <= i < member_seqs.len() ==> member_seqs[i] <= p,
{
    let mut best: u64 = 0;
    let mut i: usize = 0;
    while i < member_seqs.len()
        invariant
            0 <= i <= member_seqs.len(),
            best == max_prefix(member_seqs@, i as int),
        decreases member_seqs.len() - i,
    {
        if member_seqs[i] > best {
            best = member_seqs[i];
        }
        i += 1;
    }
    proof {
        max_prefix_ge_elem(member_seqs@, member_seqs@.len() as int);
    }
    best
}

/// Prefix max is an upper bound of its own prefix (induction on the
/// prefix length).
proof fn max_prefix_ge_elem(s: Seq<u64>, k: int)
    requires
        0 <= k <= s.len(),
    ensures
        forall|j: int| 0 <= j < k ==> s[j] <= max_prefix(s, k),
    decreases k,
{
    if k > 0 {
        max_prefix_ge_elem(s, k - 1);
        assert(max_prefix(s, k) >= max_prefix(s, k - 1));
        assert(max_prefix(s, k) >= s[k - 1]);
    }
}

/// TEST-ONLY mutant: the serialized scheduler. Caller invariant: the
/// modeled `writes_before` is small (same-group members), so the sum
/// cannot overflow — production never calls this function.
#[verifier::when_used_as_spec(as_is_serialized_spec)]
pub fn occ_conflict_as_is_serialized(
    snap: u64,
    last_seq: u64,
    writes_before: u64,
    touched_key_written_after: bool,
) -> (c: bool)
    requires
        last_seq + writes_before <= 0xffff_ffff_ffff_ffff,
    ensures
        c == as_is_serialized_spec(snap, last_seq, writes_before, touched_key_written_after),
{
    last_seq + writes_before > snap && touched_key_written_after
}

// ---------------------------------------------------------------------
// Theorems (RFC-0057 P2.1)
// ---------------------------------------------------------------------

/// The `last_seq == snap` fast path is sound: empty window ⇒ no
/// conflict, whatever the scan said.
proof fn fast_path_never_conflicts()
    ensures forall|snap: u64, touched: bool| occ_conflict(snap, snap, touched) == false,
{
}

/// The window predicate is monotone in `last_seq`: a member that
/// validates clean against `last_seq` also validates clean against any
/// smaller sequence point (used by the caller-side reasoning that the
/// collected `OccRead`s cannot go stale while the lock is held).
proof fn clean_against_last_seq_is_clean_against_smaller(
    snap: u64,
    last_seq: u64,
    touched: bool,
    smaller: u64,
)
    requires
        smaller <= last_seq,
        occ_conflict(snap, last_seq, touched) == false,
    ensures
        occ_conflict(snap, smaller, touched) == false,
{
}

/// SOLO outcome of one member (the scalar decision it would take alone).
#[verifier::when_used_as_spec(solo_outcome_spec)]
fn solo_outcome(r: OccRead, last_seq: u64) -> (b: bool)
    ensures
        b == solo_outcome_spec(r, last_seq),
{
    occ_conflict(r.snap, last_seq, r.touched_key_written_after)
}

/// GROUP ATOMICITY (simultaneity): member i's outcome in ANY group is
/// its solo outcome — it cannot depend on which other members joined
/// the group or on their order. This is the "no serialization order
/// between members of the same group" of RFC-0051 P1.3, as a theorem
/// over the kernel's own group decision. The exec `group_validate`
/// ties itself to this spec by its `ensures` (`out@ ==
/// group_validate_spec`), checked by Verus — the bridge is verified,
/// not assumed.
proof fn group_outcome_is_solo_outcome(
    a: &[OccRead],
    b: &[OccRead],
    last_seq: u64,
    i: usize,
)
    requires
        i < a.len(),
        i < b.len(),
        a[i as int] == b[i as int],
    ensures
        group_validate_spec(a, last_seq)[i as int] == solo_outcome_spec(a[i as int], last_seq),
        group_validate_spec(a, last_seq)[i as int] == group_validate_spec(b, last_seq)[i as int],
{
    assert(group_validate_spec(a, last_seq)[i as int]
        == occ_conflict_spec(a[i as int].snap, last_seq, a[i as int].touched_key_written_after));
    assert(group_validate_spec(b, last_seq)[i as int]
        == occ_conflict_spec(b[i as int].snap, last_seq, b[i as int].touched_key_written_after));
}

/// FENCE: the publish watermark is the max member sequence and never
/// below any member — the group becomes visible as one step.
proof fn fence_covers_every_member(member_seqs: &[u64])
    ensures
        forall|i: int| 0 <= i < member_seqs.len() ==> member_seqs[i] <= fence_publish_seq(member_seqs),
{
    max_prefix_ge_elem(member_seqs@, member_seqs@.len() as int);
}

/// TEETH (RFC-0051 P1.3 planted-bug shape): two members of one group,
/// second touched the key the first writes (intra-group write). The
/// group form commits both — simultaneity; the serialized mutant
/// aborts the second. The divergence is exactly where the two
/// semantics differ.
proof fn serialized_mutant_diverges_on_intra_group_write()
    ensures
        occ_conflict(10, 10, true) == false,
        occ_conflict_as_is_serialized(10, 10, 1, true) == true,
{
}

/// RFC-0071 P2.1: visibility publish only when WAL I/O succeeded.
pub open spec fn may_publish_group_spec(wal_io_ok: bool) -> bool {
    wal_io_ok
}

pub open spec fn may_publish_group_as_is_spec(_wal_io_ok: bool) -> bool {
    true
}

pub fn may_publish_group(wal_io_ok: bool) -> (ok: bool)
    ensures
        ok == may_publish_group_spec(wal_io_ok),
{
    wal_io_ok
}

pub fn may_publish_group_as_is(_wal_io_ok: bool) -> (ok: bool)
    ensures
        ok == may_publish_group_as_is_spec(_wal_io_ok),
{
    true
}

proof fn lemma_failed_wal_does_not_publish()
    ensures
        !may_publish_group_spec(false),
        may_publish_group_as_is_spec(false),
{
}

/// RFC-0070 P2.1: finite PCT depth never admits ∀ OS schedules.
pub open spec fn forall_schedules_admitted_spec(_pct_depth: u64) -> bool {
    false
}

pub open spec fn forall_schedules_admitted_as_is_spec(pct_depth: u64) -> bool {
    pct_depth >= 2
}

pub fn forall_schedules_admitted(_pct_depth: u64) -> (ok: bool)
    ensures
        ok == forall_schedules_admitted_spec(_pct_depth),
{
    false
}

pub fn forall_schedules_admitted_as_is(pct_depth: u64) -> (ok: bool)
    ensures
        ok == forall_schedules_admitted_as_is_spec(pct_depth),
{
    pct_depth >= 2
}

proof fn lemma_d2_is_not_forall()
    ensures
        !forall_schedules_admitted_spec(2),
        forall_schedules_admitted_as_is_spec(2),
{
}

/// RFC-0155 P2.1 / R-group-glue: lock/OS-scheduler interleavings are not ∀π.
pub open spec fn lock_interleavings_admitted_spec() -> bool {
    false
}

pub open spec fn lock_interleavings_admitted_as_is_spec() -> bool {
    true
}

pub fn lock_interleavings_admitted() -> (ok: bool)
    ensures
        ok == lock_interleavings_admitted_spec(),
{
    false
}

pub fn lock_interleavings_admitted_as_is() -> (ok: bool)
    ensures
        ok == lock_interleavings_admitted_as_is_spec(),
{
    true
}

/// RFC-0070 P2.2: campaign default PCT depth stays 2. d>2 remains RFC-0051.
pub open spec fn pct_campaign_default_depth_spec() -> u64 {
    2
}

pub fn pct_campaign_default_depth() -> (d: u64)
    ensures
        d == pct_campaign_default_depth_spec(),
{
    2
}

/// Admit a “0070 raised the default PCT depth” claim. Always false.
pub open spec fn default_pct_depth_raised_spec() -> bool {
    false
}

pub open spec fn default_pct_depth_raised_as_is_spec() -> bool {
    true
}

pub fn default_pct_depth_raised() -> (ok: bool)
    ensures
        ok == default_pct_depth_raised_spec(),
{
    false
}

pub fn default_pct_depth_raised_as_is() -> (ok: bool)
    ensures
        ok == default_pct_depth_raised_as_is_spec(),
{
    true
}

proof fn lemma_default_depth_not_raised()
    ensures
        pct_campaign_default_depth_spec() == 2,
        !default_pct_depth_raised_spec(),
        default_pct_depth_raised_as_is_spec(),
{
}

/// RFC-0078 P2.1: promote pending bytes only when the Env is honest.
pub open spec fn fsync_promotes_pending_spec(os_honest: bool) -> bool {
    os_honest
}

pub open spec fn fsync_promotes_pending_as_is_spec(_os_honest: bool) -> bool {
    true
}

pub fn fsync_promotes_pending(os_honest: bool) -> (ok: bool)
    ensures
        ok == fsync_promotes_pending_spec(os_honest),
{
    os_honest
}

pub fn fsync_promotes_pending_as_is(_os_honest: bool) -> (ok: bool)
    ensures
        ok == fsync_promotes_pending_as_is_spec(_os_honest),
{
    true
}

proof fn lemma_lying_fsync_does_not_promote()
    ensures
        !fsync_promotes_pending_spec(false),
        fsync_promotes_pending_as_is_spec(false),
{
}

/// RFC-0078 P2.1: fsync Ok is not a media theorem.
pub open spec fn media_durable_admitted_spec(_fsync_ok: bool) -> bool {
    false
}

pub open spec fn media_durable_admitted_as_is_spec(fsync_ok: bool) -> bool {
    fsync_ok
}

pub fn media_durable_admitted(_fsync_ok: bool) -> (ok: bool)
    ensures
        ok == media_durable_admitted_spec(_fsync_ok),
{
    false
}

pub fn media_durable_admitted_as_is(fsync_ok: bool) -> (ok: bool)
    ensures
        ok == media_durable_admitted_as_is_spec(fsync_ok),
{
    fsync_ok
}

proof fn lemma_fsync_ok_is_not_media_proof()
    ensures
        !media_durable_admitted_spec(true),
        media_durable_admitted_as_is_spec(true),
{
}

} // verus!
