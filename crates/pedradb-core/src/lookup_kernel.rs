//! Point-lookup snapshot predicates (RFC-0174 P1.1).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). `db.rs` `get_at` / lookup call these
//! instead of raw `snap.seq` comparisons.
//!
//!   ./scripts/verus_lookup.sh

#![forbid(unsafe_code)]

macro_rules! snap_is_empty_body {
    ($seq:expr) => {
        $seq == 0u64
    };
}

macro_rules! snap_below_watermark_body {
    ($seq:expr, $earliest:expr) => {
        $seq < $earliest
    };
}

macro_rules! mem_point_decides_body {
    ($has_point:expr) => {
        $has_point
    };
}

macro_rules! prefer_newer_seq_body {
    ($have_best:expr, $new_seq:expr, $best_seq:expr) => {
        !$have_best || $new_seq > $best_seq
    };
}

macro_rules! vlog_ptr_orphaned_body {
    ($vlog_closed:expr) => {
        match $vlog_closed {
            true => true,
            false => false,
        }
    };
}

#[cfg(not(verus_keep_ghost))]
/// Snapshot sequence 0 never observes a version (empty snap → miss).
#[must_use]
pub fn snap_is_empty(seq: u64) -> bool {
    snap_is_empty_body!(seq)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: empty snap is treated as live (would serve seq-0 as a real read).
#[must_use]
pub fn snap_is_empty_as_is(_seq: u64) -> bool {
    false
}

#[cfg(not(verus_keep_ghost))]
/// Snapshot sits below the GC watermark — archive/LSM fallback, not the
/// published point path.
#[must_use]
pub fn snap_below_watermark(seq: u64, earliest: u64) -> bool {
    snap_below_watermark_body!(seq, earliest)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never treat a snap as below the watermark (would skip archive
/// and serve a dropped version from the LSM, or miss a retained one).
#[must_use]
pub fn snap_below_watermark_as_is(_seq: u64, _earliest: u64) -> bool {
    false
}

#[cfg(not(verus_keep_ghost))]
/// A memtable point at `seq ≤ snap` decides the lookup — skip SST.
#[must_use]
pub fn mem_point_decides(has_point: bool) -> bool {
    mem_point_decides_body!(has_point)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: mem never decides (would fall through to SST and resurrect a
/// deleted key, or miss a mem-only put).
#[must_use]
pub fn mem_point_decides_as_is(_has_point: bool) -> bool {
    false
}

#[cfg(not(verus_keep_ghost))]
/// Newest sequence among candidates covering the key wins.
#[must_use]
pub fn prefer_newer_seq(have_best: bool, new_seq: u64, best_seq: u64) -> bool {
    prefer_newer_seq_body!(have_best, new_seq, best_seq)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: first candidate always wins (older L0 put hides a newer delete).
#[must_use]
pub fn prefer_newer_seq_as_is(_have_best: bool, _new_seq: u64, _best_seq: u64) -> bool {
    true
}

#[cfg(not(verus_keep_ghost))]
/// Decoded vlog pointer but `VALUES.vlog` is closed — refuse, never serve
/// the pointer bytes as the user value (F1).
#[must_use]
pub fn vlog_ptr_orphaned(vlog_closed: bool) -> bool {
    vlog_ptr_orphaned_body!(vlog_closed)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never orphaned (would lock None / serve pointer bytes).
#[must_use]
pub fn vlog_ptr_orphaned_as_is(_vlog_closed: bool) -> bool {
    false
}

/// RFC-0219 P1.1: fate of a point/prefix cache fill or hit (F198/F207).
/// The fill runs under the read lock, which does not exclude
/// `publish_sequence` — only while the published seq still equals the
/// seq the answer was computed at is the answer admissible for the
/// cache.
#[cfg(not(verus_keep_ghost))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointCachePlan {
    /// Published still equals the answer's seq — fill / hit admissible.
    CacheCurrent,
    /// A publish landed mid-read — the pre-publish answer is stale; skip
    /// the fill / hit (the next read recomputes at the newer seq).
    PublishAdvanced,
}

#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn point_cache_validity(published_seq: u64, answer_seq: u64) -> PointCachePlan {
    if published_seq == answer_seq {
        PointCachePlan::CacheCurrent
    } else {
        PointCachePlan::PublishAdvanced
    }
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: fill / hit regardless of a racing publish — the stale
/// pre-publish answer is cached indefinitely (silent-wrong; F198).
#[must_use]
pub fn point_cache_validity_as_is(_published_seq: u64, _answer_seq: u64) -> PointCachePlan {
    PointCachePlan::CacheCurrent
}

/// RFC-0219 P1.1: fate of a Found point read under range tombstones
/// (RFC-0150). The point candidate won its user key; whether the
/// caller sees the value is decided here — a covering range tombstone
/// (`t.seq > point_seq`, computed by `merge::range_deleted`) shadows it.
#[cfg(not(verus_keep_ghost))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointTombstonePlan {
    /// No covering range tombstone — serve the found value.
    ValueVisible,
    /// A covering range tombstone hides the point — serve Deleted.
    ShadowedDeleted,
}

#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn point_tombstone_plan(range_hidden: bool) -> PointTombstonePlan {
    if range_hidden {
        PointTombstonePlan::ShadowedDeleted
    } else {
        PointTombstonePlan::ValueVisible
    }
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never shadow — a range-deleted point scans as live
/// (resurrection; RFC-0150 dente).
#[must_use]
pub fn point_tombstone_plan_as_is(_range_hidden: bool) -> PointTombstonePlan {
    PointTombstonePlan::ValueVisible
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn snap_is_empty_spec(seq: u64) -> bool {
    seq == 0
}

#[verifier::when_used_as_spec(snap_is_empty_spec)]
pub fn snap_is_empty(seq: u64) -> (d: bool)
    ensures
        d == snap_is_empty_spec(seq),
{
    snap_is_empty_body!(seq)
}

pub open spec fn snap_is_empty_as_is_spec(_seq: u64) -> bool {
    false
}

pub fn snap_is_empty_as_is(seq: u64) -> (d: bool)
    ensures
        d == snap_is_empty_as_is_spec(seq),
        d == false,
{
    let _ = seq;
    false
}

pub open spec fn snap_below_watermark_spec(seq: u64, earliest: u64) -> bool {
    seq < earliest
}

#[verifier::when_used_as_spec(snap_below_watermark_spec)]
pub fn snap_below_watermark(seq: u64, earliest: u64) -> (d: bool)
    ensures
        d == snap_below_watermark_spec(seq, earliest),
{
    snap_below_watermark_body!(seq, earliest)
}

pub open spec fn snap_below_watermark_as_is_spec(_seq: u64, _earliest: u64) -> bool {
    false
}

pub fn snap_below_watermark_as_is(seq: u64, earliest: u64) -> (d: bool)
    ensures
        d == snap_below_watermark_as_is_spec(seq, earliest),
        d == false,
{
    let _ = (seq, earliest);
    false
}

pub open spec fn mem_point_decides_spec(has_point: bool) -> bool {
    has_point
}

#[verifier::when_used_as_spec(mem_point_decides_spec)]
pub fn mem_point_decides(has_point: bool) -> (d: bool)
    ensures
        d == mem_point_decides_spec(has_point),
{
    mem_point_decides_body!(has_point)
}

pub open spec fn mem_point_decides_as_is_spec(_has_point: bool) -> bool {
    false
}

pub fn mem_point_decides_as_is(has_point: bool) -> (d: bool)
    ensures
        d == mem_point_decides_as_is_spec(has_point),
        d == false,
{
    let _ = has_point;
    false
}

pub open spec fn prefer_newer_seq_spec(have_best: bool, new_seq: u64, best_seq: u64) -> bool {
    !have_best || new_seq > best_seq
}

#[verifier::when_used_as_spec(prefer_newer_seq_spec)]
pub fn prefer_newer_seq(have_best: bool, new_seq: u64, best_seq: u64) -> (d: bool)
    ensures
        d == prefer_newer_seq_spec(have_best, new_seq, best_seq),
{
    prefer_newer_seq_body!(have_best, new_seq, best_seq)
}

pub open spec fn prefer_newer_seq_as_is_spec(_have_best: bool, _new_seq: u64, _best_seq: u64) -> bool {
    true
}

pub fn prefer_newer_seq_as_is(have_best: bool, new_seq: u64, best_seq: u64) -> (d: bool)
    ensures
        d == prefer_newer_seq_as_is_spec(have_best, new_seq, best_seq),
        d == true,
{
    let _ = (have_best, new_seq, best_seq);
    true
}

proof fn lemma_empty_snap_is_empty()
    ensures
        snap_is_empty(0),
        !snap_is_empty(1),
        !snap_is_empty_as_is_spec(0),
{
}

proof fn lemma_below_watermark()
    ensures
        snap_below_watermark(3, 5),
        !snap_below_watermark(5, 5),
        !snap_below_watermark_as_is_spec(3, 5),
{
}

proof fn lemma_mem_point_decides()
    ensures
        mem_point_decides(true),
        !mem_point_decides(false),
        !mem_point_decides_as_is_spec(true),
{
}

pub open spec fn vlog_ptr_orphaned_spec(vlog_closed: bool) -> bool {
    vlog_closed
}

pub open spec fn vlog_ptr_orphaned_as_is_spec(_vlog_closed: bool) -> bool {
    false
}

#[verifier::when_used_as_spec(vlog_ptr_orphaned_spec)]
pub fn vlog_ptr_orphaned(vlog_closed: bool) -> (d: bool)
    ensures
        d == vlog_ptr_orphaned_spec(vlog_closed),
{
    vlog_ptr_orphaned_body!(vlog_closed)
}

pub fn vlog_ptr_orphaned_as_is(vlog_closed: bool) -> (d: bool)
    ensures
        d == false,
{
    let _ = vlog_closed;
    false
}

proof fn lemma_vlog_ptr_orphaned()
    ensures
        vlog_ptr_orphaned(true),
        !vlog_ptr_orphaned(false),
        !vlog_ptr_orphaned_as_is_spec(true),
{
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_is_empty_on_live_zero_is_not_ok() {
        assert!(snap_is_empty(0));
        assert!(
            !snap_is_empty_as_is(0),
            "AS-IS dente: empty snap treated live"
        );
        assert!(!snap_is_empty(1));
        let body =
            named_fn_src(include_str!("db_kernel.rs"), "count_in_range").expect("count_in_range");
        assert!(
            body.contains("snap_is_empty("),
            "count_in_range must match snap_is_empty"
        );
        assert!(
            !body.contains("batch_is_empty("),
            "count_in_range must not wrap seq==0 onto batch_is_empty"
        );
        let last = named_fn_src(include_str!("db_kernel.rs"), "last_under_prefix")
            .expect("last_under_prefix");
        assert!(
            last.contains("snap_is_empty("),
            "last_under_prefix must match snap_is_empty"
        );
        let user = named_fn_src(include_str!("db_kernel.rs"), "last_under_user_prefix")
            .expect("last_under_user_prefix");
        assert!(
            user.contains("snap_is_empty("),
            "last_under_user_prefix must match snap_is_empty"
        );
        let vis =
            named_fn_src(include_str!("db_kernel.rs"), "count_visible").expect("count_visible");
        assert!(
            vis.contains("snap_is_empty("),
            "count_visible must match snap_is_empty"
        );
        let scan = named_fn_src(include_str!("db_kernel.rs"), "scan_at_raw").expect("scan_at_raw");
        assert!(
            scan.contains("snap_is_empty("),
            "scan_at_raw must match snap_is_empty"
        );
        assert!(
            !scan.contains("batch_is_empty("),
            "scan_at_raw must not wrap seq==0 onto batch_is_empty"
        );
    }

    #[test]
    fn snap_below_watermark_on_live_below_is_not_ok() {
        assert!(snap_below_watermark(3, 5));
        assert!(
            !snap_below_watermark_as_is(3, 5),
            "AS-IS dente: never below watermark"
        );
        assert!(!snap_below_watermark(5, 5));
        assert!(!snap_below_watermark(7, 5));
        let body = named_fn_src(include_str!("db_kernel.rs"), "ensure_snapshot_readable")
            .expect("ensure_snapshot_readable");
        assert!(
            body.contains("snap_below_watermark("),
            "ensure_snapshot_readable must match snap_below_watermark"
        );
        let ch = named_fn_src(include_str!("db_put_kernel.rs"), "changes").expect("changes");
        assert!(
            ch.contains("snap_below_watermark("),
            "changes must match snap_below_watermark"
        );
    }

    #[test]
    fn mem_point_decides_on_live_hit_is_not_ok() {
        assert!(mem_point_decides(true));
        assert!(
            !mem_point_decides_as_is(true),
            "AS-IS dente: mem never wins"
        );
        assert!(!mem_point_decides(false));
    }

    #[test]
    fn vlog_ptr_orphaned_on_live_closed_is_not_ok() {
        assert!(vlog_ptr_orphaned(true));
        assert!(!vlog_ptr_orphaned(false));
        assert!(
            !vlog_ptr_orphaned_as_is(true),
            "AS-IS dente: closed vlog still serves the pointer"
        );
        let body = named_fn_src(include_str!("db_kernel.rs"), "resolve_stored_value")
            .expect("resolve_stored_value");
        assert!(
            body.contains("vlog_ptr_orphaned("),
            "resolve_stored_value must match vlog_ptr_orphaned"
        );
        assert!(
            !body.contains("let Some(ref vlog) = self.vlog else"),
            "resolve_stored_value must not keep a raw closed-vlog if"
        );
    }

    #[test]
    fn prefer_newer_seq_on_live_older_first_is_not_ok() {
        assert!(prefer_newer_seq(false, 1, 0));
        assert!(prefer_newer_seq(true, 4, 2));
        assert!(!prefer_newer_seq(true, 2, 4));
        assert!(
            prefer_newer_seq_as_is(true, 2, 4),
            "AS-IS dente: first candidate always wins"
        );
        let src = include_str!("sst/table_kernel.rs");
        let body = src
            .split("fn best_point_in_entry_slice")
            .nth(1)
            .and_then(|s| s.split("fn ").next())
            .expect("best_point_in_entry_slice");
        assert!(
            body.contains("prefer_newer_seq("),
            "SST point newest-wins must call catalog prefer_newer_seq"
        );
    }

    #[test]
    fn point_cache_validity_on_live_publish_advanced_skips_fill() {
        // RFC-0219 P1.1: the F198/F207 fill/hit gates are the kernel's
        // plan — fill only while published still equals the answer's seq.
        assert_eq!(point_cache_validity(7, 7), PointCachePlan::CacheCurrent);
        assert_eq!(point_cache_validity(7, 6), PointCachePlan::PublishAdvanced);
        assert_eq!(
            point_cache_validity_as_is(7, 6),
            PointCachePlan::CacheCurrent,
            "AS-IS dente: stale pre-publish answer cached indefinitely"
        );
        // Live: all three gates match the kernel plan; the raw seq
        // equality left the trampoline.
        for (fn_name, raw) in [
            (
                "get_after_point_miss",
                "published_seq.load(Ordering::Acquire) == snap.seq",
            ),
            (
                "get_at",
                "published_seq.load(Ordering::Acquire) == snap.seq",
            ),
            (
                "last_under_user_prefix",
                "published_seq.load(Ordering::Acquire) == snapshot",
            ),
        ] {
            let body = named_fn_src(include_str!("db_kernel.rs"), fn_name)
                .unwrap_or_else(|| panic!("missing fn {fn_name}"));
            assert!(
                body.contains("match crate::lookup_kernel::point_cache_validity("),
                "{fn_name} must match point_cache_validity"
            );
            assert!(
                !body.contains(raw),
                "{fn_name} must not keep the raw published==answer if"
            );
        }
    }

    #[test]
    fn point_tombstone_plan_on_live_range_hidden_serves_deleted() {
        // RFC-0219 P1.1: a covering range tombstone shadows the found
        // point — the caller sees Deleted; AS-IS resurrects the value.
        assert_eq!(
            point_tombstone_plan(true),
            PointTombstonePlan::ShadowedDeleted
        );
        assert_eq!(
            point_tombstone_plan(false),
            PointTombstonePlan::ValueVisible
        );
        assert_eq!(
            point_tombstone_plan_as_is(true),
            PointTombstonePlan::ValueVisible,
            "AS-IS dente: range-deleted point scans as live"
        );
        // Live: the four point gates match the kernel plan; the inline
        // visible_at(Value, ..) gate left the trampoline.
        let lu = named_fn_src(include_str!("db_kernel.rs"), "lookup").expect("lookup");
        assert!(
            lu.matches("point_tombstone_plan(").count() >= 2,
            "lookup must match point_tombstone_plan on both gates"
        );
        let src = include_str!("db_kernel.rs");
        assert_eq!(
            src.matches("point_tombstone_plan(").count(),
            4,
            "exactly four call sites (lookup x2, lock-free path x2)"
        );
    }

    /// RFC-0174 P1.2: data-fate `if`s on get_at / lookup must call a kernel.
    #[test]
    fn get_at_and_lookup_path_data_fate_ifs_call_kernels() {
        let src = include_str!("db_kernel.rs");
        let get_fns = ["get_at", "lookup", "lookup_body", "scan_mem_for_lookup"];
        let mut bad = Vec::new();
        let mut seen = 0usize;
        for name in get_fns {
            let Some(body) = named_fn_src(src, name) else {
                continue;
            };
            seen += 1;
            for cond in if_conditions(&body) {
                if is_get_trampoline(&cond) || is_kernel_pred(&cond) {
                    continue;
                }
                bad.push(format!("{name}: {cond}"));
            }
        }
        assert!(seen >= 2, "missing get_at/lookup in db.rs");
        assert!(
            bad.is_empty(),
            "data-fate ifs must call kernels:\n{}",
            bad.join("\n")
        );
    }

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

    fn is_get_trampoline(cond: &str) -> bool {
        let t = cond.trim_start();
        t.starts_with("let ")
            || cond.contains("published_seq")
            || cond.contains("point_cache")
            || cond.contains("decode_vlog_ptr")
            || cond.contains("sst_only_settled")
            || cond.contains("bulk_runs")
            || cond.contains("parked_bulk")
            || cond.contains("bulk_encoding")
            || cond.contains("is_empty()")
            || cond.contains("has_range_tombstones")
            || cond.contains("key_may_match")
            || cond.contains("range_tomb_envelope")
            || cond.contains("elo.as_ref()")
            || cond.contains("ehi.as_ref()")
            || cond.contains("sorted_by_lo")
            || cond.contains("packed_lo")
            || cond.contains("packed_hi")
            || cond.contains("disjoint_by_lo")
            || cond.contains("partition_point")
            || cond.contains("phis.lo")
            || cond.contains("seek_scratch")
            || cond.contains("sst_envelope")
            || cond.contains("f == fam")
            || cond.contains("p == 0")
            || cond.contains("p > 0")
            || cond.contains("key < lo")
            || cond.contains("key > hi")
    }

    fn is_kernel_pred(cond: &str) -> bool {
        cond.contains("_kernel::")
            || cond.contains("lookup_kernel::")
            || cond.contains("probe_order_kernel::")
            || cond.contains("visible_at")
            || cond.contains("range_deleted")
            || cond.contains("snap_is_empty(")
            || cond.contains("snap_below_watermark(")
            || cond.contains("mem_point_decides(")
            || cond.contains("prefer_newer_seq(")
            || cond.contains("vlog_ptr_orphaned(")
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    #[kani::proof]
    fn kani_point_cache_validity_strict_invalidation() {
        let published: u64 = kani::any();
        let answer: u64 = kani::any();
        let plan = point_cache_validity(published, answer);
        if published == answer {
            assert_eq!(plan, PointCachePlan::CacheCurrent);
        } else {
            assert_eq!(plan, PointCachePlan::PublishAdvanced);
        }
    }

    #[kani::proof]
    fn kani_point_tombstone_shadowing_iff() {
        let hidden: bool = kani::any();
        let plan = point_tombstone_plan(hidden);
        if hidden {
            assert_eq!(plan, PointTombstonePlan::ShadowedDeleted);
        } else {
            assert_eq!(plan, PointTombstonePlan::ValueVisible);
        }
    }
}


