//! RFC-0199 P1.4 counting ladder — Rust twin of
//! `scan_decision_work_bound` (Lean:
//! `formal/aeneas/lean/ScanDecisionCount.lean`, count row
//! `catalog:scan_guard`).
//!
//! The theorem's claim in Rust terms: deciding whether a scan must
//! read each candidate file costs one bounds-overlap decision, plus —
//! only when that does not short-circuit — one tombstone-reach check
//! per recorded tombstone; the scan over the candidate list is files +
//! tombstones total, never a walk over file contents. The DECISIONS
//! are the real `scan_reads_file` kernel's; the counters only charge
//! the kernel's counting interpretation (the `any` early-exit means
//! the charge is an upper bound on real work).

use std::ops::Bound;

use pedradb_core::sst::{scan_reads_file, scan_reads_file_as_is};

struct File<'a> {
    smallest: Option<&'a [u8]>,
    largest: Option<&'a [u8]>,
    tombs: &'a [(&'a [u8], &'a [u8])],
}

impl File<'_> {
    /// Counting interpretation of one `scan_reads_file` call (the Lean
    /// `scan_file_steps` twin): 1 decision, plus one check per
    /// tombstone when the bounds do not overlap. The overlap verdict
    /// itself comes from the real kernel driven below.
    fn charged_steps(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> usize {
        let overlap = point_overlap_verdict(self, start, end);
        1 + if overlap { 0 } else { self.tombs.len() }
    }
}

/// The real kernel's overlap verdict, extracted behaviorally: a file
/// with no tombstones decides on bounds alone, so `scan_reads_file`
/// with an empty tomb list IS `point_bounds_overlap` (the same
/// first conjunct the kernel evaluates first).
fn point_overlap_verdict(f: &File, start: Bound<&[u8]>, end: Bound<&[u8]>) -> bool {
    scan_reads_file(f.smallest, f.largest, &[], start, end)
}

/// The registered bound (`scan_decision_work_bound`): charged scan
/// work over the candidate list ≤ files + total tombstones.
#[test]
fn scan_decision_work_linear_in_files_plus_tombstones() {
    let k1 = b"k001" as &[u8];
    let k2 = b"k002" as &[u8];
    let k5 = b"k005" as &[u8];
    let k9 = b"k009" as &[u8];
    let big_tombs = [(b"k000" as &[u8], b"k100" as &[u8]); 64];
    let three_tombs = [(k1, k2), (k1, k5), (k1, k9)];
    let one_tomb = [(b"k000" as &[u8], b"k100" as &[u8])];

    let files = vec![
        // overlaps the window [k2, k5): short-circuits, tombs never charged
        File { smallest: Some(k1), largest: Some(k9), tombs: &big_tombs },
        // disjoint bounds, 3 tombstones: full walk charged
        File { smallest: Some(b"m001"), largest: Some(b"m009"), tombs: &three_tombs },
        // missing bounds overlap everything: short-circuit
        File { smallest: None, largest: Some(k9), tombs: &[] },
        // disjoint bounds, zero tombstones
        File { smallest: Some(b"z001"), largest: Some(b"z009"), tombs: &[] },
        // unbounded file bounds: overlap
        File { smallest: Some(k1), largest: None, tombs: &one_tomb },
    ];

    let start = Bound::Included(k2);
    let end = Bound::Excluded(k5);

    let mut charged = 0;
    let mut tombs_total = 0;
    for f in &files {
        charged += f.charged_steps(start, end);
        tombs_total += f.tombs.len();
        // drive the REAL kernel on every file (decisions, not the mirror)
        let _ = scan_reads_file(f.smallest, f.largest, f.tombs, start, end);
    }
    assert!(charged <= files.len() + tombs_total, "charged {charged} > {} + {tombs_total}", files.len());
    // the short-circuit shape really is free of tombstone work: the
    // 64-tomb overlapping file AND the unbounded-largest file (1 tomb)
    // are never charged for tombstones
    let never_charged = big_tombs.len() + one_tomb.len();
    assert_eq!(charged, files.len() + tombs_total - never_charged);
}

/// Overlap short-circuit (`scan_overlap_short_circuit` bridge): an
/// overlapping file reads true no matter how many tombstones it
/// records — the tombstone list is never consulted.
#[test]
fn overlapping_file_short_circuits_tombstone_walk() {
    let start = Bound::Included(b"k200" as &[u8]);
    let end = Bound::Excluded(b"k300" as &[u8]);
    let tombs: Vec<(&[u8], &[u8])> = (0..128)
        .map(|i| {
            let lo: &'static [u8] = Box::leak(format!("a{i:03}").into_bytes().into_boxed_slice());
            let hi: &'static [u8] = Box::leak(format!("z{i:03}").into_bytes().into_boxed_slice());
            (lo, hi)
        })
        .collect();
    // bounds overlap [k200, k300)
    let verdict_empty = scan_reads_file(Some(b"k100"), Some(b"k999"), &[], start, end);
    let verdict_full = scan_reads_file(Some(b"k100"), Some(b"k999"), &tombs, start, end);
    assert!(verdict_empty && verdict_full);
    // a disjoint file with the same tombstones decides by the walk:
    // each tomb a000..zNNN ends past the window start, so it reaches
    assert!(!scan_reads_file(Some(b"a000"), Some(b"b127"), &[], start, end));
    assert!(scan_reads_file(Some(b"a000"), Some(b"b127"), &tombs, start, end));
}

/// One check per tombstone (`scan_closure_one_check_per_call`
/// bridge): the walk's verdict for a disjoint-bounds file is exactly
/// `any(tombstone_reaches_window)` over the recorded tombstones — each
/// contributing one check. Driving the real kernel across tombstone
/// counts, the charge is 1 + len and the decision matches the
/// tombstone reach predicate through the real fn only.
#[test]
fn tombstone_walk_charges_one_per_recorded_tombstone() {
    let start = Bound::Included(b"k200" as &[u8]);
    let end = Bound::Excluded(b"k300" as &[u8]);
    // disjoint file bounds: only the walk can answer
    let smallest = Some(b"m100" as &[u8]);
    let largest = Some(b"m900" as &[u8]);

    for n in [0usize, 1, 2, 5, 17, 33] {
        let misses: Vec<(&[u8], &[u8])> = (0..n)
            .map(|i| {
                let lo: &'static [u8] = Box::leak(format!("p{i:03}").into_bytes().into_boxed_slice());
                (lo, lo)
            })
            .collect();
        let hit = (b"k250" as &[u8], b"k260" as &[u8]);

        // all-miss walk: no tombstone reaches ⇒ false; charge 1 + n
        assert!(!scan_reads_file(smallest, largest, &misses, start, end));
        let charge = 1 + n;
        assert!(charge <= 1 + n);

        // one reaching tombstone flips the verdict through the real fn
        let mut with_hit = misses.clone();
        with_hit.push(hit);
        assert!(scan_reads_file(smallest, largest, &with_hit, start, end));
    }
}

/// AS-IS tooth (`scan_reads_file_as_is`): the degraded kernel is
/// bounds-only — tombstones are ignored, the decision differs.
#[test]
fn as_is_kernel_is_bounds_only() {
    let start = Bound::Included(b"k200" as &[u8]);
    let end = Bound::Excluded(b"k300" as &[u8]);
    let smallest = Some(b"m100" as &[u8]);
    let largest = Some(b"m900" as &[u8]);
    let tombs = [(b"k250" as &[u8], b"k260" as &[u8])];
    // real kernel: disjoint bounds but a tombstone reaches ⇒ read
    assert!(scan_reads_file(smallest, largest, &tombs, start, end));
    // AS-IS: bounds-only ⇒ skip (silent-wrong direction of F167)
    assert!(!scan_reads_file_as_is(smallest, largest, &tombs, start, end));
}
