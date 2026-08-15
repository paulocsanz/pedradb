// Verus proof of stream cursor rules (RFC-0002 P20 / F54).
// Twin of `src/cursor_kernel.rs`. Not linked into production.
//
//   ./scripts/verus_stream_cursor.sh

use vstd::prelude::*;

verus! {

pub open spec fn sat_add1(x: u64) -> u64 {
    if x == u64::MAX {
        x
    } else {
        (x + 1) as u64
    }
}

pub fn next_seq(last_acked: u64) -> (n: u64)
    ensures
        n == sat_add1(last_acked),
        last_acked < u64::MAX ==> n == last_acked + 1,
{
    if last_acked == u64::MAX {
        last_acked
    } else {
        last_acked + 1
    }
}

pub fn ack_in_order(last_acked: u64, seq: u64) -> (d: bool)
    ensures
        d == (seq == sat_add1(last_acked) && seq > last_acked),
{
    let n = if last_acked == u64::MAX {
        last_acked
    } else {
        last_acked + 1
    };
    seq == n && seq > last_acked
}

pub open spec fn ack_in_order_as_is(last_acked: u64, seq: u64) -> bool {
    seq > last_acked
}

proof fn lemma_as_is_skips(last: u64)
    requires
        last + 2 <= u64::MAX,
    ensures
        ack_in_order_as_is(last, (last + 2) as u64),
        !((last + 2) as u64 == sat_add1(last) && (last + 2) as u64 > last),
{
}

pub fn peek_pins_cursor() -> (d: bool)
    ensures
        !d,
{
    false
}

pub open spec fn peek_pins_cursor_as_is() -> bool {
    true
}

proof fn lemma_as_is_pins()
    ensures
        peek_pins_cursor_as_is(),
{
}

} // verus!
