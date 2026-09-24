//! RFC-0057 P2.1 / RFC-0058 P2.1: the group-commit kernel — the pure
//! decision core of the `ConcurrentDb` write group. The semantics
//! documented in RFC-0051 P1.3 (and enforced there by an empirical
//! oracle) become a theorem target here:
//!
//! - **first-committer-wins** ([`occ_conflict`]): a member that read
//!   snapshot `snap` conflicts iff some key it touched was written in
//!   `(snap, last_seq]`. Called by `WriteGroup::validate_occ_batch` and
//!   `WriteGroup::lone_commit` (`concurrent.rs`).
//! - **group atomicity** ([`group_validate`]): every member of a group
//!   is validated against the **same** `last_seq`, before any of the
//!   group's own sequences exist — members of the same group are
//!   simultaneous, with no serialization order between them (intra-group
//!   writes never conflict). Inner loop of [`occ_batch_plan`]; production
//!   `WriteGroup::validate_occ_batch` calls `occ_batch_plan` (RFC-0222 P0.6).
//! - **fence** ([`fence_publish_seq`]): the group becomes visible at one
//!   publish watermark — the max appended member sequence — after WAL
//!   durability. Called by `GroupInFlight::max_appended_seq` (`db.rs`).
//!
//! The Verus twin is `crates/pedradb-core/verus/group_commit.rs` except
//! [`rwlock_client_may_mutate`]: that fn is **single artifact** — this
//! file is what `rustc` links *and* what Verus proves
//! (`cfg(verus_keep_ghost)`). `./scripts/verus_group_commit_kernel.sh`
//!
//! The Aeneas extract is `formal/aeneas/lean/GroupCommitKernel.lean` with
//! theorems in `GroupCommit.lean` (second machine).
//!
//! `occ_conflict_as_is_serialized` is TEST-ONLY teeth (the serialized
//! mutant the theorems diverge from); production never calls it.

macro_rules! rwlock_client_may_mutate_body {
    ($holding_write:expr) => {
        $holding_write
    };
}

macro_rules! rwlock_client_may_mutate_as_is_body {
    ($holding_write:expr) => {{
        let _ = $holding_write;
        true
    }};
}

/// Shared read of `Db` is allowed with a read **or** write guard. Always
/// calls the mutate token so Lean can unfold both.
macro_rules! rwlock_client_may_read_body {
    ($holding_read:expr, $holding_write:expr) => {{
        let write_ok = rwlock_client_may_mutate($holding_write);
        $holding_read || write_ok
    }};
}

macro_rules! rwlock_client_may_read_as_is_body {
    ($holding_read:expr, $holding_write:expr) => {{
        let _ = ($holding_read, $holding_write);
        true
    }};
}

macro_rules! occ_member_fate_body {
    ($too_old:expr, $conflict:expr) => {
        if $too_old {
            OccMemberFate::TooOld
        } else if $conflict {
            OccMemberFate::Conflict
        } else {
            OccMemberFate::Ok
        }
    };
}

macro_rules! occ_member_fate_as_is_body {
    ($too_old:expr, $conflict:expr) => {{
        let _ = ($too_old, $conflict);
        OccMemberFate::Ok
    }};
}

macro_rules! occ_conflict_body {
    ($snap:expr, $last_seq:expr, $touched:expr) => {
        $last_seq > $snap && $touched
    };
}

macro_rules! occ_conflict_as_is_serialized_body {
    ($snap:expr, $last_seq:expr, $writes_before:expr, $touched:expr) => {
        $last_seq + $writes_before > $snap && $touched
    };
}

macro_rules! may_publish_group_body {
    ($wal_io_ok:expr) => {
        $wal_io_ok
    };
}

macro_rules! may_publish_group_as_is_body {
    ($wal_io_ok:expr) => {{
        let _ = $wal_io_ok;
        true
    }};
}

macro_rules! lock_interleavings_admitted_body {
    () => {
        false
    };
}

macro_rules! lock_interleavings_admitted_as_is_body {
    () => {
        true
    };
}

macro_rules! forall_schedules_admitted_body {
    ($pct_depth:expr) => {{
        let _ = $pct_depth;
        false
    }};
}

macro_rules! forall_schedules_admitted_as_is_body {
    ($pct_depth:expr) => {
        $pct_depth >= 2
    };
}

macro_rules! fsync_promotes_pending_body {
    ($os_honest:expr) => {
        $os_honest
    };
}

macro_rules! fsync_promotes_pending_as_is_body {
    ($os_honest:expr) => {{
        let _ = $os_honest;
        true
    }};
}

macro_rules! media_durable_admitted_body {
    ($fsync_ok:expr) => {{
        let _ = $fsync_ok;
        false
    }};
}

macro_rules! media_durable_admitted_as_is_body {
    ($fsync_ok:expr) => {
        $fsync_ok
    };
}

macro_rules! stacked_fsync_liars_admitted_body {
    ($lying:expr, $det_io:expr) => {{
        let _ = ($lying, $det_io);
        false
    }};
}

macro_rules! stacked_fsync_liars_admitted_as_is_body {
    ($lying:expr, $det_io:expr) => {
        $lying && $det_io
    };
}

macro_rules! fsync_lie_closes_tcg_guest_body {
    () => {
        false
    };
}

macro_rules! fsync_lie_closes_tcg_guest_as_is_body {
    () => {
        true
    };
}

macro_rules! pct_campaign_default_depth_body {
    () => {
        2u64
    };
}

macro_rules! pct_campaign_default_depth_as_is_body {
    () => {
        3u64
    };
}

macro_rules! default_pct_depth_raised_body {
    () => {
        false
    };
}

macro_rules! default_pct_depth_raised_as_is_body {
    () => {
        true
    };
}

/// RFC-0229 P0.3 / RFC-0220 P2.3: the chain-3 PCT signature (d=2 miss,
/// d=3 hit) is a campaign **plant**, never a ∀π theorem.
macro_rules! pct_chain3_row_is_plant_body {
    () => {
        true
    };
}

/// AS-IS: round the plant to a ∀-schedules theorem (the 0220/0229 hole).
macro_rules! pct_chain3_row_is_plant_as_is_body {
    () => {
        false
    };
}

/// First-committer-wins predicate (OCC): a transaction that read
/// snapshot `snap` against current `last_seq` conflicts iff the window
/// `(snap, last_seq]` is non-empty **and** some key it touched was
/// written inside it. `last_seq > snap` (not `!=`) is the faithful
/// window: with `last_seq <= snap` the window is empty and no key can
/// be in it.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn occ_conflict(snap: u64, last_seq: u64, touched_key_written_after: bool) -> bool {
    occ_conflict_body!(snap, last_seq, touched_key_written_after)
}

/// One member's OCC read of the pre-group state (collected under the
/// write lock, before any group sequence is assigned).
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OccRead {
    /// Snapshot the member read at.
    pub snap: u64,
    /// Whether any key the member touched (read set ∪ write set) was
    /// written in `(snap, last_seq]`.
    pub touched_key_written_after: bool,
}

/// Group validation — the pure form of `validate_occ_batch`: every
/// member is decided against the same `last_seq`, so a member's outcome
/// never depends on another member (simultaneity). Position-for-position
/// conflict flags.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn group_validate(reads: &[OccRead], last_seq: u64) -> Vec<bool> {
    let mut out = Vec::with_capacity(reads.len());
    let mut i = 0;
    while i < reads.len() {
        out.push(occ_conflict(
            reads[i].snap,
            last_seq,
            reads[i].touched_key_written_after,
        ));
        i += 1;
    }
    out
}

/// Fate of one OCC member after `group_validate` (and snapshot TooOld).
/// `validate_occ_batch` matches this — TooOld wins over Conflict.
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccMemberFate {
    /// Apply with the group.
    Ok,
    /// Snapshot unreadable — abort TooOld.
    TooOld,
    /// OCC conflict — abort TransactionConflict.
    Conflict,
}

/// Caller of `group_validate`: too-old or conflict ⇒ abort that member.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn occ_member_fate(too_old: bool, conflict: bool) -> OccMemberFate {
    occ_member_fate_body!(too_old, conflict)
}

/// AS-IS: never abort (lagging member commits).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn occ_member_fate_as_is(_too_old: bool, _conflict: bool) -> OccMemberFate {
    occ_member_fate_as_is_body!(_too_old, _conflict)
}

/// ConcurrentDb `validate_occ_batch` / `lone_commit` plan: TooOld wins
/// over Conflict over Ok, against one `last_seq`. Glue collects
/// (`too_old`, `OccRead`); this fn is the order rustc links.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn occ_batch_plan(too_old: &[bool], reads: &[OccRead], last_seq: u64) -> Vec<OccMemberFate> {
    let n = if too_old.len() <= reads.len() {
        too_old.len()
    } else {
        reads.len()
    };
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let conflict = occ_conflict(reads[i].snap, last_seq, reads[i].touched_key_written_after);
        out.push(occ_member_fate(too_old[i], conflict));
        i += 1;
    }
    out
}

/// AS-IS: every member Ok (lagging / too-old still commit).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn occ_batch_plan_as_is(
    too_old: &[bool],
    reads: &[OccRead],
    _last_seq: u64,
) -> Vec<OccMemberFate> {
    let n = if too_old.len() <= reads.len() {
        too_old.len()
    } else {
        reads.len()
    };
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let _ = (too_old[i], reads[i]);
        out.push(OccMemberFate::Ok);
        i += 1;
    }
    out
}

/// The fence watermark: one publish sequence for the whole group — the
/// max appended member sequence (0 for an empty group).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn fence_publish_seq(member_seqs: &[u64]) -> u64 {
    let mut best = 0;
    let mut i = 0;
    while i < member_seqs.len() {
        if member_seqs[i] > best {
            best = member_seqs[i];
        }
        i += 1;
    }
    best
}

/// AS-IS RFC-0057: fence is the first member's seq — later members stay
/// unpublished at the watermark.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn fence_publish_seq_as_is(member_seqs: &[u64]) -> u64 {
    if member_seqs.is_empty() {
        0
    } else {
        member_seqs[0]
    }
}

/// TEST-ONLY mutant (never called in production): the serialized
/// scheduler — members commit one at a time, so member `writes_before`
/// later members validate against `last_seq + writes_before`. With an
/// intra-group write to a shared key, the serialized form conflicts
/// where the group form does not: that divergence is exactly the
/// RFC-0051 P1.3 planted-bug shape the theorems pin.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn occ_conflict_as_is_serialized(
    snap: u64,
    last_seq: u64,
    writes_before: u64,
    touched_key_written_after: bool,
) -> bool {
    occ_conflict_as_is_serialized_body!(snap, last_seq, writes_before, touched_key_written_after)
}

/// Finite PCT depth never covers ∀ OS interleavings (RFC-0070 / R-pct).
/// A campaign of depth `pct_depth` (including d=2) is not a ∀π theorem.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn forall_schedules_admitted(_pct_depth: u64) -> bool {
    forall_schedules_admitted_body!(_pct_depth)
}

/// AS-IS: d≥2 is rounded to forall (the 0070 hole — PCT CLEAN as a theorem).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn forall_schedules_admitted_as_is(pct_depth: u64) -> bool {
    forall_schedules_admitted_as_is_body!(pct_depth)
}

/// RFC-0070 P2.2: campaign default PCT depth. d>2 stays RFC-0051
/// (`planted_depth3_three_teeth`); this RFC does not raise it.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn pct_campaign_default_depth() -> u64 {
    pct_campaign_default_depth_body!()
}

/// AS-IS: 0070 P2 is rounded to “default PCT depth is now 3”.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn pct_campaign_default_depth_as_is() -> u64 {
    pct_campaign_default_depth_as_is_body!()
}

/// RFC-0070 P2.2: admit a “0070 raised the default PCT depth” claim.
/// Always false.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn default_pct_depth_raised() -> bool {
    default_pct_depth_raised_body!()
}

/// AS-IS: 0070 P2 is rounded to “we now run d>2 by default”.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn default_pct_depth_raised_as_is() -> bool {
    default_pct_depth_raised_as_is_body!()
}

/// RFC-0229 P0.3: scheduler row for the chain-3 PCT plant. Production
/// names it a plant. AS-IS would call it a theorem (`false` here).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn pct_chain3_row_is_plant() -> bool {
    pct_chain3_row_is_plant_body!()
}

/// AS-IS: the d=3 campaign is rounded to ∀ OS schedules.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn pct_chain3_row_is_plant_as_is() -> bool {
    pct_chain3_row_is_plant_as_is_body!()
}

/// Visibility publish after group (or lone) WAL I/O (RFC-0071 / R-group-glue).
/// The group becomes visible only when off-lock / lone WAL I/O succeeded.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn may_publish_group(wal_io_ok: bool) -> bool {
    may_publish_group_body!(wal_io_ok)
}

/// Data-race token (CapybaraKV RW-lock *client*, not `parking_lot`):
/// exclusive mutate of `Db` only while the write guard is held. Off-lock
/// fd (`drop(guard)` then `sync_data`) must pass `false`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn rwlock_client_may_mutate(holding_write: bool) -> bool {
    rwlock_client_may_mutate_body!(holding_write)
}

/// AS-IS: mutate even after dropping the write lock (data-race lie).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn rwlock_client_may_mutate_as_is(_holding_write: bool) -> bool {
    rwlock_client_may_mutate_as_is_body!(_holding_write)
}

/// Data-race token (CapybaraKV RW-lock *client*): shared read of `Db` only
/// while a read **or** write guard is held. `occ_snapshot` matches this —
/// no guard ⇒ published seq, not `last_sequence`. Calls
/// [`rwlock_client_may_mutate`].
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn rwlock_client_may_read(holding_read: bool, holding_write: bool) -> bool {
    rwlock_client_may_read_body!(holding_read, holding_write)
}

/// AS-IS: read `Db` with no guard (data-race lie).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn rwlock_client_may_read_as_is(_holding_read: bool, _holding_write: bool) -> bool {
    rwlock_client_may_read_as_is_body!(_holding_read, _holding_write)
}

/// AS-IS: publish even if WAL I/O failed (the 0071 hole — Ok with a lie).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn may_publish_group_as_is(_wal_io_ok: bool) -> bool {
    may_publish_group_as_is_body!(_wal_io_ok)
}

/// RFC-0219 P2.1: fate of the group's visibility publish after the
/// (lone/group) WAL I/O.
#[cfg(not(verus_keep_ghost))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupAckPlan {
    /// WAL I/O ok — publish the group's entries and ack the batch.
    AckPublishGroup,
    /// WAL I/O failed — fence; no publish, no Ok.
    FenceRefuseIoFail,
}

#[cfg(not(verus_keep_ghost))]
/// Publish EXACTLY when the group's WAL I/O succeeded (may_publish_group
/// stays live and proved in the body).
#[must_use]
pub fn group_ack_plan(wal_io_ok: bool) -> GroupAckPlan {
    if may_publish_group(wal_io_ok) {
        GroupAckPlan::AckPublishGroup
    } else {
        GroupAckPlan::FenceRefuseIoFail
    }
}

/// AS-IS: acks even when the WAL I/O failed (Ok with a lie — dente).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn group_ack_plan_as_is(_wal_io_ok: bool) -> GroupAckPlan {
    GroupAckPlan::AckPublishGroup
}

/// RFC-0071 P2.2: lock / OS-scheduler interleavings around the publish
/// gate are not a ∀π theorem. Always refuse.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn lock_interleavings_admitted() -> bool {
    lock_interleavings_admitted_body!()
}

/// AS-IS: a green publish gate is rounded to ∀ lock schedules.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn lock_interleavings_admitted_as_is() -> bool {
    lock_interleavings_admitted_as_is_body!()
}

/// Finite ConcurrentDb lock-client alphabet (not Linux `futex`).
/// Acquire the write-group client.
pub const LOCK_ACT_ACQUIRE_WRITE: u8 = 0;
/// Acquire the flush/rotate client (separate mutex).
pub const LOCK_ACT_ACQUIRE_FLUSH: u8 = 1;
/// Submit a batch under the write client.
pub const LOCK_ACT_SUBMIT: u8 = 2;
/// Publish after a successful submit (WAL Ok).
pub const LOCK_ACT_PUBLISH: u8 = 3;

/// One step of the N=2 alphabet: acquire-write / acquire-flush / submit /
/// publish. Submit requires the write client; publish requires a prior
/// submit. Flush is a separate mutex (legal next to write).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn lock_alphabet_step(holding_write: bool, submitted: bool, action: u8) -> bool {
    if action == LOCK_ACT_ACQUIRE_WRITE {
        !holding_write
    } else if action == LOCK_ACT_ACQUIRE_FLUSH {
        true
    } else if action == LOCK_ACT_SUBMIT {
        holding_write
    } else if action == LOCK_ACT_PUBLISH {
        submitted
    } else {
        false
    }
}

/// `∀ σ, n≤2 → plan(σ) = linearization(σ)` over the four-action alphabet.
/// Idle start; two actions `a0` then `a1`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn lock_alphabet_linearizes_n2(a0: u8, a1: u8) -> bool {
    if !lock_alphabet_step(false, false, a0) {
        return false;
    }
    let holding_write = a0 == LOCK_ACT_ACQUIRE_WRITE;
    let submitted = a0 == LOCK_ACT_SUBMIT;
    lock_alphabet_step(holding_write, submitted, a1)
}

/// AS-IS: every pair linearizes (admits publish-before-submit).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn lock_alphabet_linearizes_n2_as_is(_a0: u8, _a1: u8) -> bool {
    true
}

/// RFC-0229 P1.2: three steps of the same lock-client alphabet (not Linux
/// futex). Extra handler step = publish after acquire-write then submit.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn lock_alphabet_linearizes_n3(a0: u8, a1: u8, a2: u8) -> bool {
    if !lock_alphabet_step(false, false, a0) {
        return false;
    }
    let holding_write = a0 == LOCK_ACT_ACQUIRE_WRITE;
    let submitted = a0 == LOCK_ACT_SUBMIT;
    if !lock_alphabet_step(holding_write, submitted, a1) {
        return false;
    }
    let holding_write = holding_write || a1 == LOCK_ACT_ACQUIRE_WRITE;
    let submitted = submitted || a1 == LOCK_ACT_SUBMIT;
    lock_alphabet_step(holding_write, submitted, a2)
}

/// AS-IS: every triple linearizes (admits publish-before-submit).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn lock_alphabet_linearizes_n3_as_is(_a0: u8, _a1: u8, _a2: u8) -> bool {
    true
}

/// RFC-0229 P1.1: who owns the write-group wait wake.
#[cfg(not(verus_keep_ghost))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteGroupWait {
    /// PCT/World turnstile grants the next run.
    HarnessGrant,
    /// `parking_lot` / mpsc / `thread::sleep` (OS park).
    OsPark,
}

/// Production: harness owns the wake iff a PCT worker is bound.
/// AS-IS always reports OsPark (the 0229 hole — std park even under PCT).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn write_group_wait_grant(harness_owns: bool) -> WriteGroupWait {
    if harness_owns {
        WriteGroupWait::HarnessGrant
    } else {
        WriteGroupWait::OsPark
    }
}

/// AS-IS: every wait is an OS park (harness never owns the wake).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn write_group_wait_grant_as_is(_harness_owns: bool) -> WriteGroupWait {
    WriteGroupWait::OsPark
}

/// The wait is a legal lock-client step only after acquire-write then
/// submit (the follower/leader collect sits on that path). Unfolds both
/// the grant token and the N=2 alphabet.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn write_group_wait_grant_linearizes(harness_owns: bool) -> bool {
    match write_group_wait_grant(harness_owns) {
        WriteGroupWait::HarnessGrant | WriteGroupWait::OsPark => {
            lock_alphabet_linearizes_n2(LOCK_ACT_ACQUIRE_WRITE, LOCK_ACT_SUBMIT)
        }
    }
}

/// AS-IS: wait linearizes even without acquire-write then submit.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn write_group_wait_grant_linearizes_as_is(_harness_owns: bool) -> bool {
    true
}

/// Step 4: admitted **on this alphabet** iff the pair linearizes.
/// Not `lock_interleavings_admitted()` (OS/futex — always false).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn lock_alphabet_interleavings_admitted(a0: u8, a1: u8) -> bool {
    lock_alphabet_linearizes_n2(a0, a1)
}

/// RFC-0078 / R-fsync-lie: promote pending bytes only when the OS (or Env)
/// is honest. A lying `fsync` Ok must not make the write crash-durable.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn fsync_promotes_pending(os_honest: bool) -> bool {
    fsync_promotes_pending_body!(os_honest)
}

/// AS-IS: fsync Ok always promotes (the 0078 hole — Lying recovers).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn fsync_promotes_pending_as_is(_os_honest: bool) -> bool {
    fsync_promotes_pending_as_is_body!(_os_honest)
}

/// `fdatasync` rc==0 is not a proof the drive stored the bytes (R-fsync-lie).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn media_durable_admitted(_fsync_ok: bool) -> bool {
    media_durable_admitted_body!(_fsync_ok)
}

/// AS-IS: rc==0 is rounded to a media theorem (the 0078 hole).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn media_durable_admitted_as_is(fsync_ok: bool) -> bool {
    media_durable_admitted_as_is_body!(fsync_ok)
}

/// RFC-0078 P1.2 / RFC-0052: `RecordingEnv::Lying` and det_io PRELOAD
/// are two fsync-liar boxes. Stacking them in one process is not a
/// campaign. Always refuse.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn stacked_fsync_liars_admitted(_lying: bool, _det_io: bool) -> bool {
    stacked_fsync_liars_admitted_body!(_lying, _det_io)
}

/// AS-IS: AND both liar boxes in one run (the 0052 hole).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn stacked_fsync_liars_admitted_as_is(lying: bool, det_io: bool) -> bool {
    stacked_fsync_liars_admitted_as_is_body!(lying, det_io)
}

/// RFC-0078 P2.2: closing the lying-fsync model does not invent a TCG
/// guest (`R-tcg-guest` stays 0079). Always false.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn fsync_lie_closes_tcg_guest() -> bool {
    fsync_lie_closes_tcg_guest_body!()
}

/// AS-IS: 0078 is rounded to TCG guest coverage (the hole).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn fsync_lie_closes_tcg_guest_as_is() -> bool {
    fsync_lie_closes_tcg_guest_as_is_body!()
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn rwlock_client_may_mutate_spec(holding_write: bool) -> bool {
    holding_write
}

pub fn rwlock_client_may_mutate(holding_write: bool) -> (ok: bool)
    ensures
        ok == rwlock_client_may_mutate_spec(holding_write),
        holding_write ==> ok,
        !holding_write ==> !ok,
{
    rwlock_client_may_mutate_body!(holding_write)
}

pub fn rwlock_client_may_mutate_as_is(_holding_write: bool) -> (ok: bool)
    ensures
        ok == true,
{
    rwlock_client_may_mutate_as_is_body!(_holding_write)
}

pub open spec fn rwlock_client_may_read_spec(holding_read: bool, holding_write: bool) -> bool {
    holding_read || rwlock_client_may_mutate_spec(holding_write)
}

pub fn rwlock_client_may_read(holding_read: bool, holding_write: bool) -> (ok: bool)
    ensures
        ok == rwlock_client_may_read_spec(holding_read, holding_write),
        holding_read ==> ok,
        !holding_read ==> ok == rwlock_client_may_mutate_spec(holding_write),
{
    rwlock_client_may_read_body!(holding_read, holding_write)
}

pub fn rwlock_client_may_read_as_is(_holding_read: bool, _holding_write: bool) -> (ok: bool)
    ensures
        ok == true,
{
    rwlock_client_may_read_as_is_body!(_holding_read, _holding_write)
}

pub open spec fn may_publish_group_spec(wal_io_ok: bool) -> bool {
    wal_io_ok
}

pub fn may_publish_group(wal_io_ok: bool) -> (ok: bool)
    ensures
        ok == may_publish_group_spec(wal_io_ok),
{
    may_publish_group_body!(wal_io_ok)
}

pub fn may_publish_group_as_is(_wal_io_ok: bool) -> (ok: bool)
    ensures
        ok == true,
{
    may_publish_group_as_is_body!(_wal_io_ok)
}

pub fn lock_interleavings_admitted() -> (ok: bool)
    ensures
        ok == false,
{
    lock_interleavings_admitted_body!()
}

pub fn lock_interleavings_admitted_as_is() -> (ok: bool)
    ensures
        ok == true,
{
    lock_interleavings_admitted_as_is_body!()
}

pub fn forall_schedules_admitted(_pct_depth: u64) -> (ok: bool)
    ensures
        ok == false,
{
    forall_schedules_admitted_body!(_pct_depth)
}

pub fn forall_schedules_admitted_as_is(pct_depth: u64) -> (ok: bool)
    ensures
        ok == (pct_depth >= 2),
{
    forall_schedules_admitted_as_is_body!(pct_depth)
}

pub fn fsync_promotes_pending(os_honest: bool) -> (ok: bool)
    ensures
        ok == os_honest,
{
    fsync_promotes_pending_body!(os_honest)
}

pub fn fsync_promotes_pending_as_is(_os_honest: bool) -> (ok: bool)
    ensures
        ok == true,
{
    fsync_promotes_pending_as_is_body!(_os_honest)
}

pub fn media_durable_admitted(_fsync_ok: bool) -> (ok: bool)
    ensures
        ok == false,
{
    media_durable_admitted_body!(_fsync_ok)
}

pub fn media_durable_admitted_as_is(fsync_ok: bool) -> (ok: bool)
    ensures
        ok == fsync_ok,
{
    media_durable_admitted_as_is_body!(fsync_ok)
}

pub fn stacked_fsync_liars_admitted(_lying: bool, _det_io: bool) -> (ok: bool)
    ensures
        ok == false,
{
    stacked_fsync_liars_admitted_body!(_lying, _det_io)
}

pub fn stacked_fsync_liars_admitted_as_is(lying: bool, det_io: bool) -> (ok: bool)
    ensures
        ok == (lying && det_io),
{
    stacked_fsync_liars_admitted_as_is_body!(lying, det_io)
}

pub fn fsync_lie_closes_tcg_guest() -> (ok: bool)
    ensures
        ok == false,
{
    fsync_lie_closes_tcg_guest_body!()
}

pub fn fsync_lie_closes_tcg_guest_as_is() -> (ok: bool)
    ensures
        ok == true,
{
    fsync_lie_closes_tcg_guest_as_is_body!()
}

pub fn pct_campaign_default_depth() -> (d: u64)
    ensures
        d == 2,
{
    pct_campaign_default_depth_body!()
}

pub fn pct_campaign_default_depth_as_is() -> (d: u64)
    ensures
        d == 3,
{
    pct_campaign_default_depth_as_is_body!()
}

pub fn default_pct_depth_raised() -> (ok: bool)
    ensures
        ok == false,
{
    default_pct_depth_raised_body!()
}

pub fn default_pct_depth_raised_as_is() -> (ok: bool)
    ensures
        ok == true,
{
    default_pct_depth_raised_as_is_body!()
}

pub fn pct_chain3_row_is_plant() -> (ok: bool)
    ensures
        ok == true,
{
    pct_chain3_row_is_plant_body!()
}

pub fn pct_chain3_row_is_plant_as_is() -> (ok: bool)
    ensures
        ok == false,
{
    pct_chain3_row_is_plant_as_is_body!()
}

#[derive(PartialEq, Eq, Copy, Clone)]
pub enum OccMemberFate {
    Ok,
    TooOld,
    Conflict,
}

pub open spec fn occ_member_fate_spec(too_old: bool, conflict: bool) -> OccMemberFate {
    if too_old {
        OccMemberFate::TooOld
    } else if conflict {
        OccMemberFate::Conflict
    } else {
        OccMemberFate::Ok
    }
}

pub fn occ_member_fate(too_old: bool, conflict: bool) -> (d: OccMemberFate)
    ensures
        d == occ_member_fate_spec(too_old, conflict),
{
    occ_member_fate_body!(too_old, conflict)
}

pub fn occ_member_fate_as_is(_too_old: bool, _conflict: bool) -> (d: OccMemberFate)
    ensures
        d == OccMemberFate::Ok,
{
    occ_member_fate_as_is_body!(_too_old, _conflict)
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct OccRead {
    pub snap: u64,
    pub touched_key_written_after: bool,
}

pub open spec fn occ_conflict_spec(
    snap: u64,
    last_seq: u64,
    touched_key_written_after: bool,
) -> bool {
    last_seq > snap && touched_key_written_after
}

#[verifier::when_used_as_spec(occ_conflict_spec)]
pub fn occ_conflict(snap: u64, last_seq: u64, touched_key_written_after: bool) -> (c: bool)
    ensures
        c == occ_conflict_spec(snap, last_seq, touched_key_written_after),
{
    occ_conflict_body!(snap, last_seq, touched_key_written_after)
}

pub open spec fn occ_conflict_as_is_serialized_spec(
    snap: u64,
    last_seq: u64,
    writes_before: u64,
    touched_key_written_after: bool,
) -> bool {
    last_seq + writes_before > snap && touched_key_written_after
}

pub fn occ_conflict_as_is_serialized(
    snap: u64,
    last_seq: u64,
    writes_before: u64,
    touched_key_written_after: bool,
) -> (c: bool)
    requires
        last_seq + writes_before <= 0xffff_ffff_ffff_ffff,
    ensures
        c == occ_conflict_as_is_serialized_spec(
            snap,
            last_seq,
            writes_before,
            touched_key_written_after,
        ),
{
    occ_conflict_as_is_serialized_body!(
        snap,
        last_seq,
        writes_before,
        touched_key_written_after
    )
}

pub open spec fn occ_batch_plan_spec(
    too_old: &[bool],
    reads: &[OccRead],
    last_seq: u64,
) -> Seq<OccMemberFate> {
    let n = if too_old@.len() <= reads@.len() {
        too_old@.len()
    } else {
        reads@.len()
    };
    Seq::new(
        n,
        |i: int|
            if 0 <= i < too_old@.len() && i < reads@.len() {
                occ_member_fate_spec(
                    too_old[i],
                    occ_conflict_spec(
                        reads[i].snap,
                        last_seq,
                        reads[i].touched_key_written_after,
                    ),
                )
            } else {
                OccMemberFate::Ok
            },
    )
}

pub fn occ_batch_plan(
    too_old: &[bool],
    reads: &[OccRead],
    last_seq: u64,
) -> (out: Vec<OccMemberFate>)
    ensures
        out@ == occ_batch_plan_spec(too_old, reads, last_seq),
{
    let n: usize = if too_old.len() <= reads.len() {
        too_old.len()
    } else {
        reads.len()
    };
    let mut out: Vec<OccMemberFate> = Vec::new();
    let mut i: usize = 0;
    while i < n
        invariant
            0 <= i <= n,
            n <= too_old.len(),
            n <= reads.len(),
            n == (if too_old@.len() <= reads@.len() {
                too_old@.len()
            } else {
                reads@.len()
            }),
            out.len() == i,
            forall|j: int|
                0 <= j < i ==> out[j] == occ_member_fate_spec(
                    too_old[j],
                    occ_conflict_spec(
                        reads[j].snap,
                        last_seq,
                        reads[j].touched_key_written_after,
                    ),
                ),
        decreases n - i,
    {
        let conflict = last_seq > reads[i].snap && reads[i].touched_key_written_after;
        out.push(occ_member_fate(too_old[i], conflict));
        i += 1;
    }
    proof {
        assert(out@ == occ_batch_plan_spec(too_old, reads, last_seq));
    }
    out
}

pub fn occ_batch_plan_as_is(
    too_old: &[bool],
    reads: &[OccRead],
    _last_seq: u64,
) -> (out: Vec<OccMemberFate>)
    ensures
        out.len() == (if too_old.len() <= reads.len() {
            too_old.len()
        } else {
            reads.len()
        }),
        forall|j: int| 0 <= j < out.len() ==> out[j] == OccMemberFate::Ok,
{
    let n: usize = if too_old.len() <= reads.len() {
        too_old.len()
    } else {
        reads.len()
    };
    let mut out: Vec<OccMemberFate> = Vec::new();
    let mut i: usize = 0;
    while i < n
        invariant
            0 <= i <= n,
            n <= too_old.len(),
            n <= reads.len(),
            out.len() == i,
            forall|j: int| 0 <= j < i ==> out[j] == OccMemberFate::Ok,
        decreases n - i,
    {
        let _ = (too_old[i], reads[i]);
        out.push(OccMemberFate::Ok);
        i += 1;
    }
    out
}

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
        out.push(last_seq > reads[i].snap && reads[i].touched_key_written_after);
        i += 1;
    }
    out
}

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

pub fn fence_publish_seq_as_is(member_seqs: &[u64]) -> (p: u64)
    ensures
        p == (if member_seqs.len() == 0 {
            0
        } else {
            member_seqs[0]
        }),
{
    if member_seqs.len() == 0 {
        0
    } else {
        member_seqs[0]
    }
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn group_ack_plan_on_live_io_fail_fences() {
        // RFC-0219 P2.1: the group acks/publishes EXACTLY when its WAL
        // I/O succeeded; I/O failure fences (no publish, no Ok). AS-IS
        // acks the failure (Ok with a lie — dente).
        assert_eq!(group_ack_plan(true), GroupAckPlan::AckPublishGroup);
        assert_eq!(group_ack_plan(false), GroupAckPlan::FenceRefuseIoFail);
        assert_eq!(
            group_ack_plan_as_is(false),
            GroupAckPlan::AckPublishGroup,
            "AS-IS dente: acks a failed WAL I/O"
        );
        let lsc = named_fn_src(include_str!("db_kernel.rs"), "lone_sync_commit")
            .expect("lone_sync_commit");
        assert!(
            lsc.contains("match crate::group_commit_kernel::group_ack_plan("),
            "lone_sync_commit must match group_ack_plan"
        );
        assert!(
            !lsc.contains("may_publish_group("),
            "the raw publish gate left the trampoline"
        );
    }

    /// RFC-0157 P1.2 — property sweep over the pure group-commit kernel
    /// family (deterministic seeded trials; the recorded trial IS the
    /// shrunk counterexample). Pins: `occ_conflict` == non-empty window
    /// AND touched; `group_validate` is position-independent
    /// (simultaneity); `fence_publish_seq` == max member seq; the
    /// `may_publish_group` AS-IS mutant diverges exactly when WAL I/O
    /// failed (publish without durability).
    #[test]
    fn rfc0157_property_sweep_group_commit_kernel() {
        use crate::{Rng, SeedRng};
        // Exhaustive boolean cases first.
        assert_eq!(may_publish_group(true), true);
        assert_ne!(
            may_publish_group(false),
            may_publish_group_as_is(false),
            "AS-IS publish mutant must diverge at wal_io_ok=false"
        );
        assert_eq!(
            may_publish_group(true),
            may_publish_group_as_is(true),
            "both publish when WAL I/O succeeded"
        );
        // The RFC-0051 plant shape stays reachable in the pure kernel:
        // serialized scheduling conflicts where the group does not.
        assert!(
            occ_conflict_as_is_serialized(10, 10, 1, true) && !occ_conflict(10, 10, true),
            "AS-IS serialized mutant must keep the intra-group tooth"
        );

        let mut viol: Option<String> = None;
        'trials: for trial in 0..20_000u64 {
            let rng = SeedRng::new(0x0157_5712 ^ trial);
            let last_seq = rng.gen_range(64);
            let n = 1 + (rng.gen_range(6) as usize);
            let mut reads = Vec::with_capacity(n);
            for _ in 0..n {
                reads.push(OccRead {
                    snap: rng.gen_range(64),
                    touched_key_written_after: rng.gen_range(2) == 0,
                });
            }
            let mut seqs = Vec::with_capacity(n);
            for _ in 0..n {
                seqs.push(rng.gen_range(64));
            }
            for r in &reads {
                let expect = last_seq > r.snap && r.touched_key_written_after;
                if occ_conflict(r.snap, last_seq, r.touched_key_written_after) != expect {
                    viol = Some(format!(
                        "trial={trial} occ_conflict(snap={}, last_seq={}, touched={})",
                        r.snap, last_seq, r.touched_key_written_after
                    ));
                    break 'trials;
                }
            }
            let flags = group_validate(&reads, last_seq);
            for i in 0..n {
                let alone =
                    occ_conflict(reads[i].snap, last_seq, reads[i].touched_key_written_after);
                if flags[i] != alone {
                    viol = Some(format!(
                        "trial={trial} member {i} flag {} != alone {alone} (group not simultaneous)",
                        flags[i]
                    ));
                    break 'trials;
                }
            }
            let fold_max = seqs.iter().copied().fold(0u64, u64::max);
            if fence_publish_seq(&seqs) != fold_max {
                viol = Some(format!("trial={trial} fence != max of {seqs:?}"));
                break 'trials;
            }
            let wal_io_ok = rng.gen_range(2) == 0;
            if may_publish_group(wal_io_ok) != wal_io_ok {
                viol = Some(format!("trial={trial} may_publish_group({wal_io_ok})"));
                break 'trials;
            }
        }
        assert_eq!(viol, None, "rfc0157 sweep counterexample: {viol:?}");
    }

    #[test]
    fn fast_path_same_seq_never_conflicts() {
        assert!(!occ_conflict(7, 7, true));
        assert!(!occ_conflict(0, 0, true));
    }

    #[test]
    fn conflict_needs_window_and_touched_write() {
        assert!(occ_conflict(7, 9, true));
        assert!(!occ_conflict(7, 9, false));
        // Empty window (last_seq <= snap): nothing can be inside it.
        assert!(!occ_conflict(9, 7, true));
    }

    #[test]
    fn group_members_are_simultaneous() {
        // Two members of one group both touched the same key with
        // snapshots equal to last_seq: no conflict either way (the
        // group's own writes do not exist at validation time).
        let reads = [
            OccRead {
                snap: 10,
                touched_key_written_after: false,
            },
            OccRead {
                snap: 10,
                touched_key_written_after: false,
            },
        ];
        assert_eq!(group_validate(&reads, 10), vec![false, false]);
        // The serialized mutant aborts the second member.
        assert!(occ_conflict_as_is_serialized(10, 10, 1, true));
        let n3 = [
            OccRead {
                snap: 10,
                touched_key_written_after: true,
            },
            OccRead {
                snap: 10,
                touched_key_written_after: true,
            },
            OccRead {
                snap: 7,
                touched_key_written_after: true,
            },
        ];
        assert_eq!(
            group_validate(&n3, 10),
            vec![false, false, true],
            "N-way: only the lagging member conflicts"
        );
    }

    #[test]
    fn occ_member_fate_on_live_conflict_is_not_ok() {
        assert_eq!(occ_member_fate(false, true), OccMemberFate::Conflict);
        assert_eq!(occ_member_fate(true, true), OccMemberFate::TooOld);
        assert_eq!(occ_member_fate(false, false), OccMemberFate::Ok);
        assert_eq!(
            occ_member_fate_as_is(true, true),
            OccMemberFate::Ok,
            "AS-IS dente: lagging member still Ok"
        );
        let src = include_str!("concurrent_kernel.rs");
        assert!(
            src.contains("occ_batch_plan("),
            "validate_occ_batch must match occ_batch_plan"
        );
        let lone = src.split("fn lone_commit").nth(1).expect("lone_commit");
        assert!(
            lone.contains("occ_batch_plan("),
            "lone_commit must match occ_batch_plan"
        );
        assert!(
            lone.contains("occ_conflict("),
            "lone_commit must match occ_conflict"
        );
    }

    #[test]
    fn occ_batch_plan_n3_one_lagging_is_not_ok() {
        let too_old = [false, false, false];
        let reads = [
            OccRead {
                snap: 10,
                touched_key_written_after: true,
            },
            OccRead {
                snap: 10,
                touched_key_written_after: true,
            },
            OccRead {
                snap: 7,
                touched_key_written_after: true,
            },
        ];
        assert_eq!(
            occ_batch_plan(&too_old, &reads, 10),
            vec![
                OccMemberFate::Ok,
                OccMemberFate::Ok,
                OccMemberFate::Conflict
            ],
            "N-way: only the lagging member conflicts"
        );
        assert_eq!(
            occ_batch_plan_as_is(&too_old, &reads, 10),
            vec![OccMemberFate::Ok, OccMemberFate::Ok, OccMemberFate::Ok],
            "AS-IS dente: lagging member still Ok"
        );
        let validate = include_str!("concurrent_kernel.rs")
            .split("fn validate_occ_batch")
            .nth(1)
            .expect("validate_occ_batch");
        assert!(
            validate.contains("occ_batch_plan("),
            "validate_occ_batch must match occ_batch_plan"
        );
    }

    #[test]
    fn occ_batch_plan_on_live_lagging_is_not_ok() {
        let too_old = [false, true];
        let reads = [
            OccRead {
                snap: 10,
                touched_key_written_after: true,
            },
            OccRead {
                snap: 7,
                touched_key_written_after: true,
            },
        ];
        assert_eq!(
            occ_batch_plan(&too_old, &reads, 10),
            vec![OccMemberFate::Ok, OccMemberFate::TooOld]
        );
        let lag = [false];
        let lag_read = [OccRead {
            snap: 7,
            touched_key_written_after: true,
        }];
        assert_eq!(
            occ_batch_plan(&lag, &lag_read, 10),
            vec![OccMemberFate::Conflict]
        );
        assert_eq!(
            occ_batch_plan_as_is(&lag, &lag_read, 10),
            vec![OccMemberFate::Ok],
            "AS-IS dente: lagging member still Ok"
        );
        let src = include_str!("concurrent_kernel.rs");
        let validate = src
            .split("fn validate_occ_batch")
            .nth(1)
            .expect("validate_occ_batch");
        assert!(
            validate.contains("occ_batch_plan("),
            "validate_occ_batch must match occ_batch_plan"
        );
        let lone = src.split("fn lone_commit").nth(1).expect("lone_commit");
        assert!(
            lone.contains("occ_batch_plan("),
            "lone_commit must match occ_batch_plan"
        );
        assert!(
            lone.contains("occ_conflict("),
            "lone_commit must match occ_conflict"
        );
    }

    #[test]
    fn rwlock_client_may_mutate_on_live_off_lock_is_not_ok() {
        assert!(rwlock_client_may_mutate(true));
        assert!(!rwlock_client_may_mutate(false));
        assert!(
            rwlock_client_may_mutate_as_is(false),
            "AS-IS dente: mutate after dropping the write lock"
        );
        let src = include_str!("concurrent_kernel.rs");
        let off = src
            .split("fn finish_group_off_lock")
            .nth(1)
            .expect("finish_group_off_lock");
        assert!(
            off.contains("drop(guard)"),
            "off-lock fd drops the write guard"
        );
        assert!(
            off.contains("rwlock_client_may_mutate("),
            "finish_group_off_lock must match the data-race token"
        );
        let after_drop = off.split("drop(guard)").nth(1).expect("after drop");
        let until_reacquire = after_drop.split("db.write()").next().expect("until write");
        assert!(
            !until_reacquire.contains("group_apply("),
            "must not apply mem while the write lock is dropped"
        );
        assert!(
            !until_reacquire.contains("publish_sequence("),
            "must not publish while the write lock is dropped"
        );
    }

    #[test]
    fn rwlock_client_may_read_on_no_guard_is_not_ok() {
        assert!(
            !rwlock_client_may_read(false, false),
            "no guard ⇒ cannot read last_seq"
        );
        assert!(rwlock_client_may_read(true, false));
        assert!(rwlock_client_may_read(false, true));
        assert!(rwlock_client_may_read(true, true));
        assert!(
            rwlock_client_may_read_as_is(false, false),
            "AS-IS dente: read Db with no guard"
        );
        let snap = include_str!("concurrent_kernel.rs")
            .split("fn occ_snapshot(")
            .nth(1)
            .expect("occ_snapshot");
        assert!(
            snap.contains("rwlock_client_may_read("),
            "occ_snapshot must match the reader token"
        );
    }

    /// Catalog three-teeth plant. Direct `group_members_are_simultaneous` is **not** this tooth.
    #[test]
    fn occ_conflict_on_live_group_is_not_ok() {
        assert!(!occ_conflict(10, 10, true));
        assert!(
            occ_conflict_as_is_serialized(10, 10, 1, true),
            "AS-IS dente: serialized scheduler aborts the second intra-group member"
        );
        let dir = std::env::temp_dir().join(format!(
            "group-commit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let db = crate::ConcurrentDb::open_with(
            &dir,
            crate::OpenOptions {
                exclusive: true,
                ..crate::OpenOptions::default()
            },
        )
        .unwrap();
        db.put(b"k", b"v0").unwrap();
        let mut tx1 = db.begin_occ();
        let mut tx2 = db.begin_occ();
        assert_eq!(tx1.get(b"k").unwrap().as_deref(), Some(b"v0".as_ref()));
        assert_eq!(tx2.get(b"k").unwrap().as_deref(), Some(b"v0".as_ref()));
        tx1.put(b"k", b"from1").unwrap();
        tx2.put(b"k", b"from2").unwrap();
        tx1.commit().unwrap();
        let err = tx2.commit().unwrap_err();
        assert!(
            matches!(err, crate::CoreError::TransactionConflict),
            "live ConcurrentDb first-committer-wins must conflict the lagging OCC commit, got {err:?}"
        );
        assert_eq!(
            db.get(b"k").as_deref(),
            Some(b"from1".as_ref()),
            "live lone/group OCC path keeps the first committer"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fence_is_max_member_seq() {
        assert_eq!(fence_publish_seq(&[]), 0);
        assert_eq!(fence_publish_seq(&[3]), 3);
        assert_eq!(fence_publish_seq(&[5, 2, 9, 4]), 9);
        assert_eq!(fence_publish_seq(&[0, 0]), 0);
    }

    /// Catalog three-teeth plant. Direct `fence_is_max_member_seq` is **not** this tooth.
    #[test]
    fn fence_publish_seq_on_live_group_is_not_ok() {
        assert_eq!(fence_publish_seq(&[5, 2, 9, 4]), 9);
        assert_eq!(
            fence_publish_seq_as_is(&[5, 2, 9, 4]),
            5,
            "AS-IS dente: fence is the first member, later seqs stay unpublished"
        );
        let dir = std::env::temp_dir().join(format!(
            "group-fence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let db = std::sync::Arc::new(
            crate::ConcurrentDb::open_with(
                &dir,
                crate::OpenOptions {
                    exclusive: true,
                    ..crate::OpenOptions::default()
                },
            )
            .unwrap(),
        );
        db.set_write_group_catchup_window(std::time::Duration::from_millis(20));
        db.put(b"warm", b"1").unwrap();
        let n = 8usize;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(n));
        let mut handles = Vec::new();
        for i in 0..n {
            let db = std::sync::Arc::clone(&db);
            let barrier = std::sync::Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                let k = [b'k', u8::try_from(i).expect("n fits u8")];
                db.put(&k, b"v")
            }));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(
            results.iter().all(|r| r.is_ok()),
            "every group member must Ok: {results:?}"
        );
        let (submits, _queued, groups, group_ops) = db.write_group_stats();
        assert_eq!(submits, n as u64 + 1, "warm + {n} grouped puts");
        assert!(
            groups < n as u64 && group_ops >= 2,
            "must have taken max_appended_seq group path groups={groups} ops={group_ops}"
        );
        assert_eq!(
            db.visible_sequence(),
            db.last_sequence(),
            "live fence must publish the max member seq, not the first"
        );
        for i in 0..n {
            let k = [b'k', u8::try_from(i).expect("n fits u8")];
            assert_eq!(
                db.get(&k).as_deref(),
                Some(b"v".as_ref()),
                "live get after group Ok must see member {i}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rfc0229_write_group_wait_grant_vs_as_is() {
        assert_eq!(write_group_wait_grant(true), WriteGroupWait::HarnessGrant);
        assert_eq!(write_group_wait_grant(false), WriteGroupWait::OsPark);
        assert_eq!(
            write_group_wait_grant_as_is(true),
            WriteGroupWait::OsPark,
            "AS-IS dente: harness wait still OS-parks"
        );
        assert!(write_group_wait_grant_linearizes(true));
        assert!(write_group_wait_grant_linearizes(false));
        assert!(write_group_wait_grant_linearizes_as_is(false));
        assert!(!lock_interleavings_admitted());
        let prod = include_str!("concurrent_kernel.rs")
            .split("\nmod tests {")
            .next()
            .expect("production");
        assert!(
            prod.contains("write_group_wait_grant("),
            "production write-group wait must match the grant token"
        );
        assert!(
            prod.contains("WriteGroupWait::OsPark"),
            "OS park arm must stay on the rustc path"
        );
        assert!(
            prod.contains("WriteGroupWait::HarnessGrant"),
            "harness grant arm must stay on the rustc path"
        );
    }

    #[test]
    fn rfc0229_pct_chain3_row_is_plant_not_theorem() {
        assert!(
            pct_chain3_row_is_plant(),
            "RFC-0220 P2.3 / RFC-0229 P0.3: chain-3 is a plant"
        );
        assert!(
            !pct_chain3_row_is_plant_as_is(),
            "AS-IS dente: round the plant to a ∀ theorem"
        );
        assert!(
            !forall_schedules_admitted(3),
            "finding the plant at d=3 is not ∀ OS schedules"
        );
        assert_eq!(pct_campaign_default_depth(), 2);
    }

    #[test]
    fn pct_depth_is_not_forall_schedules() {
        assert!(!forall_schedules_admitted(0));
        assert!(!forall_schedules_admitted(2));
        assert!(!forall_schedules_admitted(3));
        assert!(!forall_schedules_admitted_as_is(0));
        assert!(forall_schedules_admitted_as_is(2));
        assert!(forall_schedules_admitted_as_is(3));
        assert_eq!(pct_campaign_default_depth(), 2);
        assert_eq!(
            pct_campaign_default_depth_as_is(),
            3,
            "AS-IS dente: 0070 would raise default PCT depth"
        );
        assert!(!default_pct_depth_raised());
        assert!(
            default_pct_depth_raised_as_is(),
            "AS-IS dente: 0070 would claim it raised default depth"
        );
    }

    #[test]
    fn fsync_ok_is_not_media_proof() {
        assert!(fsync_promotes_pending(true));
        assert!(!fsync_promotes_pending(false));
        assert!(
            fsync_promotes_pending_as_is(false),
            "AS-IS dente: promote on a lying fsync"
        );
        assert!(!media_durable_admitted(true));
        assert!(!media_durable_admitted(false));
        assert!(
            media_durable_admitted_as_is(true),
            "AS-IS dente: fsync Ok proves the drive"
        );
        assert!(!media_durable_admitted_as_is(false));
        assert!(!stacked_fsync_liars_admitted(true, true));
        assert!(!stacked_fsync_liars_admitted(true, false));
        assert!(
            stacked_fsync_liars_admitted_as_is(true, true),
            "AS-IS dente: AND Lying × det_io in one run"
        );
        assert!(!stacked_fsync_liars_admitted_as_is(true, false));
        assert!(!fsync_lie_closes_tcg_guest());
        assert!(
            fsync_lie_closes_tcg_guest_as_is(),
            "AS-IS dente: 0078 would invent a TCG guest"
        );
    }

    #[test]
    fn publish_only_when_wal_io_ok() {
        assert!(may_publish_group(true));
        assert!(!may_publish_group(false));
        assert!(may_publish_group_as_is(false));
        assert!(may_publish_group_as_is(true));
    }

    #[test]
    fn lock_interleavings_are_not_a_theorem() {
        assert!(!lock_interleavings_admitted());
        assert!(
            lock_interleavings_admitted_as_is(),
            "AS-IS dente: admit ∀ lock schedules"
        );
    }

    #[test]
    fn lock_alphabet_n2_linearizes_write_then_submit() {
        assert!(lock_alphabet_linearizes_n2(
            LOCK_ACT_ACQUIRE_WRITE,
            LOCK_ACT_SUBMIT
        ));
        assert!(lock_alphabet_linearizes_n2(
            LOCK_ACT_ACQUIRE_WRITE,
            LOCK_ACT_ACQUIRE_FLUSH
        ));
        assert!(lock_alphabet_linearizes_n2(
            LOCK_ACT_ACQUIRE_FLUSH,
            LOCK_ACT_ACQUIRE_WRITE
        ));
        assert!(
            !lock_alphabet_linearizes_n2(LOCK_ACT_PUBLISH, LOCK_ACT_SUBMIT),
            "publish-before-submit is not a linearization"
        );
        assert!(
            !lock_alphabet_linearizes_n2(LOCK_ACT_SUBMIT, LOCK_ACT_PUBLISH),
            "submit without acquire-write is not a linearization"
        );
        assert!(
            lock_alphabet_linearizes_n2_as_is(LOCK_ACT_PUBLISH, LOCK_ACT_SUBMIT),
            "AS-IS dente: illegal order still linearizes"
        );
        assert!(lock_alphabet_interleavings_admitted(
            LOCK_ACT_ACQUIRE_WRITE,
            LOCK_ACT_SUBMIT
        ));
        assert!(!lock_alphabet_interleavings_admitted(
            LOCK_ACT_PUBLISH,
            LOCK_ACT_SUBMIT
        ));
        assert!(
            !lock_interleavings_admitted(),
            "OS/futex claim stays refused after alphabet admission"
        );
        assert!(!forall_schedules_admitted(2));
        assert!(!forall_schedules_admitted(3));
        assert!(
            lock_alphabet_linearizes_n3(LOCK_ACT_ACQUIRE_WRITE, LOCK_ACT_SUBMIT, LOCK_ACT_PUBLISH),
            "RFC-0229 P1.2: write then submit then publish is the extra handler step"
        );
        assert!(
            !lock_alphabet_linearizes_n3(LOCK_ACT_PUBLISH, LOCK_ACT_SUBMIT, LOCK_ACT_ACQUIRE_WRITE),
            "publish-before-submit still refused at N=3"
        );
        assert!(
            lock_alphabet_linearizes_n3_as_is(
                LOCK_ACT_PUBLISH,
                LOCK_ACT_SUBMIT,
                LOCK_ACT_ACQUIRE_WRITE
            ),
            "AS-IS dente: illegal triple still linearizes"
        );
        let rot = include_str!("db_kernel.rs")
            .split("fn try_rotate_wal(&mut self)")
            .nth(1)
            .and_then(|s| s.split("fn wal_pin_state").next())
            .expect("try_rotate_wal");
        assert!(
            rot.contains("wal_rotate_decision("),
            "try_rotate_wal matches wal_rotate_decision (passo 1 caller)"
        );
        let pin = include_str!("db_kernel.rs")
            .split("fn wal_pin_state(")
            .nth(1)
            .and_then(|s| s.split("fn ensure_wal_rotated_for_gc").next())
            .expect("wal_pin_state");
        assert!(
            pin.contains("commit_inflight:"),
            "rotate reads commit_inflight"
        );
        let put = include_str!("concurrent_put_kernel.rs");
        assert!(
            put.contains("match crate::group_commit_kernel::lock_alphabet_linearizes_n2("),
            "put_with_seq must match lock_alphabet_linearizes_n2"
        );
        assert!(
            put.contains("match crate::group_commit_kernel::lock_alphabet_linearizes_n3("),
            "put_with_seq must match lock_alphabet_linearizes_n3 (RFC-0229 P1.2)"
        );
        assert!(
            put.contains("submit_one("),
            "ConcurrentDb put submits (acquire-write/submit client)"
        );
        let lock = include_str!("../../rocksdb-compat/src/locktab_kernel.rs");
        assert!(
            lock.contains("wait_for_deadlock("),
            "2PL locktab calls wait_for_deadlock"
        );
    }

    #[test]
    fn pct_tsan_n3_is_not_os_forall_pi() {
        assert!(
            !forall_schedules_admitted(2),
            "PCT d=2 is a campaign, not ∀π OS"
        );
        assert!(!forall_schedules_admitted(3));
        assert!(
            forall_schedules_admitted_as_is(2),
            "AS-IS dente: PCT CLEAN as a theorem"
        );
        assert!(!lock_interleavings_admitted());
        assert!(lock_interleavings_admitted_as_is());
    }
}
