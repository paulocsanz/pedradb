// Verus twin of `src/l28.rs::l28_durability_ok` (RFC-0072 P2.1).
// Not linked into production.
//
//   ./scripts/verus_l28.sh

use vstd::prelude::*;

verus! {

pub open spec fn l28_durability_ok_spec(get_ok: bool, after_kill_ok: bool, restart_ok: bool) -> bool {
    get_ok && after_kill_ok && restart_ok
}

/// AS-IS: first get is enough (ignore kill/restart).
pub open spec fn l28_durability_ok_as_is_spec(get_ok: bool, _after_kill_ok: bool, _restart_ok: bool) -> bool {
    get_ok
}

pub fn l28_durability_ok(get_ok: bool, after_kill_ok: bool, restart_ok: bool) -> (ok: bool)
    ensures
        ok == l28_durability_ok_spec(get_ok, after_kill_ok, restart_ok),
{
    get_ok && after_kill_ok && restart_ok
}

pub fn l28_durability_ok_as_is(get_ok: bool, _after_kill_ok: bool, _restart_ok: bool) -> (ok: bool)
    ensures
        ok == l28_durability_ok_as_is_spec(get_ok, _after_kill_ok, _restart_ok),
{
    get_ok
}

pub open spec fn l28_leader_kill_ok_spec(get_ok: bool, after_kill_ok: bool, restart_ok: bool) -> bool {
    l28_durability_ok_spec(get_ok, after_kill_ok, restart_ok)
}

pub fn l28_leader_kill_ok(get_ok: bool, after_kill_ok: bool, restart_ok: bool) -> (ok: bool)
    ensures
        ok == l28_leader_kill_ok_spec(get_ok, after_kill_ok, restart_ok),
{
    l28_durability_ok(get_ok, after_kill_ok, restart_ok)
}

proof fn lemma_get_only_is_not_l28()
    ensures
        !l28_durability_ok_spec(true, false, true),
        l28_durability_ok_as_is_spec(true, false, false),
{
}

/// RFC-0121 P1.2 / 0066 P2.2: on-disk C-new-only after REAL TCP plant.
pub open spec fn l28_tcp_left_ok_spec(left: bool) -> bool {
    left
}

/// AS-IS: skip the on-disk scan.
pub open spec fn l28_tcp_left_ok_as_is_spec(_left: bool) -> bool {
    true
}

pub fn l28_tcp_left_ok(left: bool) -> (ok: bool)
    ensures
        ok == l28_tcp_left_ok_spec(left),
{
    left
}

pub fn l28_tcp_left_ok_as_is(_left: bool) -> (ok: bool)
    ensures
        ok == l28_tcp_left_ok_as_is_spec(_left),
{
    true
}

/// RFC-0126 P1.2: on-disk high-water after REAL TCP plant.
pub open spec fn l28_tcp_hw_ok_spec(kept: bool) -> bool {
    kept
}

pub open spec fn l28_tcp_hw_ok_as_is_spec(_kept: bool) -> bool {
    true
}

pub fn l28_tcp_hw_ok(kept: bool) -> (ok: bool)
    ensures
        ok == l28_tcp_hw_ok_spec(kept),
{
    kept
}

pub fn l28_tcp_hw_ok_as_is(_kept: bool) -> (ok: bool)
    ensures
        ok == l28_tcp_hw_ok_as_is_spec(_kept),
{
    true
}

/// RFC-0128 P1.2: removed voter is not participating after REAL TCP plant.
pub open spec fn l28_tcp_part_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_part_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_part_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_part_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_part_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_part_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0130 P1.2: recover apply closed `commit > applied` after REAL TCP plant.
pub open spec fn l28_tcp_apply_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_apply_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_apply_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_apply_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_apply_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_apply_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0131 P1.2: recover apply on a replica already dropped from `ids`.
pub open spec fn l28_tcp_napply_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_napply_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_napply_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_napply_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_napply_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_napply_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0155 P0: harness retries are not ∀ TCP traces. Always false.
pub open spec fn l28_tcp_napply_retry_admitted_spec(_attempts: u64, _napply_ok: bool) -> bool {
    false
}

pub open spec fn l28_tcp_napply_retry_admitted_as_is_spec(attempts: u64, napply_ok: bool) -> bool {
    attempts >= 1 && napply_ok
}

pub fn l28_tcp_napply_retry_admitted(_attempts: u64, _napply_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_napply_retry_admitted_spec(_attempts, _napply_ok),
{
    false
}

pub fn l28_tcp_napply_retry_admitted_as_is(attempts: u64, napply_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_napply_retry_admitted_as_is_spec(attempts, napply_ok),
{
    attempts >= 1 && napply_ok
}

/// RFC-0132 P1.2: truncate persist on a replica already dropped from `ids`.
pub open spec fn l28_tcp_trunc_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_trunc_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_trunc_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_trunc_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_trunc_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_trunc_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0133 P1.2: orphan log_entry_key drop on a replica dropped from `ids`.
pub open spec fn l28_tcp_odrop_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_odrop_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_odrop_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_odrop_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_odrop_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_odrop_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0134 P1.2: leftover 2PC abort on a replica dropped from `ids`.
pub open spec fn l28_tcp_abort_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_abort_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_abort_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_abort_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_abort_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_abort_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0135 P1.2: now_ms persist on a replica dropped from `ids`.
pub open spec fn l28_tcp_nowms_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_nowms_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_nowms_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_nowms_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_nowms_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_nowms_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0158 P2.1: durable-term rollback on the removed replica's REAL dir.
pub open spec fn l28_tcp_dterm_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_dterm_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_dterm_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_dterm_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_dterm_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_dterm_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0136 P1.2: SI hist persist on a replica dropped from `ids`.
pub open spec fn l28_tcp_hist_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_hist_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_hist_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_hist_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_hist_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_hist_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0137 P1.2: abort-fence persist on a replica dropped from `ids`.
pub open spec fn l28_tcp_fence_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_fence_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_fence_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_fence_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_fence_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_fence_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0138 P1.2: force-local TX clear on a replica dropped from `ids`.
pub open spec fn l28_tcp_clear_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_clear_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_clear_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_clear_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_clear_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_clear_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0139 P1.2: drop TX preimages on a replica dropped from `ids`.
pub open spec fn l28_tcp_pre_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_pre_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_pre_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_pre_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_pre_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_pre_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0140 P1.2: TCP ctor election timeout follows disk C-new.
pub open spec fn l28_tcp_peer_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_peer_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_peer_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_peer_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_peer_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_peer_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0141 P1.2: TCP ctor must not treat HashMap first-key as identity.
pub open spec fn l28_tcp_lid_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_lid_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_lid_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_lid_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_lid_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_lid_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0142 P1.2: TCP ctor must not pick remote ids.first as reader.
pub open spec fn l28_tcp_rdr_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_rdr_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_rdr_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_rdr_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_rdr_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_rdr_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0143 P1.2: live discard on a replica dropped from ids.
pub open spec fn l28_tcp_dsc_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_dsc_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_dsc_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_dsc_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_dsc_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_dsc_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0144 P1.2: no-leader persist-leader must be local.
pub open spec fn l28_tcp_pld_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_pld_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_pld_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_pld_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_pld_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_pld_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0145 P1.2: re-install of C-new steps a planted Leader down.
pub open spec fn l28_tcp_std_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_std_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_std_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_std_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_std_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_std_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0146 P1.2: remaining voter's leader_hint omits the removed replica.
pub open spec fn l28_tcp_hnt_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_hnt_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_hnt_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_hnt_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_hnt_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_hnt_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0147 P1.2: remaining voter forgets next/match/sent_through of the removed replica.
pub open spec fn l28_tcp_slot_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_slot_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_slot_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_slot_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_slot_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_slot_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0148 P1.2: remaining voter forgets sent_through of a remote replica on oob remove.
pub open spec fn l28_tcp_sth_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_sth_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_sth_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_sth_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_sth_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_sth_ok_as_is_spec(_ok),
{
    true
}

/// RFC-0068 P2.2: planted committed joint without leave refuses C-old majority.
pub open spec fn l28_tcp_pj_ok_spec(ok: bool) -> bool {
    ok
}

pub open spec fn l28_tcp_pj_ok_as_is_spec(_ok: bool) -> bool {
    true
}

pub fn l28_tcp_pj_ok(ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_pj_ok_spec(ok),
{
    ok
}

pub fn l28_tcp_pj_ok_as_is(_ok: bool) -> (d: bool)
    ensures
        d == l28_tcp_pj_ok_as_is_spec(_ok),
{
    true
}

fn main() {}

} // verus!
