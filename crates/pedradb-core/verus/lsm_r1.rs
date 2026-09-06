// Verus twin of the lsm_r1 kernel (RFC-0166 P2.1 —
// crates/pedradb-core/src/lsm_r1_kernel.rs). Not linked into production.
//
//   ./scripts/verus_lsm_r1.sh
//
// Theorems:
//   - Inv-LSM (distinct keys per level / next_seq above every live seq /
//     recency: a shallower level holding a key holds a NEWER version than
//     any deeper level holding it) is preserved by write, flush, compact
//     (with the bottom-level tombstone retire) and reopen.
//   - R1 (the named corollary): under Inv-LSM the probe (recency walk)
//     answers exactly the newest version — the resurrected-delete class
//     is impossible by construction.
//   - exec==spec for the probe path.
//   - The three AS-IS teeth (deepest-first probe, tombstone-dropping
//     compact, reversed reopen) are witnessed to resurrect on the
//     atom-reachable delete-resurrect shape.
//
// Model: levels as Seq<Seq<Entry>> (u64 keys stand in for [u8] under the
// same total order — the scan_guard/probe_order pattern). Order inside a
// level carries no meaning (the kernel's slots are unordered too), so the
// twin's remove drops the key without preserving slot order.

use vstd::prelude::*;

verus! {

#[derive(PartialEq, Eq, Clone, Copy)]
pub struct Entry {
    pub key: u64,
    pub seq: u64,
    pub tomb: bool,
}

pub struct LsmState {
    pub levels: Seq<Seq<Entry>>,
    pub next_seq: u64,
}

// --- per-level spec helpers ----------------------------------------------

/// The live version of `key` in `level`, scanning from index `i`.
pub open spec fn version_rec(level: Seq<Entry>, i: int, key: u64) -> Option<Entry>
    decreases level.len() - i,
{
    if i >= level.len() {
        None
    } else if level[i].key == key {
        Some(level[i])
    } else {
        version_rec(level, i + 1, key)
    }
}

/// First index of `key` in `level`, scanning from `i`.
pub open spec fn find_key(level: Seq<Entry>, i: int, key: u64) -> Option<int>
    decreases level.len() - i,
{
    if i >= level.len() {
        None
    } else if level[i].key == key {
        Some(i)
    } else {
        find_key(level, i + 1, key)
    }
}

proof fn find_key_none_from(level: Seq<Entry>, i: int, key: u64)
    requires
        0 <= i <= level.len(),
        find_key(level, i, key) == None,
    ensures
        forall|g: int| i <= g < level.len() ==> level[g].key != key,
    decreases level.len() - i,
{
    if i >= level.len() {
    } else {
        assert(level[i].key != key);
        find_key_none_from(level, i + 1, key);
    }
}

/// find_key Some(idx) means idx is in range and holds the key (fixed-idx
/// form — no quantifier instantiation needed at the use sites).
proof fn find_key_index(level: Seq<Entry>, i: int, key: u64, idx: int)
    requires
        0 <= i <= level.len(),
        find_key(level, i, key) == Some(idx),
    ensures
        i <= idx < level.len(),
        level[idx].key == key,
    decreases level.len() - i,
{
    if i >= level.len() {
        assert(find_key(level, i, key) == None);
        assert(Some(idx) == None);
        assert(false);
    } else if level[i].key == key {
        assert(find_key(level, i, key) == Some(i));
        assert(Some(idx) == Some(i));
        assert(idx == i);
        assert(level[idx].key == key);
    } else {
        assert(find_key(level, i, key) == find_key(level, i + 1, key));
        find_key_index(level, i + 1, key, idx);
    }
}

/// Whatever version_rec returns carries the queried key.
proof fn version_key(level: Seq<Entry>, i: int, key: u64)
    requires
        0 <= i <= level.len(),
    ensures
        forall|x: Entry| version_rec(level, i, key) == Some(x) ==> x.key == key,
    decreases level.len() - i,
{
    if i >= level.len() {
    } else if level[i].key == key {
        assert(version_rec(level, i, key) == Some(level[i]));
        assert forall|x: Entry| version_rec(level, i, key) == Some(x) implies x.key == key by {
            if version_rec(level, i, key) == Some(x) {
                assert(x == level[i]);
            }
        }
    } else {
        version_key(level, i + 1, key);
    }
}

/// Distinct keys per level.
pub open spec fn distinct_spec(level: Seq<Entry>) -> bool {
    forall|i: int, j: int| 0 <= i < j < level.len() ==> level[i].key != level[j].key
}

// --- Inv-LSM ---------------------------------------------------------------

pub open spec fn inv_lsm_spec(s: LsmState) -> bool {
    (forall|li: int| 0 <= li < s.levels.len() ==> distinct_spec(s.levels[li]))
    && (forall|li: int, i: int|
        0 <= li < s.levels.len() && 0 <= i < s.levels[li].len()
        ==> s.levels[li][i].seq < s.next_seq)
    && (forall|li: int, lj: int, i: int, j: int|
        0 <= li < lj < s.levels.len()
        && 0 <= i < s.levels[li].len() && 0 <= j < s.levels[lj].len()
        && s.levels[li][i].key == s.levels[lj][j].key
        ==> s.levels[li][i].seq > s.levels[lj][j].seq)
}

// --- probe / newest --------------------------------------------------------

pub open spec fn probe_rec(levels: Seq<Seq<Entry>>, li: int, key: u64) -> Option<Entry>
    decreases levels.len() - li,
{
    if li >= levels.len() {
        None
    } else {
        match version_rec(levels[li], 0, key) {
            Some(e) => Some(e),
            None => probe_rec(levels, li + 1, key),
        }
    }
}

pub open spec fn probe_spec(s: LsmState, key: u64) -> Option<Entry> {
    probe_rec(s.levels, 0, key)
}

pub open spec fn newest_rec(levels: Seq<Seq<Entry>>, li: int, key: u64) -> Option<Entry>
    decreases levels.len() - li,
{
    if li >= levels.len() {
        None
    } else {
        match version_rec(levels[li], 0, key) {
            Some(e) => match newest_rec(levels, li + 1, key) {
                Some(b) => if b.seq > e.seq { Some(b) } else { Some(e) },
                None => Some(e),
            },
            None => newest_rec(levels, li + 1, key),
        }
    }
}

pub open spec fn newest_spec(s: LsmState, key: u64) -> Option<Entry> {
    newest_rec(s.levels, 0, key)
}

/// version_rec Some(x) means x literally sits at some index of `level`.
proof fn version_index(level: Seq<Entry>, i: int, key: u64)
    requires
        0 <= i <= level.len(),
    ensures
        forall|x: Entry| version_rec(level, i, key) == Some(x)
            ==> (exists|ii: int| i <= ii < level.len() && level[ii] == x),
    decreases level.len() - i,
{
    if i >= level.len() {
    } else if level[i].key == key {
        assert(version_rec(level, i, key) == Some(level[i]));
        assert forall|x: Entry| version_rec(level, i, key) == Some(x)
            implies exists|ii: int| i <= ii < level.len() && level[ii] == x by {
            if level[i] == x {
                assert(exists|ii: int| i <= ii < level.len() && level[ii] == x) by {
                    assert(0 <= i < level.len() && level[i] == x);
                }
            }
        }
    } else {
        version_index(level, i + 1, key);
    }
}

proof fn scan_to(level: Seq<Entry>, start: int, i: int)
    requires
        0 <= start <= i < level.len(),
        forall|g: int| start <= g < i ==> level[g].key != level[i].key,
    ensures
        version_rec(level, start, level[i].key) == Some(level[i]),
    decreases i - start,
{
    if start == i {
        assert(version_rec(level, start, level[i].key) == Some(level[i]));
    } else {
        assert(level[start].key != level[i].key);
        scan_to(level, start + 1, i);
    }
}

/// Under distinctness, the entry at index `i` IS the level's version of
/// its key.
proof fn distinct_first_version(level: Seq<Entry>, i: int)
    requires
        distinct_spec(level),
        0 <= i < level.len(),
    ensures
        version_rec(level, 0, level[i].key) == Some(level[i]),
{
    let key = level[i].key;
    assert forall|g: int| 0 <= g < i implies level[g].key != key by {
        if level[g].key == key {
            assert(level[g].key == level[i].key);
        }
    };
    scan_to(level, 0, i);
}

// --- R1 lemma chain --------------------------------------------------------

proof fn probe_none_means_all_none(levels: Seq<Seq<Entry>>, li: int, key: u64)
    requires
        0 <= li <= levels.len(),
        probe_rec(levels, li, key) == None,
    ensures
        forall|g: int| li <= g < levels.len() ==> version_rec(levels[g], 0, key) == None,
    decreases levels.len() - li,
{
    if li >= levels.len() {
    } else {
        assert(version_rec(levels[li], 0, key) == None) by {
            match version_rec(levels[li], 0, key) {
                Some(x) => {
                    assert(probe_rec(levels, li, key) == Some(x));
                    assert(false);
                }
                None => {}
            }
        };
        probe_none_means_all_none(levels, li + 1, key);
    }
}

proof fn all_none_means_newest_none(levels: Seq<Seq<Entry>>, li: int, key: u64)
    requires
        0 <= li <= levels.len(),
        forall|g: int| li <= g < levels.len() ==> version_rec(levels[g], 0, key) == None,
    ensures
        newest_rec(levels, li, key) == None,
    decreases levels.len() - li,
{
    if li >= levels.len() {
    } else {
        all_none_means_newest_none(levels, li + 1, key);
    }
}

proof fn probe_found_is_first_holder(
    levels: Seq<Seq<Entry>>,
    li: int,
    key: u64,
    e: Entry,
)
    requires
        0 <= li <= levels.len(),
        probe_rec(levels, li, key) == Some(e),
    ensures
        exists|f: int|
            li <= f < levels.len() && version_rec(levels[f], 0, key) == Some(e)
            && forall|g: int| li <= g < f ==> version_rec(levels[g], 0, key) == None,
    decreases levels.len() - li,
{
    if li >= levels.len() {
        // probe_rec is None here: contradiction with the requires
        assert(probe_rec(levels, li, key) == None);
        assert(probe_rec(levels, li, key) == Some(e));
    } else {
        match version_rec(levels[li], 0, key) {
            Some(v) => {
                assert(v == e);
                assert(exists|f: int|
                    li <= f < levels.len() && version_rec(levels[f], 0, key) == Some(e)
                    && forall|g: int| li <= g < f
                        ==> version_rec(levels[g], 0, key) == None) by {
                    assert(li <= li < levels.len()
                        && version_rec(levels[li], 0, key) == Some(e));
                }
            }
            None => {
                probe_found_is_first_holder(levels, li + 1, key, e);
                assert(exists|f: int|
                    li <= f < levels.len() && version_rec(levels[f], 0, key) == Some(e)
                    && forall|g: int| li <= g < f
                        ==> version_rec(levels[g], 0, key) == None) by {
                    let f = choose|f: int|
                        li + 1 <= f < levels.len() && version_rec(levels[f], 0, key) == Some(e)
                        && forall|g: int| li + 1 <= g < f
                            ==> version_rec(levels[g], 0, key) == None;
                    assert(li <= f < levels.len()
                        && version_rec(levels[f], 0, key) == Some(e));
                }
            }
        }
    }
}

proof fn newest_none_means_all_none(levels: Seq<Seq<Entry>>, li: int, key: u64)
    requires
        0 <= li <= levels.len(),
        newest_rec(levels, li, key) == None,
    ensures
        forall|g: int| li <= g < levels.len() ==> version_rec(levels[g], 0, key) == None,
    decreases levels.len() - li,
{
    if li >= levels.len() {
    } else {
        assert(version_rec(levels[li], 0, key) == None) by {
            match version_rec(levels[li], 0, key) {
                Some(x) => {
                    assert(newest_rec(levels, li, key) != None) by {
                        assert(match newest_rec(levels, li + 1, key) {
                            Some(b2) => (if b2.seq > x.seq { Some(b2) } else { Some(x) }) != None,
                            None => Some(x) != None,
                        });
                    }
                    assert(false);
                }
                None => {}
            }
        };
        newest_none_means_all_none(levels, li + 1, key);
    }
}

proof fn newest_dominates_suffix(
    levels: Seq<Seq<Entry>>,
    li: int,
    key: u64,
    b: Entry,
    g: int,
    x: Entry,
)
    requires
        0 <= li <= levels.len(),
        newest_rec(levels, li, key) == Some(b),
        li <= g < levels.len(),
        version_rec(levels[g], 0, key) == Some(x),
    ensures
        x.seq <= b.seq,
    decreases levels.len() - li,
{
    if li >= levels.len() {
        assert(false) by {
            assert(version_rec(levels[g], 0, key) == Some(x));
        };
    } else {
        match version_rec(levels[li], 0, key) {
            Some(v) => {
                if g == li {
                    assert(x == v);
                    assert(x.seq <= b.seq) by {
                        match newest_rec(levels, li + 1, key) {
                            Some(b2) => {
                                assert(b == (if b2.seq > v.seq { b2 } else { v }));
                            }
                            None => {
                                assert(b == v);
                            }
                        }
                    }
                } else {
                    match newest_rec(levels, li + 1, key) {
                        Some(b2) => {
                            assert(b == (if b2.seq > v.seq { b2 } else { v }));
                            newest_dominates_suffix(levels, li + 1, key, b2, g, x);
                        }
                        None => {
                            assert(false) by {
                                assert(version_rec(levels[g], 0, key) == Some(x));
                                newest_none_means_all_none(levels, li + 1, key);
                            }
                        }
                    }
                }
            }
            None => {
                assert(newest_rec(levels, li, key) == newest_rec(levels, li + 1, key));
                newest_dominates_suffix(levels, li + 1, key, b, g, x);
            }
        }
    }
}

proof fn newest_is_attained(levels: Seq<Seq<Entry>>, li: int, key: u64, b: Entry)
    requires
        0 <= li <= levels.len(),
        newest_rec(levels, li, key) == Some(b),
    ensures
        exists|g: int|
            li <= g < levels.len() && version_rec(levels[g], 0, key) == Some(b),
    decreases levels.len() - li,
{
    if li >= levels.len() {
        assert(false);
    } else {
        match version_rec(levels[li], 0, key) {
            Some(v) => {
                match newest_rec(levels, li + 1, key) {
                    None => {
                        assert(b == v);
                        assert(exists|g: int|
                            li <= g < levels.len()
                                && version_rec(levels[g], 0, key) == Some(b)) by {
                            assert(li <= li < levels.len()
                                && version_rec(levels[li], 0, key) == Some(b));
                        }
                    }
                    Some(b2) => {
                        assert(b == (if b2.seq > v.seq { b2 } else { v }));
                        if b2.seq > v.seq {
                            assert(b == b2);
                            newest_is_attained(levels, li + 1, key, b2);
                            assert(exists|g: int|
                                li <= g < levels.len()
                                    && version_rec(levels[g], 0, key) == Some(b)) by {
                                newest_is_attained(levels, li + 1, key, b2);
                            }
                        } else {
                            assert(b == v);
                            assert(exists|g: int|
                                li <= g < levels.len()
                                    && version_rec(levels[g], 0, key) == Some(b)) by {
                                assert(li <= li < levels.len()
                                    && version_rec(levels[li], 0, key) == Some(b));
                            }
                        }
                    }
                }
            }
            None => {
                newest_is_attained(levels, li + 1, key, b);
            }
        }
    }
}

proof fn newest_skips_empty_prefix(
    levels: Seq<Seq<Entry>>,
    li: int,
    f: int,
    key: u64,
)
    requires
        0 <= li <= f <= levels.len(),
        forall|g: int| li <= g < f ==> version_rec(levels[g], 0, key) == None,
    ensures
        newest_rec(levels, li, key) == newest_rec(levels, f, key),
    decreases f - li,
{
    if li == f {
    } else {
        assert(version_rec(levels[li], 0, key) == None);
        newest_skips_empty_prefix(levels, li + 1, f, key);
    }
}

/// R1 (the named corollary of RFC-0166 P2.1): under Inv-LSM the probe
/// answers exactly the newest version of the key — a delete never
/// resurrects.
proof fn r1_theorem(s: LsmState, key: u64)
    requires
        inv_lsm_spec(s),
    ensures
        probe_spec(s, key) == newest_spec(s, key),
{
    match probe_spec(s, key) {
        None => {
            probe_none_means_all_none(s.levels, 0, key);
            all_none_means_newest_none(s.levels, 0, key);
        }
        Some(e) => {
            assert(exists|f: int|
                0 <= f < s.levels.len() && version_rec(s.levels[f], 0, key) == Some(e)
                && forall|g: int| 0 <= g < f
                    ==> version_rec(s.levels[g], 0, key) == None) by {
                probe_found_is_first_holder(s.levels, 0, key, e);
            }
            let f = choose|f: int|
                0 <= f < s.levels.len() && version_rec(s.levels[f], 0, key) == Some(e)
                && forall|g: int| 0 <= g < f
                    ==> version_rec(s.levels[g], 0, key) == None;
            newest_skips_empty_prefix(s.levels, 0, f, key);
            version_index(s.levels[f], 0, key);
            let if_ = choose|ii: int|
                0 <= ii < s.levels[f].len() && s.levels[f][ii] == e;
            match newest_rec(s.levels, f + 1, key) {
                None => {
                    assert(newest_rec(s.levels, f, key) == Some(e));
                }
                Some(b2) => {
                    newest_is_attained(s.levels, f + 1, key, b2);
                    let g2 = choose|g: int|
                        f + 1 <= g < s.levels.len()
                        && version_rec(s.levels[g], 0, key) == Some(b2);
                    version_index(s.levels[g2], 0, key);
                    let jg = choose|jj: int|
                        0 <= jj < s.levels[g2].len() && s.levels[g2][jj] == b2;
                    // recency: f < g2 and both hold the key ⇒ e is newer
                    version_key(s.levels[f], 0, key);
                    version_key(s.levels[g2], 0, key);
                    assert(e.key == key);
                    assert(b2.key == key);
                    assert(s.levels[f][if_].key == s.levels[g2][jg].key);
                    assert(s.levels[f][if_].seq > s.levels[g2][jg].seq);
                    assert(e.seq > b2.seq);
                    assert(newest_rec(s.levels, f, key) == Some(e));
                }
            }
        }
    }
}

// --- state atoms (spec twins of the kernel) --------------------------------

pub open spec fn put_level(dst: Seq<Entry>, e: Entry) -> Seq<Entry> {
    match find_key(dst, 0, e.key) {
        Some(i) => dst.update(i, e),
        None => dst.push(e),
    }
}

/// Order-free removal of `key` (the kernel's swap-remove, semantically:
/// the key is dropped, the rest survives).
pub open spec fn remove_key_rec(level: Seq<Entry>, i: int, key: u64) -> Seq<Entry>
    decreases level.len() - i,
{
    if i >= level.len() {
        Seq::empty()
    } else if level[i].key == key {
        remove_key_rec(level, i + 1, key)
    } else {
        remove_key_rec(level, i + 1, key).push(level[i])
    }
}

proof fn put_distinct(dst: Seq<Entry>, e: Entry)
    requires
        distinct_spec(dst),
    ensures
        distinct_spec(put_level(dst, e)),
{
    match find_key(dst, 0, e.key) {
        Some(i) => {
            assert(find_key(dst, 0, e.key) == Some(i));
            find_key_index(dst, 0, e.key, i);
            assert(0 <= i < dst.len());
            assert(dst[i].key == e.key);
            let out = dst.update(i, e);
            assert(put_level(dst, e) === out);
            assert(out.len() == dst.len());
            assert forall|x: int| 0 <= x < out.len() implies out[x].key == dst[x].key by {
                if 0 <= x < out.len() {
                    if x == i {
                        assert(out[x] == e);
                    }
                }
            }
            assert forall|a: int, b: int|
                0 <= a < b < out.len() implies out[a].key != out[b].key by {
                if 0 <= a < b < out.len() {
                    assert(0 <= a < b < dst.len());
                    assert(dst[a].key != dst[b].key);
                }
            }
        }
        None => {
            find_key_none_from(dst, 0, e.key);
            let out = dst.push(e);
            assert(put_level(dst, e) === out);
            assert(out.len() == dst.len() + 1);
            assert forall|a: int, b: int|
                0 <= a < b < out.len() implies out[a].key != out[b].key by {
                if 0 <= a < b < out.len() {
                    if b == out.len() - 1 {
                        assert(out[b] == e);
                        assert(0 <= a < dst.len());
                        assert(dst[a].key != e.key);
                    }
                }
            }
        }
    }
}

proof fn put_prov(dst: Seq<Entry>, e: Entry)
    ensures
        forall|oi: int| 0 <= oi < put_level(dst, e).len()
            ==> put_level(dst, e)[oi] == e
                || (exists|di: int| 0 <= di < dst.len()
                    && put_level(dst, e)[oi] == dst[di]),
{
    match find_key(dst, 0, e.key) {
        Some(i) => {
            assert(find_key(dst, 0, e.key) == Some(i));
            find_key_index(dst, 0, e.key, i);
            assert(0 <= i < dst.len());
            let out = dst.update(i, e);
            assert(put_level(dst, e) === out);
            assert forall|oi: int| 0 <= oi < out.len()
                implies out[oi] == e
                    || (exists|di: int| 0 <= di < dst.len() && out[oi] == dst[di]) by {
                if 0 <= oi < out.len() {
                    if oi == i {
                        assert(out[oi] == e);
                    } else {
                        assert(out[oi] == dst[oi]);
                        assert(exists|di: int| 0 <= di < dst.len() && out[oi] == dst[di]) by {
                            assert(0 <= oi < dst.len() && out[oi] == dst[oi]);
                        }
                    }
                }
            }
        }
        None => {
            let out = dst.push(e);
            assert(put_level(dst, e) === out);
            assert forall|oi: int| 0 <= oi < out.len()
                implies out[oi] == e
                    || (exists|di: int| 0 <= di < dst.len() && out[oi] == dst[di]) by {
                if 0 <= oi < out.len() {
                    if oi < dst.len() {
                        assert(out[oi] == dst[oi]);
                        assert(exists|di: int| 0 <= di < dst.len() && out[oi] == dst[di]) by {
                            assert(0 <= oi < dst.len() && out[oi] == dst[oi]);
                        }
                    } else {
                        assert(oi == dst.len());
                        assert(out[oi] == e);
                    }
                }
            }
        }
    }
}

/// Named indexing of the removal result.
pub open spec fn removed_at(level: Seq<Entry>, i: int, key: u64, c: int) -> Entry {
    remove_key_rec(level, i, key)[c]
}

/// Every survivor of the removal is a source entry (with a different
/// key) — returned as the witness index.
proof fn remove_prov_index(level: Seq<Entry>, i: int, key: u64, c: int) -> (g: int)
    requires
        0 <= i <= level.len(),
        0 <= c < remove_key_rec(level, i, key).len(),
    ensures
        i <= g < level.len(),
        level[g].key != key,
        remove_key_rec(level, i, key)[c] == level[g],
    decreases level.len() - i,
{
    if i >= level.len() {
        assert(remove_key_rec(level, i, key).len() == 0);
        assert(false);
        0
    } else if level[i].key == key {
        assert(remove_key_rec(level, i, key) === remove_key_rec(level, i + 1, key));
        remove_prov_index(level, i + 1, key, c)
    } else {
        let rest = remove_key_rec(level, i + 1, key);
        assert(remove_key_rec(level, i, key) === rest.push(level[i]));
        assert(rest.push(level[i]).len() == rest.len() + 1);
        if c < rest.len() {
            remove_prov_index(level, i + 1, key, c)
        } else {
            assert(c == rest.len());
            assert(rest.push(level[i])[c] == level[i]);
            i
        }
    }
}

proof fn remove_distinct(level: Seq<Entry>, i: int, key: u64)
    requires
        0 <= i <= level.len(),
        distinct_spec(level),
    ensures
        distinct_spec(remove_key_rec(level, i, key)),
    decreases level.len() - i,
{
    if i >= level.len() {
    } else if level[i].key == key {
        remove_distinct(level, i + 1, key);
    } else {
        remove_distinct(level, i + 1, key);
        let rest = remove_key_rec(level, i + 1, key);
        assert(remove_key_rec(level, i, key) === rest.push(level[i]));
        assert forall|c: int| 0 <= c < rest.len() implies rest[c].key != level[i].key by {
            if 0 <= c < rest.len() {
                let g = remove_prov_index(level, i + 1, key, c);
                assert(rest[c] == level[g]);
                if level[g].key == level[i].key {
                    assert(false);
                }
            }
        }
        assert forall|a: int, b: int|
            0 <= a < b < rest.push(level[i]).len()
            implies rest.push(level[i])[a].key != rest.push(level[i])[b].key by {
            if 0 <= a < b < rest.push(level[i]).len() {
                if b == rest.len() {
                    assert(rest.push(level[i])[b] == level[i]);
                    if a < rest.len() {
                        assert(rest.push(level[i])[a] == rest[a]);
                    }
                }
            }
        }
    }
}

/// One merge step over `src` from index `i`: put every entry (sources
/// folded later win), or — when `drop_tomb` — retire tombstones.
pub open spec fn merge_step(dst: Seq<Entry>, src: Seq<Entry>, i: int, drop_tomb: bool) -> Seq<Entry>
    decreases src.len() - i,
{
    if i >= src.len() {
        dst
    } else if drop_tomb && src[i].tomb {
        merge_step(remove_key_rec(dst, 0, src[i].key), src, i + 1, drop_tomb)
    } else {
        merge_step(put_level(dst, src[i]), src, i + 1, drop_tomb)
    }
}

proof fn merge_distinct(dst: Seq<Entry>, src: Seq<Entry>, i: int, drop_tomb: bool)
    requires
        0 <= i <= src.len(),
        distinct_spec(dst),
        distinct_spec(src),
    ensures
        distinct_spec(merge_step(dst, src, i, drop_tomb)),
    decreases src.len() - i,
{
    if i >= src.len() {
    } else if drop_tomb && src[i].tomb {
        remove_distinct(dst, 0, src[i].key);
        merge_distinct(remove_key_rec(dst, 0, src[i].key), src, i + 1, drop_tomb);
    } else {
        put_distinct(dst, src[i]);
        merge_distinct(put_level(dst, src[i]), src, i + 1, drop_tomb);
    }
}

/// `e` literally sits in `s.levels[li]`.
pub open spec fn entry_of_level(s: LsmState, li: int, e: Entry) -> bool {
    exists|ii: int| 0 <= ii < s.levels[li].len() && s.levels[li][ii] == e
}

/// `e` sits in some level of `s` within `[lo, hi)`.
pub open spec fn prov_in(s: LsmState, lo: int, hi: int, e: Entry) -> bool {
    exists|li: int| lo <= li < hi && entry_of_level(s, li, e)
}

proof fn prov_in_widen(s: LsmState, lo: int, mlo: int, hi: int, e: Entry)
    requires
        prov_in(s, lo, hi, e),
        mlo <= lo,
    ensures
        prov_in(s, mlo, hi, e),
{
    let li = choose|l: int| lo <= l < hi && entry_of_level(s, l, e);
    assert(prov_in(s, mlo, hi, e)) by {
        assert(mlo <= li < hi && entry_of_level(s, li, e));
    }
}

/// Provenance transfers across entry equality.
proof fn prov_in_eq(s: LsmState, mlo: int, hi: int, e1: Entry, e2: Entry)
    requires
        prov_in(s, mlo, hi, e1),
        e1 == e2,
    ensures
        prov_in(s, mlo, hi, e2),
{
    let l = choose|l: int| mlo <= l < hi && entry_of_level(s, l, e1);
    let ii = choose|ii: int| 0 <= ii < s.levels[l].len() && s.levels[l][ii] == e1;
    assert(prov_in(s, mlo, hi, e2)) by {
        assert(exists|ii2: int| 0 <= ii2 < s.levels[l].len()
            && s.levels[l][ii2] == e2) by {
            assert(0 <= ii < s.levels[l].len() && s.levels[l][ii] == e2);
        }
        assert(mlo <= l < hi && entry_of_level(s, l, e2));
    }
}

/// Any entry of `s.levels[li]` is prov_in for any range covering `li`.
proof fn prov_in_at(s: LsmState, li: int, mlo: int, hi: int, e: Entry)
    requires
        0 <= li < s.levels.len(),
        mlo <= li < hi <= s.levels.len(),
        exists|ii: int| 0 <= ii < s.levels[li].len() && s.levels[li][ii] == e,
    ensures
        prov_in(s, mlo, hi, e),
{
    let ii = choose|ii: int| 0 <= ii < s.levels[li].len() && s.levels[li][ii] == e;
    assert(prov_in(s, mlo, hi, e)) by {
        assert(mlo <= li < hi && entry_of_level(s, li, e));
    }
}

proof fn merge_prov(
    dst: Seq<Entry>,
    src: Seq<Entry>,
    i: int,
    drop_tomb: bool,
    s: LsmState,
    mlo: int,
    hi: int,
)
    requires
        0 <= i <= src.len(),
        mlo <= hi <= s.levels.len(),
        forall|oi: int| 0 <= oi < dst.len() ==> prov_in(s, mlo, hi, dst[oi]),
        forall|oi: int| 0 <= oi < src.len() ==> prov_in(s, mlo, hi, src[oi]),
    ensures
        forall|oi: int| 0 <= oi < merge_step(dst, src, i, drop_tomb).len()
            ==> prov_in(s, mlo, hi, merge_step(dst, src, i, drop_tomb)[oi]),
    decreases src.len() - i,
{
    if i >= src.len() {
        assert(merge_step(dst, src, i, drop_tomb) === dst);
        assert forall|oi: int| 0 <= oi < merge_step(dst, src, i, drop_tomb).len()
            ==> prov_in(s, mlo, hi, merge_step(dst, src, i, drop_tomb)[oi]) by {
            if 0 <= oi < merge_step(dst, src, i, drop_tomb).len() {
                assert(merge_step(dst, src, i, drop_tomb)[oi] == dst[oi]);
                assert(prov_in(s, mlo, hi, dst[oi]));
                prov_in_eq(s, mlo, hi, dst[oi], merge_step(dst, src, i, drop_tomb)[oi]);
            }
        };
    } else if drop_tomb && src[i].tomb {
        let dst2 = remove_key_rec(dst, 0, src[i].key);
        assert forall|oi: int| 0 <= oi < dst2.len() ==> prov_in(s, mlo, hi, dst2[oi]) by {
            if 0 <= oi < dst2.len() {
                let g = remove_prov_index(dst, 0, src[i].key, oi);
                assert(dst2[oi] == dst[g]);
                assert(prov_in(s, mlo, hi, dst[g]));
                prov_in_eq(s, mlo, hi, dst[g], dst2[oi]);
            }
        };
        assert(merge_step(dst, src, i, drop_tomb)
            === merge_step(dst2, src, i + 1, drop_tomb));
        merge_prov(dst2, src, i + 1, drop_tomb, s, mlo, hi);
    } else {
        let dst2 = put_level(dst, src[i]);
        assert forall|oi: int| 0 <= oi < dst2.len() ==> prov_in(s, mlo, hi, dst2[oi]) by {
            if 0 <= oi < dst2.len() {
                assert(dst2[oi] == src[i]
                    || (exists|di: int| 0 <= di < dst.len() && dst2[oi] == dst[di])) by {
                    put_prov(dst, src[i]);
                }
                if dst2[oi] == src[i] {
                    assert(prov_in(s, mlo, hi, src[i]));
                    prov_in_eq(s, mlo, hi, src[i], dst2[oi]);
                } else {
                    let di = choose|di: int| 0 <= di < dst.len() && dst2[oi] == dst[di];
                    assert(prov_in(s, mlo, hi, dst[di]));
                    prov_in_eq(s, mlo, hi, dst[di], dst2[oi]);
                }
            }
        };
        assert(merge_step(dst, src, i, drop_tomb)
            === merge_step(dst2, src, i + 1, drop_tomb));
        merge_prov(dst2, src, i + 1, drop_tomb, s, mlo, hi);
    }
}

// --- the state atoms -------------------------------------------------------

pub open spec fn write_spec(s: LsmState, key: u64, tomb: bool) -> LsmState {
    LsmState {
        levels: s.levels.update(
            0,
            put_level(s.levels[0], Entry {
                key,
                seq: s.next_seq,
                tomb,
            }),
        ),
        next_seq: (s.next_seq + 1) as u64,
    }
}

pub open spec fn flush_spec(s: LsmState) -> LsmState {
    LsmState {
        levels: s.levels.update(0, Seq::empty()).update(
            1,
            merge_step(s.levels[1], s.levels[0], 0, false),
        ),
        next_seq: s.next_seq,
    }
}

/// Compact fold: levels `li..depth` fold into `depth`, deepest sources
/// first (a shallower — newer — source overwrites older ones). When
/// `always_drop`, every tombstone retires (the AS-IS mutant); otherwise
/// tombstones retire only at the bottom level.
pub open spec fn compact_fold_flag(
    s: LsmState,
    depth: int,
    li: int,
    always_drop: bool,
) -> LsmState
    decreases depth - li,
{
    if li >= depth {
        s
    } else {
        let base = compact_fold_flag(s, depth, li + 1, always_drop);
        let drop = always_drop || depth == s.levels.len() - 1;
        LsmState {
            levels: base.levels.update(li, Seq::empty()).update(
                depth,
                merge_step(base.levels[depth], base.levels[li], 0, drop),
            ),
            next_seq: base.next_seq,
        }
    }
}

pub open spec fn compact_spec(s: LsmState, depth: int) -> LsmState {
    compact_fold_flag(s, depth, 0, false)
}

pub open spec fn compact_as_is_spec(s: LsmState, depth: int) -> LsmState {
    compact_fold_flag(s, depth, 0, true)
}

pub open spec fn reopen_spec(s: LsmState) -> LsmState {
    s
}

// --- preservation theorems ---------------------------------------------------

/// Inv-LSM is preserved by write (the new entry mints the global newest
/// seq at level 0).
proof fn write_preserves_inv(s: LsmState, key: u64, tomb: bool)
    requires
        inv_lsm_spec(s),
        s.levels.len() >= 1,
        s.next_seq < 0xFFFF_FFFF_FFFF_FFFF,
    ensures
        inv_lsm_spec(write_spec(s, key, tomb)),
{
    let e0 = Entry {
        key,
        seq: s.next_seq,
        tomb,
    };
    let new0 = put_level(s.levels[0], e0);
    let out = write_spec(s, key, tomb);
    assert(out.levels === s.levels.update(0, new0));
    put_distinct(s.levels[0], e0);
    put_prov(s.levels[0], e0);

    // clause 1: distinct keys per level
    assert forall|g: int| 0 <= g < out.levels.len() implies distinct_spec(out.levels[g]) by {
        if 0 <= g < out.levels.len() {
            if g == 0 {
                assert(out.levels[0] === new0);
            } else {
                assert(out.levels[g] === s.levels[g]);
            }
        }
    }
    // clause 2: every live seq < next_seq (the new next_seq is +1)
    assert forall|li: int, i: int|
        0 <= li < out.levels.len() && 0 <= i < out.levels[li].len()
        implies out.levels[li][i].seq < out.next_seq by {
        if 0 <= li < out.levels.len() && 0 <= i < out.levels[li].len() {
            if li == 0 {
                assert(out.levels[0] === new0);
                assert(new0[i] == e0 || (exists|di: int|
                    0 <= di < s.levels[0].len() && new0[i] == s.levels[0][di])) by {
                    put_prov(s.levels[0], e0);
                }
                if new0[i] == e0 {
                    assert(new0[i].seq == s.next_seq);
                } else {
                    let di = choose|di: int| 0 <= di < s.levels[0].len()
                        && new0[i] == s.levels[0][di];
                    assert(out.levels[li][i].seq == s.levels[0][di].seq);
                    assert(s.levels[0][di].seq < s.next_seq);
                }
                assert(s.next_seq < out.next_seq);
            } else {
                assert(out.levels[li][i] === s.levels[li][i]);
            }
        }
    }
    // clause 3: recency
    assert forall|li: int, lj: int, i: int, j: int|
        0 <= li < lj < out.levels.len() && 0 <= i < out.levels[li].len()
        && 0 <= j < out.levels[lj].len() && out.levels[li][i].key == out.levels[lj][j].key
        implies out.levels[li][i].seq > out.levels[lj][j].seq by {
        if 0 <= li < lj < out.levels.len() && 0 <= i < out.levels[li].len()
            && 0 <= j < out.levels[lj].len()
            && out.levels[li][i].key == out.levels[lj][j].key {
            if li == 0 {
                assert(out.levels[0] === new0);
                assert(new0[i] == e0 || (exists|di: int|
                    0 <= di < s.levels[0].len() && new0[i] == s.levels[0][di])) by {
                    put_prov(s.levels[0], e0);
                }
                assert(out.levels[lj][j] === s.levels[lj][j]);
                if new0[i] == e0 {
                    assert(new0[i].seq == s.next_seq);
                    assert(s.levels[lj][j].seq < s.next_seq);
                } else {
                    assert(exists|di: int| 0 <= di < s.levels[0].len()
                        && new0[i] == s.levels[0][di]) by {
                        put_prov(s.levels[0], e0);
                    }
                    let di = choose|di: int| 0 <= di < s.levels[0].len()
                        && new0[i] == s.levels[0][di];
                    distinct_first_version(s.levels[0], di);
                    assert(s.levels[lj][j].key == s.levels[0][di].key);
                    assert(s.levels[0][di].seq > s.levels[lj][j].seq);
                }
            } else {
                assert(out.levels[li][i] === s.levels[li][i]);
                assert(out.levels[lj][j] === s.levels[lj][j]);
            }
        }
    }
}

/// Inv-LSM is preserved by flush (level 0 merges into level 1 keeping
/// the newer version; every survivor provably came from level 0 or 1).
proof fn flush_preserves_inv(s: LsmState)
    requires
        inv_lsm_spec(s),
        s.levels.len() >= 2,
    ensures
        inv_lsm_spec(flush_spec(s)),
{
    let out = flush_spec(s);
    let merged = merge_step(s.levels[1], s.levels[0], 0, false);
    assert(out.levels[0].len() == 0);
    assert(out.levels[1] === merged);
    merge_distinct(s.levels[1], s.levels[0], 0, false);
    assert forall|oi: int| 0 <= oi < s.levels[0].len() ==> prov_in(s, 0, 2, s.levels[0][oi]) by {
        if 0 <= oi < s.levels[0].len() {
            prov_in_at(s, 0, 0, 2, s.levels[0][oi]);
        }
    }
    assert forall|oi: int| 0 <= oi < s.levels[1].len() ==> prov_in(s, 0, 2, s.levels[1][oi]) by {
        if 0 <= oi < s.levels[1].len() {
            prov_in_at(s, 1, 0, 2, s.levels[1][oi]);
        }
    }
    merge_prov(s.levels[1], s.levels[0], 0, false, s, 0, 2);
    assert forall|oi: int| 0 <= oi < merged.len() ==> prov_in(s, 0, 2, merged[oi]) by {
        if 0 <= oi < merged.len() {
            merge_prov(s.levels[1], s.levels[0], 0, false, s, 0, 2);
        }
    }

    // clause 1
    assert forall|g: int| 0 <= g < out.levels.len() implies distinct_spec(out.levels[g]) by {
        if 0 <= g < out.levels.len() {
            if g == 1 {
                merge_distinct(s.levels[1], s.levels[0], 0, false);
                assert(distinct_spec(out.levels[1]));
            } else if g == 0 {
                assert(out.levels[0].len() == 0);
            } else {
                assert(out.levels[g] === s.levels[g]);
            }
        }
    }
    // clause 2
    assert forall|li: int, i: int|
        0 <= li < out.levels.len() && 0 <= i < out.levels[li].len()
        implies out.levels[li][i].seq < out.next_seq by {
        if 0 <= li < out.levels.len() && 0 <= i < out.levels[li].len() {
            if li == 1 {
                assert(prov_in(s, 0, 2, out.levels[1][i]));
                let l = choose|l: int| 0 <= l < 2 && entry_of_level(s, l, out.levels[1][i]);
                let ii = choose|ii: int| 0 <= ii < s.levels[l].len()
                    && s.levels[l][ii] == out.levels[1][i];
                assert(out.levels[1][i].seq == s.levels[l][ii].seq);
            } else {
                assert(out.levels[li][i] === s.levels[li][i]);
            }
        }
    }
    // clause 3
    assert forall|li: int, lj: int, i: int, j: int|
        0 <= li < lj < out.levels.len() && 0 <= i < out.levels[li].len()
        && 0 <= j < out.levels[lj].len() && out.levels[li][i].key == out.levels[lj][j].key
        implies out.levels[li][i].seq > out.levels[lj][j].seq by {
        if 0 <= li < lj < out.levels.len() && 0 <= i < out.levels[li].len()
            && 0 <= j < out.levels[lj].len()
            && out.levels[li][i].key == out.levels[lj][j].key {
            if li == 1 {
                assert(prov_in(s, 0, 2, out.levels[1][i]));
                let l = choose|l: int| 0 <= l < 2 && entry_of_level(s, l, out.levels[1][i]);
                let ii = choose|ii: int| 0 <= ii < s.levels[l].len()
                    && s.levels[l][ii] == out.levels[1][i];
                distinct_first_version(s.levels[l], ii);
                assert(version_rec(s.levels[l], 0, out.levels[1][i].key)
                    == Some(s.levels[l][ii]));
                assert(out.levels[lj][j] === s.levels[lj][j]);
                assert(s.levels[l][ii].seq > s.levels[lj][j].seq);
                assert(out.levels[1][i].seq == s.levels[l][ii].seq);
            } else if li == 0 {
                assert(out.levels[0].len() == 0);
            }
        }
    }
}

proof fn fold_len(s: LsmState, depth: int, li: int, always_drop: bool)
    requires
        0 <= li <= depth < s.levels.len(),
    ensures
        compact_fold_flag(s, depth, li, always_drop).levels.len() == s.levels.len(),
        compact_fold_flag(s, depth, li, always_drop).next_seq == s.next_seq,
    decreases depth - li,
{
    if li >= depth {
    } else {
        fold_len(s, depth, li + 1, always_drop);
    }
}

/// The fold never mints or loses sequence numbers.
proof fn fold_next_seq(s: LsmState, depth: int, li: int, always_drop: bool)
    requires
        0 <= li <= depth < s.levels.len(),
    ensures
        compact_fold_flag(s, depth, li, always_drop).next_seq == s.next_seq,
    decreases depth - li,
{
    if li >= depth {
    } else {
        fold_next_seq(s, depth, li + 1, always_drop);
    }
}

proof fn fold_untouched(s: LsmState, depth: int, li: int, always_drop: bool)
    requires
        0 <= li <= depth < s.levels.len(),
    ensures
        forall|g: int| 0 <= g < li
            ==> compact_fold_flag(s, depth, li, always_drop).levels[g] === s.levels[g],
    decreases depth - li,
{
    if li >= depth {
    } else {
        fold_len(s, depth, li + 1, always_drop);
        fold_untouched(s, depth, li + 1, always_drop);
        let base = compact_fold_flag(s, depth, li + 1, always_drop);
        let out = compact_fold_flag(s, depth, li, always_drop);
        assert(out.levels === base.levels.update(li, Seq::empty()).update(
            depth,
            merge_step(base.levels[depth], base.levels[li], 0, always_drop
                || depth == s.levels.len() - 1),
        ));
        assert forall|g: int| 0 <= g < li implies out.levels[g] === s.levels[g] by {
            if 0 <= g < li {
                assert(li != depth);
                assert(out.levels[g] === base.levels[g]);
                assert(base.levels[g] === s.levels[g]);
            }
        }
    }
}

proof fn fold_emptied(s: LsmState, depth: int, li: int, always_drop: bool)
    requires
        0 <= li <= depth < s.levels.len(),
    ensures
        forall|g: int| li <= g < depth
            ==> compact_fold_flag(s, depth, li, always_drop).levels[g].len() == 0,
    decreases depth - li,
{
    if li >= depth {
    } else {
        fold_len(s, depth, li + 1, always_drop);
        fold_emptied(s, depth, li + 1, always_drop);
        let base = compact_fold_flag(s, depth, li + 1, always_drop);
        let out = compact_fold_flag(s, depth, li, always_drop);
        assert(out.levels === base.levels.update(li, Seq::empty()).update(
            depth,
            merge_step(base.levels[depth], base.levels[li], 0, always_drop
                || depth == s.levels.len() - 1),
        ));
        assert(li != depth);
        assert(out.levels[li].len() == 0);
        assert forall|g: int| li + 1 <= g < depth implies out.levels[g].len() == 0 by {
            if li + 1 <= g < depth {
                assert(g != depth);
                assert(out.levels[g] === base.levels[g]);
                assert(base.levels[g].len() == 0);
            }
        }
    }
}

/// Levels ABOVE the fold depth are never touched by the fold.
proof fn fold_above(s: LsmState, depth: int, li: int, always_drop: bool)
    requires
        0 <= li <= depth < s.levels.len(),
    ensures
        forall|g: int| depth < g < s.levels.len()
            ==> compact_fold_flag(s, depth, li, always_drop).levels[g] === s.levels[g],
    decreases depth - li,
{
    if li >= depth {
        assert(compact_fold_flag(s, depth, li, always_drop).levels === s.levels);
    } else {
        fold_above(s, depth, li + 1, always_drop);
        let base = compact_fold_flag(s, depth, li + 1, always_drop);
        let out = compact_fold_flag(s, depth, li, always_drop);
        assert(out.levels === base.levels.update(li, Seq::empty()).update(
            depth,
            merge_step(base.levels[depth], base.levels[li], 0, always_drop
                || depth == s.levels.len() - 1),
        ));
        assert forall|g: int| depth < g < s.levels.len()
            implies out.levels[g] === s.levels[g] by {
            if depth < g < s.levels.len() {
                assert(g != li);
                assert(g != depth);
                assert(out.levels[g] === base.levels[g]);
                assert(base.levels[g] === s.levels[g]);
            }
        }
    }
}

proof fn fold_distinct_depth(s: LsmState, depth: int, li: int, always_drop: bool)
    requires
        inv_lsm_spec(s),
        0 <= li <= depth < s.levels.len(),
    ensures
        distinct_spec(compact_fold_flag(s, depth, li, always_drop).levels[depth]),
    decreases depth - li,
{
    if li >= depth {
        assert(compact_fold_flag(s, depth, li, always_drop).levels === s.levels);
    } else {
        fold_len(s, depth, li + 1, always_drop);
        fold_distinct_depth(s, depth, li + 1, always_drop);
        fold_untouched(s, depth, li + 1, always_drop);
        let base = compact_fold_flag(s, depth, li + 1, always_drop);
        let drop = always_drop || depth == s.levels.len() - 1;
        assert(base.levels[li] === s.levels[li]);
        assert(distinct_spec(base.levels[depth]));
        assert(distinct_spec(base.levels[li]));
        merge_distinct(base.levels[depth], base.levels[li], 0, drop);
        let out = compact_fold_flag(s, depth, li, always_drop);
        let b0 = base.levels.update(li, Seq::empty());
        let chain = b0.update(depth, merge_step(base.levels[depth], base.levels[li], 0, drop));
        assert(out.levels === chain);
        assert(li != depth);
        assert(b0.len() == base.levels.len());
        assert(base.levels.len() == s.levels.len());
        assert(0 <= depth < b0.len());
        assert(chain[depth] == merge_step(base.levels[depth], base.levels[li], 0, drop));
        assert(out.levels[depth] === merge_step(base.levels[depth], base.levels[li], 0, drop));
    }
}

proof fn fold_prov(s: LsmState, depth: int, li: int, always_drop: bool)
    requires
        inv_lsm_spec(s),
        0 <= li <= depth < s.levels.len(),
    ensures
        forall|oi: int|
            0 <= oi < compact_fold_flag(s, depth, li, always_drop).levels[depth].len()
            ==> prov_in(
                s,
                li,
                depth + 1,
                compact_fold_flag(s, depth, li, always_drop).levels[depth][oi],
            ),
    decreases depth - li,
{
    if li >= depth {
        assert(compact_fold_flag(s, depth, li, always_drop).levels === s.levels);
        assert forall|oi: int| 0 <= oi < s.levels[depth].len()
            implies prov_in(s, li, depth + 1, s.levels[depth][oi]) by {
            if 0 <= oi < s.levels[depth].len() {
                prov_in_at(s, depth, li, depth + 1, s.levels[depth][oi]);
            }
        }
    } else {
        fold_len(s, depth, li + 1, always_drop);
        fold_prov(s, depth, li + 1, always_drop);
        fold_untouched(s, depth, li + 1, always_drop);
        let base = compact_fold_flag(s, depth, li + 1, always_drop);
        let drop = always_drop || depth == s.levels.len() - 1;
        assert(base.levels[li] === s.levels[li]);
        assert forall|oi: int| 0 <= oi < base.levels[depth].len()
            ==> prov_in(s, li + 1, depth + 1, base.levels[depth][oi]) by {
            if 0 <= oi < base.levels[depth].len() {
                fold_prov(s, depth, li + 1, always_drop);
            }
        }
        assert forall|oi: int| 0 <= oi < base.levels[depth].len()
            ==> prov_in(s, li, depth + 1, base.levels[depth][oi]) by {
            if 0 <= oi < base.levels[depth].len() {
                prov_in_widen(s, li + 1, li, depth + 1, base.levels[depth][oi]);
            }
        }
        assert forall|oi: int| 0 <= oi < s.levels[li].len()
            ==> prov_in(s, li, depth + 1, s.levels[li][oi]) by {
            if 0 <= oi < s.levels[li].len() {
                prov_in_at(s, li, li, depth + 1, s.levels[li][oi]);
            }
        }
        merge_prov(base.levels[depth], s.levels[li], 0, drop, s, li, depth + 1);
        let merged = merge_step(base.levels[depth], s.levels[li], 0, drop);
        let out = compact_fold_flag(s, depth, li, always_drop);
        let b0 = base.levels.update(li, Seq::empty());
        let chain = b0.update(depth, merged);
        assert(out.levels === chain);
        assert(li != depth);
        assert(b0.len() == base.levels.len());
        assert(base.levels.len() == s.levels.len());
        assert(0 <= depth < b0.len());
        assert(chain[depth] == merged);
        assert(out.levels[depth] === merged);
    }
}

/// Inv-LSM is preserved by compact (fold 0..=depth, newest wins,
/// tombstones retire at the bottom level).
proof fn compact_preserves_inv(s: LsmState, depth: int)
    requires
        inv_lsm_spec(s),
        1 <= depth < s.levels.len(),
    ensures
        inv_lsm_spec(compact_spec(s, depth)),
{
    fold_len(s, depth, 0, false);
    fold_untouched(s, depth, 0, false);
    fold_emptied(s, depth, 0, false);
    fold_above(s, depth, 0, false);
    fold_distinct_depth(s, depth, 0, false);
    fold_prov(s, depth, 0, false);
    let out = compact_spec(s, depth);

    // clause 1: distinct keys per level
    assert forall|g: int| 0 <= g < out.levels.len() implies distinct_spec(out.levels[g]) by {
        if 0 <= g < out.levels.len() {
            if g < depth {
                assert(out.levels[g].len() == 0);
            } else if g == depth {
                fold_distinct_depth(s, depth, 0, false);
                assert(distinct_spec(out.levels[depth]));
            } else {
                assert(out.levels[g] === s.levels[g]);
            }
        }
    }
    // clause 2: every live seq < next_seq
    assert forall|li: int, i: int|
        0 <= li < out.levels.len() && 0 <= i < out.levels[li].len()
        implies out.levels[li][i].seq < out.next_seq by {
        if 0 <= li < out.levels.len() && 0 <= i < out.levels[li].len() {
            if li < depth {
                assert(out.levels[li].len() == 0);
            } else if li == depth {
                assert(prov_in(s, 0, depth + 1, out.levels[depth][i]));
                let l = choose|l: int| 0 <= l < depth + 1
                    && entry_of_level(s, l, out.levels[depth][i]);
                let ii = choose|ii: int| 0 <= ii < s.levels[l].len()
                    && s.levels[l][ii] == out.levels[depth][i];
                assert(out.levels[depth][i].seq == s.levels[l][ii].seq);
            } else {
                assert(out.levels[li][i] === s.levels[li][i]);
            }
        }
    }
    // clause 3: recency
    assert forall|li: int, lj: int, i: int, j: int|
        0 <= li < lj < out.levels.len() && 0 <= i < out.levels[li].len()
        && 0 <= j < out.levels[lj].len() && out.levels[li][i].key == out.levels[lj][j].key
        implies out.levels[li][i].seq > out.levels[lj][j].seq by {
        if 0 <= li < lj < out.levels.len() && 0 <= i < out.levels[li].len()
            && 0 <= j < out.levels[lj].len()
            && out.levels[li][i].key == out.levels[lj][j].key {
            if li < depth {
                assert(out.levels[li].len() == 0);
            } else if li == depth {
                assert(prov_in(s, 0, depth + 1, out.levels[depth][i]));
                let l = choose|l: int| 0 <= l < depth + 1
                    && entry_of_level(s, l, out.levels[depth][i]);
                let ii = choose|ii: int| 0 <= ii < s.levels[l].len()
                    && s.levels[l][ii] == out.levels[depth][i];
                distinct_first_version(s.levels[l], ii);
                assert(version_rec(s.levels[l], 0, out.levels[depth][i].key)
                    == Some(s.levels[l][ii]));
                assert(out.levels[depth][i].key == s.levels[l][ii].key);
                assert(out.levels[lj][j] === s.levels[lj][j]);
                assert(out.levels[depth][i].key == s.levels[lj][j].key);
                assert(s.levels[l][ii].key == s.levels[lj][j].key);
                assert(l < lj);
                assert(s.levels[l][ii].seq > s.levels[lj][j].seq);
                assert(out.levels[depth][i].seq == s.levels[l][ii].seq);
            } else {
                assert(out.levels[li][i] === s.levels[li][i]);
                assert(out.levels[lj][j] === s.levels[lj][j]);
            }
        }
    }
}

/// Inv-LSM is preserved by reopen (the durable order rebuilds the same
/// probe order — identity in the model).
proof fn reopen_preserves_inv(s: LsmState)
    requires
        inv_lsm_spec(s),
    ensures
        inv_lsm_spec(reopen_spec(s)),
{
}

// --- exec==spec teeth -------------------------------------------------------

/// Spec view of a Vec of levels.
pub open spec fn views_of(levels: Seq<Vec<Entry>>) -> Seq<Seq<Entry>> {
    Seq::new(levels.len(), |i: int| levels[i]@)
}

pub fn version_exec(level: &Vec<Entry>, i: usize, key: u64) -> (r: Option<Entry>)
    requires
        i <= level.len(),
    ensures
        r == version_rec(level@, i as int, key)
    decreases level.len() - i,
{
    if i >= level.len() {
        None
    } else {
        let e = level[i];
        if e.key == key {
            Some(e)
        } else {
            version_exec(level, (i + 1) as usize, key)
        }
    }
}

pub fn probe_exec(levels: &Vec<Vec<Entry>>, li: usize, key: u64) -> (r: Option<Entry>)
    requires
        li <= levels.len(),
    ensures
        r == probe_rec(views_of(levels@), li as int, key)
    decreases levels.len() - li,
{
    if li >= levels.len() {
        None
    } else {
        let lvl: Vec<Entry> = levels[li].clone();
        assert(views_of(levels@)[li as int] === levels@[li as int]@);
        assert(views_of(levels@)[li as int] === lvl@);
        match version_exec(&lvl, 0, key) {
            Some(e) => Some(e),
            None => probe_exec(levels, (li + 1) as usize, key),
        }
    }
}

/// Spec view of a Vec-of-Vecs state at `next_seq`.
pub open spec fn state_view(levels: &Vec<Vec<Entry>>, next_seq: u64) -> LsmState {
    LsmState {
        levels: views_of(levels@),
        next_seq,
    }
}

/// find_key skips a non-matching prefix.
proof fn find_key_skip_prefix(level: Seq<Entry>, i: int, key: u64)
    requires
        0 <= i <= level.len(),
        forall|k: int| 0 <= k < i ==> level[k].key != key,
    ensures
        find_key(level, 0, key) == find_key(level, i, key),
    decreases i,
{
    if i == 0 {
    } else {
        find_key_skip_prefix(level, i - 1, key);
        assert(level[i - 1].key != key);
        assert(find_key(level, i - 1, key) == find_key(level, i, key));
    }
}

pub fn put_exec(level: &Vec<Entry>, e: Entry) -> (out: Vec<Entry>)
    ensures
        out@ === put_level(level@, e),
{
    let mut i: usize = 0;
    while i < level.len()
        invariant
            0 <= i <= level.len(),
            forall|k: int| 0 <= k < i ==> level@[k].key != e.key,
        decreases level.len() - i,
    {
        if level[i].key == e.key {
            assert(find_key(level@, 0, e.key) == Some(i as int)) by {
                find_key_skip_prefix(level@, i as int, e.key);
                assert(find_key(level@, i as int, e.key) == Some(i as int));
            }
            let mut out = level.clone();
            out.set(i, e);
            assert(out@ === level@.update(i as int, e));
            assert(put_level(level@, e) === out@);
            return out;
        }
        i += 1;
    }
    assert(find_key(level@, 0, e.key) == None) by {
        find_key_skip_prefix(level@, level.len() as int, e.key);
        assert(find_key(level@, level.len() as int, e.key) == None);
    }
    let mut out = level.clone();
    out.push(e);
    assert(out@ === level@.push(e));
    assert(put_level(level@, e) === out@);
    out
}

/// Order-free removal matching the spec recursion (survivors pushed
/// deepest-first, so the result lists them reversed).
pub fn remove_exec(level: &Vec<Entry>, key: u64) -> (out: Vec<Entry>)
    ensures
        out@ === remove_key_rec(level@, 0, key),
{
    let mut out: Vec<Entry> = Vec::new();
    let mut i: usize = level.len();
    while i > 0
        invariant
            0 <= i <= level.len(),
            out@ === remove_key_rec(level@, i as int, key),
        decreases i,
    {
        i -= 1;
        let e = level[i];
        if e.key != key {
            out.push(e);
            assert(out@ === remove_key_rec(level@, (i + 1) as int, key).push(level@[i as int]));
            assert(out@ === remove_key_rec(level@, i as int, key));
        } else {
            assert(out@ === remove_key_rec(level@, i as int, key));
        }
    }
    out
}

pub fn merge_exec(dst: &Vec<Entry>, src: &Vec<Entry>, drop_tomb: bool) -> (out: Vec<Entry>)
    ensures
        out@ === merge_step(dst@, src@, 0, drop_tomb),
{
    let mut acc: Vec<Entry> = dst.clone();
    assert(acc@ === dst@);
    let mut i: usize = 0;
    while i < src.len()
        invariant
            0 <= i <= src.len(),
            merge_step(dst@, src@, 0, drop_tomb) === merge_step(acc@, src@, i as int, drop_tomb),
        decreases src.len() - i,
    {
        let e = src[i];
        if drop_tomb && e.tomb {
            let next = remove_exec(&acc, e.key);
            assert(merge_step(acc@, src@, i as int, drop_tomb)
                === merge_step(remove_key_rec(acc@, 0, e.key), src@, (i + 1) as int, drop_tomb));
            assert(merge_step(dst@, src@, 0, drop_tomb)
                === merge_step(next@, src@, (i + 1) as int, drop_tomb));
            acc = next;
        } else {
            let next = put_exec(&acc, e);
            assert(merge_step(acc@, src@, i as int, drop_tomb)
                === merge_step(put_level(acc@, e), src@, (i + 1) as int, drop_tomb));
            assert(merge_step(dst@, src@, 0, drop_tomb)
                === merge_step(next@, src@, (i + 1) as int, drop_tomb));
            acc = next;
        }
        i += 1;
    }
    assert(merge_step(dst@, src@, 0, drop_tomb)
        === merge_step(acc@, src@, src.len() as int, drop_tomb));
    assert(merge_step(acc@, src@, src.len() as int, drop_tomb) === acc@);
    acc
}

pub fn newest_exec(levels: &Vec<Vec<Entry>>, li: usize, key: u64) -> (r: Option<Entry>)
    requires
        li <= levels.len(),
    ensures
        r == newest_rec(views_of(levels@), li as int, key)
    decreases levels.len() - li,
{
    if li >= levels.len() {
        None
    } else {
        let lvl: Vec<Entry> = levels[li].clone();
        assert(views_of(levels@)[li as int] === lvl@);
        match version_exec(&lvl, 0, key) {
            Some(e) => match newest_exec(levels, (li + 1) as usize, key) {
                Some(b) => if b.seq > e.seq {
                    Some(b)
                } else {
                    Some(e)
                },
                None => Some(e),
            },
            None => newest_exec(levels, (li + 1) as usize, key),
        }
    }
}

/// Exec tooth for the probe entry: the recency walk on a Vec state.
pub fn lsm_probe(levels: &Vec<Vec<Entry>>, key: u64) -> (r: Option<Entry>)
    ensures
        r == probe_spec(state_view(levels, 0), key),
{
    probe_exec(levels, 0, key)
}

/// Exec tooth for the reopen entry: identity on the durable order.
pub fn lsm_reopen(levels: &Vec<Vec<Entry>>) -> (out: Vec<Vec<Entry>>)
    ensures
        views_of(out@) === views_of(levels@),
{
    let mut out: Vec<Vec<Entry>> = Vec::new();
    let mut i: usize = 0;
    while i < levels.len()
        invariant
            i <= levels.len(),
            out.len() == i,
            forall|k: int| 0 <= k < i ==> views_of(out@)[k] === views_of(levels@)[k],
        decreases levels.len() - i,
    {
        let lvl: Vec<Entry> = levels[i].clone();
        assert(views_of(levels@)[i as int] === levels@[i as int]@);
        assert(views_of(levels@)[i as int] === lvl@);
        out.push(lvl);
        assert(views_of(out@)[i as int] === out@[i as int]@);
        assert(out@[i as int]@ === lvl@);
        i += 1;
    }
    assert(views_of(out@).len() == out.len());
    assert(views_of(levels@).len() == levels.len());
    assert(views_of(out@) === views_of(levels@));
    out
}

/// Exec tooth for the compact entry: the deepest-first fold with the
/// bottom-level tombstone retire rule (the fold order the real kernel
/// fixed — shallow-first let old versions win).
pub fn lsm_compact(
    levels: &Vec<Vec<Entry>>,
    depth: usize,
    next_seq: u64,
) -> (out: Vec<Vec<Entry>>)
    requires
        1 <= depth < levels.len(),
    ensures
        state_view(&out, next_seq) === compact_spec(state_view(levels, next_seq), depth as int),
{
    let mut out: Vec<Vec<Entry>> = Vec::new();
    let mut j: usize = 0;
    while j < levels.len()
        invariant
            j <= levels.len(),
            out.len() == j,
            forall|k: int| 0 <= k < j ==> views_of(out@)[k] === views_of(levels@)[k],
        decreases levels.len() - j,
    {
        let lvl: Vec<Entry> = levels[j].clone();
        assert(views_of(levels@)[j as int] === levels@[j as int]@);
        assert(views_of(levels@)[j as int] === lvl@);
        out.push(lvl);
        assert(views_of(out@)[j as int] === out@[j as int]@);
        assert(out@[j as int]@ === lvl@);
        j += 1;
    }
    assert(views_of(out@).len() == out.len());
    assert(views_of(out@) === views_of(levels@));
    let drop_tomb = depth == levels.len() - 1;
    assert(views_of(levels@).len() == levels.len());
    assert(drop_tomb == (false || depth == views_of(levels@).len() - 1));
    let mut li: usize = depth;
    while li > 0
        invariant
            0 <= li <= depth < levels.len(),
            out.len() == levels.len(),
            views_of(levels@).len() == levels.len(),
            drop_tomb == (false || depth == views_of(levels@).len() - 1),
            views_of(out@) === compact_fold_flag(
                state_view(levels, next_seq),
                depth as int,
                li as int,
                false,
            ).levels,
        decreases li,
    {
        li -= 1;
        // facts about the pre-iteration view (the invariant at li + 1)
        assert(views_of(out@)[depth as int] === compact_fold_flag(
            state_view(levels, next_seq),
            depth as int,
            (li + 1) as int,
            false,
        ).levels[depth as int]);
        assert(compact_fold_flag(state_view(levels, next_seq), depth as int, (li + 1) as int, false)
            .levels[li as int] === views_of(levels@)[li as int]) by {
            fold_untouched(state_view(levels, next_seq), depth as int, (li + 1) as int, false);
        }
        let src: Vec<Entry> = out[li].clone();
        assert(views_of(out@)[li as int] === src@);
        assert(src@ === views_of(levels@)[li as int]);
        out.set(li, Vec::new());
        assert(views_of(out@)[depth as int] === compact_fold_flag(
            state_view(levels, next_seq),
            depth as int,
            (li + 1) as int,
            false,
        ).levels[depth as int]);
        let merged = merge_exec(&out[depth], &src, drop_tomb);
        assert(merged@ === merge_step(views_of(out@)[depth as int], src@, 0, drop_tomb));
        out.set(depth, merged);
        assert(li != depth);
        assert(views_of(out@).len() == out.len());
        assert(compact_fold_flag(state_view(levels, next_seq), depth as int, li as int, false)
            .levels.len() == levels.len());
        assert(state_view(levels, next_seq).levels === views_of(levels@));
        assert(state_view(levels, next_seq).levels.len() == levels.len());
        assert(li < depth);
        assert(compact_fold_flag(state_view(levels, next_seq), depth as int, li as int, false).levels
            === compact_fold_flag(state_view(levels, next_seq), depth as int, (li + 1) as int, false)
                .levels.update(li as int, Seq::empty())
                .update(depth as int, merge_step(
                    compact_fold_flag(state_view(levels, next_seq), depth as int, (li + 1) as int, false).levels[depth as int],
                    compact_fold_flag(state_view(levels, next_seq), depth as int, (li + 1) as int, false).levels[li as int],
                    0,
                    false || depth == views_of(levels@).len() - 1,
                ))) by {
            assert(state_view(levels, next_seq).levels === views_of(levels@));
        }
        assert forall|g: int| 0 <= g < out.len() implies
            views_of(out@)[g] === compact_fold_flag(
                state_view(levels, next_seq),
                depth as int,
                li as int,
                false,
            ).levels[g] by {
            fold_untouched(state_view(levels, next_seq), depth as int, (li + 1) as int, false);
            fold_above(state_view(levels, next_seq), depth as int, (li + 1) as int, false);
            fold_emptied(state_view(levels, next_seq), depth as int, (li + 1) as int, false);
            if 0 <= g < out.len() {
                if g == li as int {
                    assert(views_of(out@)[g] === Seq::empty());
                } else if g == depth as int {
                    assert(views_of(out@)[g] === merged@);
                } else if g < li as int {
                    assert(views_of(out@)[g] === views_of(levels@)[g]);
                } else if (depth as int) < g {
                    assert(views_of(out@)[g] === views_of(levels@)[g]);
                } else {
                    assert((li as int) < g);
                    assert(g < depth as int);
                    assert(views_of(out@)[g] === compact_fold_flag(
                        state_view(levels, next_seq),
                        depth as int,
                        (li + 1) as int,
                        false,
                    ).levels[g]);
                    assert(views_of(out@)[g].len() == 0);
                }
            }
        }
        assert(views_of(out@) === compact_fold_flag(
            state_view(levels, next_seq),
            depth as int,
            li as int,
            false,
        ).levels);
    }
    assert(li == 0);
    assert(views_of(out@) === compact_fold_flag(
        state_view(levels, next_seq),
        depth as int,
        0,
        false,
    ).levels);
    assert(compact_spec(state_view(levels, next_seq), depth as int)
        === compact_fold_flag(state_view(levels, next_seq), depth as int, 0, false));
    assert(compact_fold_flag(state_view(levels, next_seq), depth as int, 0, false).next_seq
        == next_seq) by {
        fold_next_seq(state_view(levels, next_seq), depth as int, 0, false);
    }
    assert(state_view(&out, next_seq) === LsmState {
        levels: compact_fold_flag(state_view(levels, next_seq), depth as int, 0, false).levels,
        next_seq,
    });
    assert(state_view(&out, next_seq) === compact_spec(state_view(levels, next_seq), depth as int));
    out
}

/// Exec tooth for the R1 corollary: on an Inv-LSM state the probe
/// answers the newest version.
pub open spec fn r1_modelo_spec(s: LsmState, key: u64) -> bool {
    !inv_lsm_spec(s) || probe_spec(s, key) == newest_spec(s, key)
}

pub fn r1_modelo(levels: &Vec<Vec<Entry>>, next_seq: u64, key: u64) -> (b: bool)
    requires
        inv_lsm_spec(state_view(levels, next_seq)),
    ensures
        b == r1_modelo_spec(state_view(levels, next_seq), key),
{
    assert(r1_modelo_spec(state_view(levels, next_seq), key)) by {
        r1_theorem(state_view(levels, next_seq), key);
        assert(probe_spec(state_view(levels, next_seq), key)
            == newest_spec(state_view(levels, next_seq), key));
        assert(!inv_lsm_spec(state_view(levels, next_seq))
            || probe_spec(state_view(levels, next_seq), key)
                == newest_spec(state_view(levels, next_seq), key));
    };
    true
}

// --- AS-IS teeth (witnesses on the atom-reachable resurrect shape) ----------

/// The delete-resurrect shape reachable by atoms: put → flush → compact
/// to level 2 → delete. Level 0 holds the tombstone (newest); level 2
/// the old value.
pub open spec fn seq1(e: Entry) -> Seq<Entry> {
    Seq::empty().push(e)
}

pub open spec fn resurrect_state() -> LsmState {
    LsmState {
        levels: Seq::empty()
            .push(seq1(Entry {
                key: 7,
                seq: 3,
                tomb: true,
            }))
            .push(Seq::empty())
            .push(seq1(Entry {
                key: 7,
                seq: 1,
                tomb: false,
            }))
            .push(Seq::empty()),
        next_seq: 4,
    }
}

/// AS-IS probe: deepest-first walk — the descending-lo order of
/// findings/2026-09-04-reopen-delete-resurrected.
pub open spec fn probe_as_is_rec(levels: Seq<Seq<Entry>>, li: int, key: u64) -> Option<Entry>
    decreases li + 1,
{
    if li < 0 {
        None
    } else {
        match version_rec(levels[li], 0, key) {
            Some(e) => Some(e),
            None => probe_as_is_rec(levels, li - 1, key),
        }
    }
}

pub open spec fn probe_as_is_spec(s: LsmState, key: u64) -> Option<Entry> {
    probe_as_is_rec(s.levels, s.levels.len() - 1, key)
}

proof fn probe_as_is_resurrects()
    ensures
        ({
            let s = resurrect_state();
            inv_lsm_spec(s)
                && probe_spec(s, 7) == Some(Entry {
                    key: 7,
                    seq: 3,
                    tomb: true,
                })
                && probe_as_is_spec(s, 7) == Some(Entry {
                    key: 7,
                    seq: 1,
                    tomb: false,
                })
        }),
{
    let s = resurrect_state();
    let ls = s.levels;
    assert(ls.len() == 4);
    let t = Entry {
        key: 7,
        seq: 3,
        tomb: true,
    };
    let v = Entry {
        key: 7,
        seq: 1,
        tomb: false,
    };
    assert(ls[0] === seq1(t));
    assert(ls[1] === Seq::empty());
    assert(ls[2] === seq1(v));
    assert(ls[3] === Seq::empty());
    assert(version_rec(ls[0], 0, 7) == Some(t));
    assert(version_rec(ls[2], 0, 7) == Some(v));
    assert(version_rec(ls[3], 0, 7) == None);
    assert(probe_rec(ls, 0, 7) == Some(t));
    assert(probe_as_is_rec(ls, 3, 7) == probe_as_is_rec(ls, 2, 7));
    assert(probe_as_is_rec(ls, 2, 7) == Some(v));
    assert(probe_as_is_rec(ls, 3, 7) == Some(v));
    assert(inv_lsm_spec(s));
}

proof fn compact_as_is_resurrects()
    ensures
        ({
            let s = resurrect_state();
            let honest = compact_spec(s, 1);
            let bad = compact_as_is_spec(s, 1);
            inv_lsm_spec(s)
                && probe_spec(honest, 7) == Some(Entry {
                    key: 7,
                    seq: 3,
                    tomb: true,
                })
                && probe_spec(bad, 7) == Some(Entry {
                    key: 7,
                    seq: 1,
                    tomb: false,
                })
        }),
{
    let s = resurrect_state();
    let ls = s.levels;
    assert(ls.len() == 4);
    let t = Entry {
        key: 7,
        seq: 3,
        tomb: true,
    };
    let v = Entry {
        key: 7,
        seq: 1,
        tomb: false,
    };
    assert(ls[0] === seq1(t));
    assert(ls[1] === Seq::empty());
    assert(ls[2] === seq1(v));
    assert(ls[3] === Seq::empty());
    // both folds stop immediately at li == 1 >= depth == 1
    assert(compact_fold_flag(s, 1, 1, false) === s);
    assert(compact_fold_flag(s, 1, 1, true) === s);
    // honest: the tombstone lands in level 1
    let merged_honest = merge_step(ls[1], ls[0], 0, false);
    let d1 = put_level(ls[1], t);
    assert(merged_honest === merge_step(d1, ls[0], 1, false));
    assert(merge_step(d1, ls[0], 1, false) === d1);
    assert(merged_honest === d1);
    assert(find_key(ls[1], 0, 7) == None);
    assert(d1 === seq1(t));
    let honest = compact_spec(s, 1);
    assert(honest.levels === ls.update(0, Seq::empty()).update(1, merged_honest));
    assert(honest.levels[0].len() == 0);
    assert(honest.levels[1] === merged_honest);
    assert(honest.levels[2] === ls[2]);
    assert(version_rec(honest.levels[0], 0, 7) == None);
    assert(version_rec(honest.levels[1], 0, 7) == Some(t));
    assert(probe_rec(honest.levels, 0, 7) == probe_rec(honest.levels, 1, 7));
    assert(probe_rec(honest.levels, 1, 7) == Some(t));
    assert(probe_rec(honest.levels, 0, 7) == Some(t));
    // as-is: the tombstone is dropped; level 2 survives
    let merged_bad = merge_step(ls[1], ls[0], 0, true);
    let r1 = remove_key_rec(ls[1], 0, 7);
    assert(r1 === Seq::empty());
    assert(merged_bad === merge_step(r1, ls[0], 1, true));
    assert(merge_step(r1, ls[0], 1, true) === r1);
    assert(merged_bad === r1);
    let bad = compact_as_is_spec(s, 1);
    assert(bad.levels === ls.update(0, Seq::empty()).update(1, merged_bad));
    assert(bad.levels[0].len() == 0);
    assert(bad.levels[1].len() == 0);
    assert(bad.levels[2] === ls[2]);
    assert(version_rec(bad.levels[0], 0, 7) == None);
    assert(version_rec(bad.levels[1], 0, 7) == None);
    assert(version_rec(bad.levels[2], 0, 7) == Some(v));
    assert(probe_rec(bad.levels, 0, 7) == probe_rec(bad.levels, 1, 7));
    assert(probe_rec(bad.levels, 1, 7) == probe_rec(bad.levels, 2, 7));
    assert(probe_rec(bad.levels, 2, 7) == Some(v));
    assert(probe_rec(bad.levels, 0, 7) == Some(v));
    assert(inv_lsm_spec(s));
}

/// AS-IS reopen: the level stack rebuilds reversed — recency breaks.
pub open spec fn reopen_as_is_spec(s: LsmState) -> LsmState {
    LsmState {
        levels: Seq::new(s.levels.len(), |i: int| s.levels[s.levels.len() - 1 - i]),
        next_seq: s.next_seq,
    }
}

proof fn reopen_as_is_breaks_recency_and_resurrects()
    ensures
        ({
            let s = resurrect_state();
            let bad = reopen_as_is_spec(s);
            !inv_lsm_spec(bad)
                && probe_spec(bad, 7) == Some(Entry {
                    key: 7,
                    seq: 1,
                    tomb: false,
                })
        }),
{
    let s = resurrect_state();
    let ls = s.levels;
    assert(ls.len() == 4);
    let t = Entry {
        key: 7,
        seq: 3,
        tomb: true,
    };
    let v = Entry {
        key: 7,
        seq: 1,
        tomb: false,
    };
    let bad = reopen_as_is_spec(s);
    assert(bad.levels.len() == 4);
    assert(bad.levels[0] === ls[3]);
    assert(bad.levels[1] === ls[2]);
    assert(bad.levels[2] === ls[1]);
    assert(bad.levels[3] === ls[0]);
    assert(bad.levels[1][0] == v);
    assert(bad.levels[3][0] == t);
    assert(version_rec(bad.levels[1], 0, 7) == Some(v));
    assert(version_rec(bad.levels[0], 0, 7) == None);
    assert(probe_rec(bad.levels, 0, 7) == probe_rec(bad.levels, 1, 7));
    assert(probe_rec(bad.levels, 1, 7) == Some(v));
    assert(probe_rec(bad.levels, 0, 7) == Some(v));
    assert(!inv_lsm_spec(bad)) by {
        // witness the violated recency pair: levels (1, 3), same key 7
        assert(bad.levels[1][0].key == bad.levels[3][0].key);
        assert(bad.levels[1][0].seq < bad.levels[3][0].seq);
    }
}

/// AS-IS corollary: the deepest-first probe against the same newest —
/// the resurrect state (Inv-LSM intact) makes it FALSE, which is the
/// measured failure shape.
pub open spec fn r1_modelo_as_is_spec(s: LsmState, key: u64) -> bool {
    !inv_lsm_spec(s) || probe_as_is_spec(s, key) == newest_spec(s, key)
}

proof fn r1_modelo_as_is_breaks()
    ensures
        !r1_modelo_as_is_spec(resurrect_state(), 7),
{
    let s = resurrect_state();
    let ls = s.levels;
    assert(ls.len() == 4);
    let t = Entry {
        key: 7,
        seq: 3,
        tomb: true,
    };
    let v = Entry {
        key: 7,
        seq: 1,
        tomb: false,
    };
    assert(inv_lsm_spec(s));
    assert(ls[0] === seq1(t));
    assert(ls[1] === Seq::empty());
    assert(ls[2] === seq1(v));
    assert(ls[3] === Seq::empty());
    assert(version_rec(ls[0], 0, 7) == Some(t));
    assert(version_rec(ls[2], 0, 7) == Some(v));
    assert(version_rec(ls[3], 0, 7) == None);
    assert(probe_as_is_rec(ls, 3, 7) == probe_as_is_rec(ls, 2, 7));
    assert(probe_as_is_rec(ls, 2, 7) == Some(v));
    assert(probe_as_is_spec(s, 7) == Some(v));
    // newest walks 0..1..2..3: t (seq 3) beats v (seq 1).
    assert(version_rec(ls[1], 0, 7) == None);
    assert(newest_rec(ls, 4, 7) == None);
    assert(newest_rec(ls, 3, 7) == None);
    assert(newest_rec(ls, 2, 7) == Some(v));
    assert(newest_rec(ls, 1, 7) == newest_rec(ls, 2, 7));
    assert(newest_rec(ls, 0, 7) == Some(t));
    assert(newest_spec(s, 7) == Some(t));
    assert(probe_as_is_spec(s, 7) != newest_spec(s, 7));
}

} // verus!
