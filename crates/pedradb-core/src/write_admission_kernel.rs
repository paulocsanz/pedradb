//! Write-admission + put-Ok path predicates (RFC-0170 P2.4 / RFC-0171 P0.3).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_write_admission.sh

#![forbid(unsafe_code)]

macro_rules! idle_body {
    ($mem:expr, $pressure:expr, $stall:expr) => {
        !$mem && !$pressure && !$stall
    };
}

macro_rules! idle_as_is_body {
    ($mem:expr, $pressure:expr, $stall:expr) => {{
        let _ = ($mem, $pressure, $stall);
        true
    }};
}

macro_rules! admit_body {
    ($mem_bytes:expr, $mem_armed:expr, $mem_limit:expr, $l0:expr, $l0_armed:expr, $l0_limit:expr) => {{
        if $mem_armed && $mem_bytes >= $mem_limit {
            WriteAdmit::StallMem
        } else if $l0_armed && $l0 >= $l0_limit {
            WriteAdmit::StallL0
        } else {
            WriteAdmit::Ok
        }
    }};
}

macro_rules! admit_as_is_body {
    ($a:expr, $b:expr, $c:expr, $d:expr, $e:expr, $f:expr) => {{
        let _ = ($a, $b, $c, $d, $e, $f);
        WriteAdmit::Ok
    }};
}

macro_rules! wal_sync_required_body {
    ($client_set:expr, $client_sync:expr, $db_sync:expr) => {
        if $client_set {
            $client_sync
        } else {
            $db_sync
        }
    };
}

macro_rules! seq_exhausted_body {
    ($seq:expr, $max:expr) => {
        $seq > $max
    };
}

macro_rules! batch_is_empty_body {
    ($n:expr) => {
        $n == 0u64
    };
}

macro_rules! fence_on_sync_fail_body {
    ($sync_required:expr, $sync_failed:expr) => {
        $sync_required && $sync_failed
    };
}

macro_rules! dir_sync_required_body {
    ($sync:expr) => {
        $sync
    };
}

macro_rules! torn_head_empty_log_body {
    ($len:expr, $tiny_max:expr) => {
        $len < $tiny_max
    };
}

macro_rules! torn_tail_needs_cut_body {
    ($len:expr, $last_good:expr) => {
        $len > $last_good
    };
}

/// Truncated(0) WAL smaller than this is an empty failed-first-append, not bitrot.
pub const TINY_WAL_EMPTY_MAX: u64 = 64;

/// Hard-admit verdict after the handler measured mem/L0 (drain is glue).
#[cfg(not(verus_keep_ghost))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteAdmit {
    /// Write may proceed.
    Ok,
    /// Mem bytes still at/above the armed mem stall limit.
    StallMem,
    /// L0 file count still at/above the armed L0 stall limit.
    StallL0,
}

#[cfg(not(verus_keep_ghost))]
/// True iff no write-stall knob is armed.
#[must_use]
pub fn write_admission_idle(mem_stall: bool, pressure_l0: bool, stall_l0: bool) -> bool {
    idle_body!(mem_stall, pressure_l0, stall_l0)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: always idle — stall knobs are decorative.
#[must_use]
pub fn write_admission_idle_as_is(mem_stall: bool, pressure_l0: bool, stall_l0: bool) -> bool {
    idle_as_is_body!(mem_stall, pressure_l0, stall_l0)
}

#[cfg(not(verus_keep_ghost))]
/// Mem axis first, then L0.
#[must_use]
pub fn write_admit(
    mem_bytes: u64,
    mem_armed: bool,
    mem_limit: u64,
    l0: u64,
    l0_armed: bool,
    l0_limit: u64,
) -> WriteAdmit {
    admit_body!(mem_bytes, mem_armed, mem_limit, l0, l0_armed, l0_limit)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: always admit.
#[must_use]
pub fn write_admit_as_is(
    mem_bytes: u64,
    mem_armed: bool,
    mem_limit: u64,
    l0: u64,
    l0_armed: bool,
    l0_limit: u64,
) -> WriteAdmit {
    admit_as_is_body!(mem_bytes, mem_armed, mem_limit, l0, l0_armed, l0_limit)
}

#[cfg(not(verus_keep_ghost))]
/// Whether this commit must `fdatasync` the WAL before Ok.
#[must_use]
pub fn wal_sync_required(client_set: bool, client_sync: bool, db_sync: bool) -> bool {
    wal_sync_required_body!(client_set, client_sync, db_sync)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never sync (acks without a barrier).
#[must_use]
pub fn wal_sync_required_as_is(_client_set: bool, _client_sync: bool, _db_sync: bool) -> bool {
    false
}

#[cfg(not(verus_keep_ghost))]
/// Sequence counter has no remaining values.
#[must_use]
pub fn seq_exhausted(seq: u64, max: u64) -> bool {
    seq_exhausted_body!(seq, max)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never exhaust (wrap / burn past the ceiling).
#[must_use]
pub fn seq_exhausted_as_is(_seq: u64, _max: u64) -> bool {
    false
}

#[cfg(not(verus_keep_ghost))]
/// Empty batch: no WAL record, return last sequence.
#[must_use]
pub fn batch_is_empty(n: u64) -> bool {
    batch_is_empty_body!(n)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: empty is treated as non-empty (would WAL-commit nothing / skip Ok).
#[must_use]
pub fn batch_is_empty_as_is(_n: u64) -> bool {
    false
}

#[cfg(not(verus_keep_ghost))]
/// Required sync failed ⇒ fence so later fsync cannot publish an unacked prefix.
#[must_use]
pub fn fence_on_sync_fail(sync_required: bool, sync_failed: bool) -> bool {
    fence_on_sync_fail_body!(sync_required, sync_failed)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never fence (H1 hole).
#[must_use]
pub fn fence_on_sync_fail_as_is(_sync_required: bool, _sync_failed: bool) -> bool {
    false
}

#[cfg(not(verus_keep_ghost))]
/// Truncated(0) on a tiny WAL is empty-log, not bitrot of the first record.
#[must_use]
pub fn torn_head_is_empty_log(len: u64, tiny_max: u64) -> bool {
    torn_head_empty_log_body!(len, tiny_max)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: every Truncated(0) is empty (bitrot of a large WAL is served).
#[must_use]
pub fn torn_head_is_empty_log_as_is(_len: u64, _tiny_max: u64) -> bool {
    true
}

#[cfg(not(verus_keep_ghost))]
/// WAL bytes past last-good must be cut before new appends.
#[must_use]
pub fn torn_tail_needs_cut(len: u64, last_good: u64) -> bool {
    torn_tail_needs_cut_body!(len, last_good)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never cut the torn tail (next open fail-stops on garbage).
#[must_use]
pub fn torn_tail_needs_cut_as_is(_len: u64, _last_good: u64) -> bool {
    false
}

#[cfg(not(verus_keep_ghost))]
/// WAL record newer than CHANGELOG max must be rebuilt into the feed.
#[must_use]
pub fn seq_after_feed(seq: u64, feed_max: u64) -> bool {
    seq_exhausted_body!(seq, feed_max)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never rebuild missing feed entries (crash gap stays silent).
#[must_use]
pub fn seq_after_feed_as_is(_seq: u64, _feed_max: u64) -> bool {
    false
}

#[cfg(not(verus_keep_ghost))]
/// PointInTime resync report ⇒ rewrite WAL from recovered prefix.
#[must_use]
pub fn pit_resync_needs_rewrite(is_resync: bool) -> bool {
    is_resync
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never rewrite (next fail-closed open sees mid-log damage).
#[must_use]
pub fn pit_resync_needs_rewrite_as_is(_is_resync: bool) -> bool {
    false
}

#[cfg(not(verus_keep_ghost))]
/// Open-options `sync` requires a directory fsync after rename/create
/// (CURRENT, MANIFEST, SST publish).
#[must_use]
pub fn dir_sync_required(sync: bool) -> bool {
    dir_sync_required_body!(sync)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never dir-fsync — the dentry of CURRENT/MANIFEST/SST can vanish
/// after a crash even though the file contents were durable.
#[must_use]
pub fn dir_sync_required_as_is(_sync: bool) -> bool {
    false
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WriteAdmit {
    Ok,
    StallMem,
    StallL0,
}

pub open spec fn write_admission_idle_spec(mem: bool, pressure: bool, stall: bool) -> bool {
    !mem && !pressure && !stall
}

pub fn write_admission_idle(mem_stall: bool, pressure_l0: bool, stall_l0: bool) -> (d: bool)
    ensures
        d == write_admission_idle_spec(mem_stall, pressure_l0, stall_l0),
{
    idle_body!(mem_stall, pressure_l0, stall_l0)
}

pub open spec fn write_admission_idle_as_is_spec(_mem: bool, _pressure: bool, _stall: bool) -> bool {
    true
}

pub fn write_admission_idle_as_is(mem_stall: bool, pressure_l0: bool, stall_l0: bool) -> (d: bool)
    ensures
        d == write_admission_idle_as_is_spec(mem_stall, pressure_l0, stall_l0),
        d == true,
{
    idle_as_is_body!(mem_stall, pressure_l0, stall_l0)
}

pub open spec fn write_admit_spec(
    mem_bytes: u64,
    mem_armed: bool,
    mem_limit: u64,
    l0: u64,
    l0_armed: bool,
    l0_limit: u64,
) -> WriteAdmit {
    if mem_armed && mem_bytes >= mem_limit {
        WriteAdmit::StallMem
    } else if l0_armed && l0 >= l0_limit {
        WriteAdmit::StallL0
    } else {
        WriteAdmit::Ok
    }
}

pub fn write_admit(
    mem_bytes: u64,
    mem_armed: bool,
    mem_limit: u64,
    l0: u64,
    l0_armed: bool,
    l0_limit: u64,
) -> (d: WriteAdmit)
    ensures
        d == write_admit_spec(mem_bytes, mem_armed, mem_limit, l0, l0_armed, l0_limit),
{
    admit_body!(mem_bytes, mem_armed, mem_limit, l0, l0_armed, l0_limit)
}

pub open spec fn write_admit_as_is_spec(
    _mem_bytes: u64,
    _mem_armed: bool,
    _mem_limit: u64,
    _l0: u64,
    _l0_armed: bool,
    _l0_limit: u64,
) -> WriteAdmit {
    WriteAdmit::Ok
}

pub fn write_admit_as_is(
    mem_bytes: u64,
    mem_armed: bool,
    mem_limit: u64,
    l0: u64,
    l0_armed: bool,
    l0_limit: u64,
) -> (d: WriteAdmit)
    ensures
        d == write_admit_as_is_spec(mem_bytes, mem_armed, mem_limit, l0, l0_armed, l0_limit),
{
    admit_as_is_body!(mem_bytes, mem_armed, mem_limit, l0, l0_armed, l0_limit)
}

pub open spec fn wal_sync_required_spec(client_set: bool, client_sync: bool, db_sync: bool) -> bool {
    if client_set {
        client_sync
    } else {
        db_sync
    }
}

pub fn wal_sync_required(client_set: bool, client_sync: bool, db_sync: bool) -> (d: bool)
    ensures
        d == wal_sync_required_spec(client_set, client_sync, db_sync),
{
    wal_sync_required_body!(client_set, client_sync, db_sync)
}

pub fn wal_sync_required_as_is(client_set: bool, client_sync: bool, db_sync: bool) -> (d: bool)
    ensures
        d == false,
{
    let _ = (client_set, client_sync, db_sync);
    false
}

pub open spec fn seq_exhausted_spec(seq: u64, max: u64) -> bool {
    seq > max
}

pub fn seq_exhausted(seq: u64, max: u64) -> (d: bool)
    ensures
        d == seq_exhausted_spec(seq, max),
{
    seq_exhausted_body!(seq, max)
}

pub fn seq_exhausted_as_is(seq: u64, max: u64) -> (d: bool)
    ensures
        d == false,
{
    let _ = (seq, max);
    false
}

pub open spec fn batch_is_empty_spec(n: u64) -> bool {
    n == 0
}

pub fn batch_is_empty(n: u64) -> (d: bool)
    ensures
        d == batch_is_empty_spec(n),
{
    batch_is_empty_body!(n)
}

pub fn batch_is_empty_as_is(n: u64) -> (d: bool)
    ensures
        d == false,
{
    let _ = n;
    false
}

pub open spec fn fence_on_sync_fail_spec(sync_required: bool, sync_failed: bool) -> bool {
    sync_required && sync_failed
}

pub fn fence_on_sync_fail(sync_required: bool, sync_failed: bool) -> (d: bool)
    ensures
        d == fence_on_sync_fail_spec(sync_required, sync_failed),
{
    fence_on_sync_fail_body!(sync_required, sync_failed)
}

pub fn fence_on_sync_fail_as_is(sync_required: bool, sync_failed: bool) -> (d: bool)
    ensures
        d == false,
{
    let _ = (sync_required, sync_failed);
    false
}

pub open spec fn torn_head_is_empty_log_spec(len: u64, tiny_max: u64) -> bool {
    len < tiny_max
}

pub fn torn_head_is_empty_log(len: u64, tiny_max: u64) -> (d: bool)
    ensures
        d == torn_head_is_empty_log_spec(len, tiny_max),
{
    torn_head_empty_log_body!(len, tiny_max)
}

pub fn torn_head_is_empty_log_as_is(len: u64, tiny_max: u64) -> (d: bool)
    ensures
        d == true,
{
    let _ = (len, tiny_max);
    true
}

pub open spec fn torn_tail_needs_cut_spec(len: u64, last_good: u64) -> bool {
    len > last_good
}

pub fn torn_tail_needs_cut(len: u64, last_good: u64) -> (d: bool)
    ensures
        d == torn_tail_needs_cut_spec(len, last_good),
{
    torn_tail_needs_cut_body!(len, last_good)
}

pub fn torn_tail_needs_cut_as_is(len: u64, last_good: u64) -> (d: bool)
    ensures
        d == false,
{
    let _ = (len, last_good);
    false
}

pub open spec fn seq_after_feed_spec(seq: u64, feed_max: u64) -> bool {
    seq > feed_max
}

pub fn seq_after_feed(seq: u64, feed_max: u64) -> (d: bool)
    ensures
        d == seq_after_feed_spec(seq, feed_max),
{
    seq_exhausted_body!(seq, feed_max)
}

pub fn seq_after_feed_as_is(seq: u64, feed_max: u64) -> (d: bool)
    ensures
        d == false,
{
    let _ = (seq, feed_max);
    false
}

pub fn pit_resync_needs_rewrite(is_resync: bool) -> (d: bool)
    ensures
        d == is_resync,
{
    is_resync
}

pub fn pit_resync_needs_rewrite_as_is(is_resync: bool) -> (d: bool)
    ensures
        d == false,
{
    let _ = is_resync;
    false
}

pub open spec fn dir_sync_required_spec(sync: bool) -> bool {
    sync
}

pub fn dir_sync_required(sync: bool) -> (d: bool)
    ensures
        d == dir_sync_required_spec(sync),
{
    dir_sync_required_body!(sync)
}

pub open spec fn dir_sync_required_as_is_spec(_sync: bool) -> bool {
    false
}

pub fn dir_sync_required_as_is(sync: bool) -> (d: bool)
    ensures
        d == dir_sync_required_as_is_spec(sync),
        d == false,
{
    let _ = sync;
    false
}

proof fn lemma_as_is_skips_dir_sync()
    ensures
        dir_sync_required_spec(true),
        !dir_sync_required_as_is_spec(true),
{
}

proof fn lemma_as_is_ignores_stall()
    ensures
        !write_admission_idle_spec(true, false, false),
        write_admission_idle_as_is_spec(true, false, false),
{
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_admission_idle_on_live_stall_is_not_ok() {
        assert!(write_admission_idle(false, false, false));
        assert!(!write_admission_idle(true, false, false));
        assert!(
            write_admission_idle_as_is(true, true, true),
            "AS-IS dente: stall knobs ignored"
        );
    }

    #[test]
    fn write_admit_on_live_mem_over_is_not_ok() {
        assert_eq!(
            write_admit(100, true, 50, 0, false, 0),
            WriteAdmit::StallMem
        );
        assert_eq!(
            write_admit_as_is(100, true, 50, 0, false, 0),
            WriteAdmit::Ok,
            "AS-IS dente: mem over still admits"
        );
        assert_eq!(write_admit(10, true, 50, 8, true, 4), WriteAdmit::StallL0);
        assert_eq!(write_admit(10, true, 50, 2, true, 4), WriteAdmit::Ok);
        assert_eq!(
            write_admit(100, true, 50, 8, true, 4),
            WriteAdmit::StallMem,
            "mem axis wins when both over"
        );
    }

    #[test]
    fn wal_sync_required_on_live_client_true_is_not_ok() {
        assert!(wal_sync_required(true, true, false));
        assert!(!wal_sync_required_as_is(true, true, false));
        assert!(!wal_sync_required(true, false, true));
        assert!(wal_sync_required(false, false, true));
    }

    #[test]
    fn seq_exhausted_on_live_ceiling_is_not_ok() {
        assert!(seq_exhausted(7, 6));
        assert!(!seq_exhausted_as_is(7, 6));
        assert!(!seq_exhausted(6, 6));
    }

    #[test]
    fn batch_is_empty_on_live_zero_is_not_ok() {
        assert!(batch_is_empty(0));
        assert!(!batch_is_empty_as_is(0));
        assert!(!batch_is_empty(1));
    }

    #[test]
    fn fence_on_sync_fail_on_live_required_fail_is_not_ok() {
        assert!(fence_on_sync_fail(true, true));
        assert!(!fence_on_sync_fail_as_is(true, true));
        assert!(!fence_on_sync_fail(true, false));
    }

    #[test]
    fn dir_sync_required_on_live_sync_is_not_ok() {
        assert!(dir_sync_required(true));
        assert!(
            !dir_sync_required_as_is(true),
            "AS-IS dente: never dir-fsync"
        );
        assert!(!dir_sync_required(false));
    }

    #[test]
    fn torn_head_is_empty_log_on_live_large_wal_is_not_ok() {
        assert!(torn_head_is_empty_log(8, TINY_WAL_EMPTY_MAX));
        assert!(
            torn_head_is_empty_log_as_is(10_000, TINY_WAL_EMPTY_MAX),
            "AS-IS dente: large Truncated(0) treated as empty"
        );
        assert!(!torn_head_is_empty_log(10_000, TINY_WAL_EMPTY_MAX));
    }

    #[test]
    fn torn_tail_needs_cut_on_live_overhang_is_not_ok() {
        assert!(torn_tail_needs_cut(10, 8));
        assert!(!torn_tail_needs_cut_as_is(10, 8));
        assert!(!torn_tail_needs_cut(8, 8));
    }

    #[test]
    fn seq_after_feed_on_live_newer_is_not_ok() {
        assert!(seq_after_feed(5, 4));
        assert!(!seq_after_feed_as_is(5, 4));
        assert!(!seq_after_feed(4, 4));
    }

    #[test]
    fn pit_resync_needs_rewrite_on_live_resync_is_not_ok() {
        assert!(pit_resync_needs_rewrite(true));
        assert!(!pit_resync_needs_rewrite_as_is(true));
        assert!(!pit_resync_needs_rewrite(false));
    }

    /// RFC-0171 P1.1/P1.2: data-fate `if`s on put-Ok and recover/reopen
    /// must call a kernel (not a raw predicate in `db.rs`).
    #[test]
    fn put_ok_and_recover_path_data_fate_ifs_call_kernels() {
        let src = include_str!("db.rs");
        let put_fns = [
            "put_with",
            "apply_batch_with",
            "commit_ops_with",
            "alloc_seq",
            "wal_sync_group",
            "sync_dir_if_required",
            "ensure_write_admitted_for",
        ];
        let rec_fns = ["open_with_env_sourced"];
        let mut bad = Vec::new();
        for name in put_fns.iter().chain(rec_fns.iter()) {
            let body = named_fn_src(src, name).unwrap_or_else(|| panic!("missing fn {name}"));
            for cond in if_conditions(&body) {
                if is_env_trampoline(&cond) || is_kernel_pred(&cond) {
                    continue;
                }
                bad.push(format!("{name}: {cond}"));
            }
        }
        assert!(
            bad.is_empty(),
            "data-fate ifs must call kernels:\n{}",
            bad.join("\n")
        );
    }

    // Brace bytes (not char literals) so pedra_formal match_braces can
    // still parse this module if cfg(test) stripping is skipped.
    const B_OPEN: u8 = 123;
    const B_CLOSE: u8 = 125;

    fn named_fn_src(src: &str, name: &str) -> Option<String> {
        let needle = format!("fn {}{}", name, "(");
        let start = src.find(&needle)?;
        let rest = &src[start..];
        let bytes = rest.as_bytes();
        let brace = bytes.iter().position(|&b| b == B_OPEN)?;
        let mut depth = 0i32;
        for (i, &b) in bytes[brace..].iter().enumerate() {
            if b == B_OPEN {
                depth += 1;
            } else if b == B_CLOSE {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[brace..=brace + i].to_string());
                }
            }
        }
        None
    }

    fn strip_comments(s: &str) -> String {
        let mut out = String::new();
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
                continue;
            }
            out.push(b[i] as char);
            i += 1;
        }
        out
    }

    fn if_conditions(body: &str) -> Vec<String> {
        let body = strip_comments(body);
        let mut out = Vec::new();
        let bytes = body.as_bytes();
        let mut i = 0;
        while i + 3 < bytes.len() {
            let at_if = bytes[i..].starts_with(b"if")
                && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_')
                && (bytes[i + 2] == b' ' || bytes[i + 2] == b'(' || bytes[i + 2] == b'\n');
            if at_if {
                let rest = &body[i + 2..];
                let end = rest.as_bytes().iter().position(|&b| b == B_OPEN);
                if let Some(end) = end {
                    out.push(rest[..end].trim().to_string());
                    i += 2 + end;
                    continue;
                }
            }
            i += 1;
        }
        out
    }

    fn is_env_trampoline(cond: &str) -> bool {
        cond.contains("env.")
            || cond.contains("exists(")
            || cond.contains("metadata_len")
            || cond.contains("cfg!")
            || cond.contains("debug_assert")
            || cond.contains("opts.")
            || cond.contains("exclusive")
            || cond.contains("source")
            || cond.contains("sst_payload")
            || cond.contains("buggify")
            || cond.contains("per_cf")
            || cond.contains("write_stall_drain")
            || cond.contains("defer_auto_compact")
            || cond.contains("physical_cfs")
            || cond.contains("resync_origin")
            || cond.contains("max_sequence")
            || cond.contains("large_value_threshold")
            || cond.contains("auto_blob_gc")
    }

    fn is_kernel_pred(cond: &str) -> bool {
        cond.contains("_kernel::")
            || cond.contains("write_admission_kernel::")
            || cond.contains("flush_kernel::")
            || cond.contains("reopen_kernel::")
            || cond.contains("recover_kernel::")
            || cond.contains("vlog_gc_kernel::")
            || cond.contains("write_ack_kernel::")
            || cond.contains("wal_state_kernel::")
            || cond.contains("write_admission_idle(")
            || cond.contains("write_admit(")
            || cond.contains("wal_sync_required(")
            || cond.contains("seq_exhausted(")
            || cond.contains("batch_is_empty(")
            || cond.contains("fence_on_sync_fail(")
            || cond.contains("dir_sync_required(")
            || cond.contains("torn_head_is_empty_log(")
            || cond.contains("torn_tail_needs_cut(")
            || cond.contains("seq_after_feed(")
            || cond.contains("pit_resync_needs_rewrite(")
            || cond.contains("reopen_outcome(")
            || cond.contains("feed_is_lazy(")
    }
}
