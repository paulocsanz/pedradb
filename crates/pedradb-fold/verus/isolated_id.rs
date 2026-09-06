// Verus proof of F83 isolated-id match (RFC-0002 P29).
// Twin of `src/isolated_kernel.rs` (slice starts_with is caller).
//
//   ./scripts/verus_isolated_id.sh

use vstd::prelude::*;

verus! {

pub open spec fn isolated_child_sep() -> u8 {
    0x2f
}

pub open spec fn isolated_child_byte_spec(next: u8) -> bool {
    next == isolated_child_sep()
}

pub fn isolated_child_byte(next: u8) -> (d: bool)
    ensures
        d == isolated_child_byte_spec(next),
        d == (next == 0x2f),
{
    next == 0x2f
}

pub open spec fn isolated_child_byte_as_is(_next: u8) -> bool {
    true
}

proof fn lemma_slash_is_child()
    ensures
        isolated_child_byte_spec(0x2f),
{
}

proof fn lemma_as_is_leaks_sibling_b()
    ensures
        !isolated_child_byte_spec(0x62),
        isolated_child_byte_as_is(0x62),
{
}

/// Production F83 loop (RFC-0170 close of `isolated_id_matches`).
pub open spec fn isolated_id_matches_spec(key: Seq<u8>, id: Seq<u8>) -> bool {
    id.len() <= key.len()
        && (forall |j: int| 0 <= j && j < id.len() ==> key[j] == id[j])
        && (key.len() == id.len() || key[id.len() as int] == isolated_child_sep())
}

pub open spec fn isolated_id_matches_as_is_spec(key: Seq<u8>, id: Seq<u8>) -> bool {
    id.len() <= key.len()
        && (forall |j: int| 0 <= j && j < id.len() ==> key[j] == id[j])
}

pub fn isolated_id_matches(key: &[u8], id: &[u8]) -> (d: bool)
    ensures
        d == isolated_id_matches_spec(key@, id@),
{
    if key.len() < id.len() {
        assert(!(id.len() <= key.len()));
        return false;
    }
    let mut i: usize = 0;
    while i < id.len()
        invariant
            i <= id.len(),
            id.len() <= key.len(),
            forall |j: int| 0 <= j && j < i ==> key@[j] == id@[j],
        decreases id.len() - i,
    {
        if key[i] != id[i] {
            proof {
                assert(key@[i as int] != id@[i as int]);
                assert(!isolated_id_matches_spec(key@, id@));
            }
            return false;
        }
        i = i + 1;
    }
    key.len() == id.len() || key[i] == 0x2f
}

pub fn isolated_id_matches_as_is(key: &[u8], id: &[u8]) -> (d: bool)
    ensures
        d == isolated_id_matches_as_is_spec(key@, id@),
{
    if key.len() < id.len() {
        return false;
    }
    let mut i: usize = 0;
    while i < id.len()
        invariant
            i <= id.len(),
            id.len() <= key.len(),
            forall |j: int| 0 <= j && j < i ==> key@[j] == id@[j],
        decreases id.len() - i,
    {
        if key[i] != id[i] {
            return false;
        }
        i = i + 1;
    }
    true
}

proof fn lemma_as_is_leaks_sibling_key()
    ensures
        !isolated_id_matches_spec(seq![0x61u8, 0x62u8], seq![0x61u8]),
        isolated_id_matches_as_is_spec(seq![0x61u8, 0x62u8], seq![0x61u8]),
{
    assert(seq![0x61u8, 0x62u8][1] != isolated_child_sep());
}

} // verus!
