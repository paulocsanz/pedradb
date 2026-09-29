// Verus proof of packed-children exclusive end (RFC-0002 P23 / F59).
// Twin of `src/children_kernel.rs` (Vec concat is caller).
//
//   ./scripts/verus_children_range.sh

use vstd::prelude::*;

verus! {

pub open spec fn packed_child_sep() -> u8 {
    0x00
}

pub open spec fn packed_child_end() -> u8 {
    0x01
}

pub open spec fn packed_child_end_as_is() -> u8 {
    0xff
}

pub open spec fn next_byte_in_packed_children_spec(next: u8) -> bool {
    next == packed_child_sep()
}

pub fn next_byte_in_packed_children(next: u8) -> (d: bool)
    ensures
        d == next_byte_in_packed_children_spec(next),
        d == (next == 0x00),
{
    next == 0x00
}

pub open spec fn next_byte_in_packed_children_as_is(next: u8) -> bool {
    next < 0xff
}

proof fn lemma_sep_is_child()
    ensures
        next_byte_in_packed_children_spec(0x00),
{
}

proof fn lemma_as_is_leaks_zero_char()
    ensures
        !next_byte_in_packed_children_spec(0x30),
        next_byte_in_packed_children_as_is(0x30),
        packed_child_end() == 0x01,
        packed_child_end_as_is() == 0xff,
{
}

/// Model domain: byte keys. Half-open `[start, end)` membership.
pub open spec fn key_in_half_open_spec(key: u8, start: u8, end: u8) -> bool {
    start <= key && key < end
}

pub fn key_in_half_open(key: u8, start: u8, end: u8) -> (d: bool)
    ensures
        d == key_in_half_open_spec(key, start, end),
        d ==> key < end,
{
    start <= key && key < end
}

pub open spec fn key_in_half_open_as_is_spec(key: u8, start: u8, _end: u8) -> bool {
    key >= start
}

pub fn key_in_half_open_as_is(key: u8, start: u8, end: u8) -> (d: bool)
    ensures
        d == key_in_half_open_as_is_spec(key, start, end),
{
    key >= start
}

/// F57/F59 teeth: at the exclusive end (and above it) the AS-IS rule still
/// says "member" — the end bound no longer cuts anything.
proof fn lemma_as_is_end_is_a_member(key: u8, start: u8, end: u8)
    requires
        start <= key,
        key == end,
    ensures
        !key_in_half_open_spec(key, start, end),
        key_in_half_open_as_is_spec(key, start, end),
{
}

/// Model domain: `Seq<u8>`. Inclusive start is `packed || sep`.
pub open spec fn packed_children_start_spec(packed: Seq<u8>) -> Seq<u8> {
    packed.push(packed_child_sep())
}

pub fn packed_children_start(packed: &[u8]) -> (s: Vec<u8>)
    ensures
        s@ == packed_children_start_spec(packed@),
        s.len() == packed.len() + 1,
{
    let mut s: Vec<u8> = Vec::new();
    let mut i: usize = 0;
    while i < packed.len()
        invariant
            0 <= i <= packed.len(),
            s.len() == i,
            forall|j: int| 0 <= j < i ==> s[j] == packed[j],
        decreases packed.len() - i,
    {
        s.push(packed[i]);
        i += 1;
    }
    assert(s@ == packed@);
    s.push(0x00);
    assert(s@ == packed_children_start_spec(packed@));
    s
}

pub open spec fn packed_children_start_as_is_spec(packed: Seq<u8>) -> Seq<u8> {
    packed
}

pub fn packed_children_start_as_is(packed: &[u8]) -> (s: Vec<u8>)
    ensures
        s@ == packed_children_start_as_is_spec(packed@),
        s.len() == packed.len(),
{
    let mut s: Vec<u8> = Vec::new();
    let mut i: usize = 0;
    while i < packed.len()
        invariant
            0 <= i <= packed.len(),
            s.len() == i,
            forall|j: int| 0 <= j < i ==> s[j] == packed[j],
        decreases packed.len() - i,
    {
        s.push(packed[i]);
        i += 1;
    }
    assert(s@ == packed@);
    s
}

/// F57 teeth: the AS-IS start IS the parent — the parent key is a member of
/// its own children range; the FIXED start (one separator longer) is not.
proof fn lemma_as_is_start_swallows_parent(packed: Seq<u8>)
    ensures
        packed_children_start_as_is_spec(packed) == packed,
        packed_children_start_spec(packed) != packed,
        packed_children_start_spec(packed).len() == packed.len() + 1,
{
}

/// RFC-0170 close: production packed_children_end.
pub fn packed_children_end() -> (b: u8)
    ensures
        b == packed_child_end(),
        b == 0x01,
{
    0x01u8
}

} // verus!
