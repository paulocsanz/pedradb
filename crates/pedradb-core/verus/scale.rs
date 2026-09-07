// Close twin of scale_kernel (RFC-0176): probes + WARM cap + worst probes.
//
//   ./scripts/verus_scale.sh

use vstd::prelude::*;

verus! {

pub open spec fn probes_spec(levels: int, l0_covering: int) -> int {
    levels + l0_covering
}

pub open spec fn probes_as_is_spec(n_files: int) -> int {
    n_files
}

pub fn point_get_probes(levels: u64, l0_covering: u64) -> (p: u64)
    requires
        levels as int + l0_covering as int <= 0xffff_ffff_ffff_ffffint,
    ensures
        p as int == probes_spec(levels as int, l0_covering as int),
{
    levels + l0_covering
}

pub fn point_get_probes_as_is(n_files: u64, _levels: u64, _l0_covering: u64) -> (p: u64)
    ensures
        p as int == probes_as_is_spec(n_files as int),
{
    n_files
}

pub fn probes_worst(levels: u64, l0_max: u64) -> (p: u64)
    requires
        levels as int + l0_max as int <= 0xffff_ffff_ffff_ffffint,
    ensures
        p as int == probes_spec(levels as int, l0_max as int),
{
    levels + l0_max
}

proof fn lemma_best_probes_le_worst(levels: int, l0_max: int)
    requires
        levels >= 0,
        l0_max >= 1,
    ensures
        probes_spec(levels, 1) <= probes_spec(levels, l0_max),
{
}

proof fn lemma_as_is_walks_every_file(levels: int, l0: int, n_files: int)
    requires
        levels >= 0,
        l0 >= 0,
        n_files > levels + l0,
    ensures
        probes_spec(levels, l0) < probes_as_is_spec(n_files),
{
}

proof fn lemma_ten_billion_is_one_more_level() {
    assert(probes_spec(4, 1) == 5);
    assert(probes_spec(5, 1) == 6);
    assert(probes_as_is_spec(9127) == 9127);
    assert(6 < 9127);
}

pub open spec fn three_gib() -> int {
    3 * 0x4000_0000int
}

pub open spec fn one_gib() -> int {
    0x4000_0000int
}

pub open spec fn warm_cap_spec(ram: int) -> int {
    if ram == 0 {
        three_gib()
    } else {
        let share = ram * 3 / 4;
        let cap = if three_gib() >= share { three_gib() } else { share };
        let reserved = if ram >= one_gib() { ram - one_gib() } else { 0int };
        if cap <= reserved { cap } else { reserved }
    }
}

pub open spec fn warm_cap_as_is_spec(_ram: int) -> int {
    0xffff_ffff_ffff_ffffint
}

pub fn warm_cap_bytes(ram: u64) -> (c: u64)
    requires
        ram < 0x4000_0000_0000_0000u64,
    ensures
        c as int == warm_cap_spec(ram as int),
{
    let three_gib: u64 = 3u64 * 0x4000_0000u64;
    let one_gib: u64 = 0x4000_0000u64;
    if ram == 0 {
        three_gib
    } else {
        let share: u64 = ram * 3u64 / 4u64;
        let cap: u64 = if three_gib >= share { three_gib } else { share };
        let reserved: u64 = if ram >= one_gib { ram - one_gib } else { 0u64 };
        if cap <= reserved { cap } else { reserved }
    }
}

pub fn warm_cap_bytes_as_is(_ram: u64) -> (c: u64)
    ensures
        c as int == warm_cap_as_is_spec(_ram as int),
{
    0xffff_ffff_ffff_ffffu64
}

proof fn lemma_four_gib_cap_is_three_gib() {
    assert(warm_cap_spec(0x1_0000_0000int) == three_gib());
    assert(warm_cap_spec(0x1_0000_0000int) < warm_cap_as_is_spec(0x1_0000_0000int));
}

fn main() {}

}
