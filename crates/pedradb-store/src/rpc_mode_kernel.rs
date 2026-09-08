//! DST Queued-RPC pin (RFC-0067 P0).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_rpc_mode.sh
//!
//! `RpcMode::Direct` is a same-`PeerMsg` sync pump that skips Net
//! drop/reorder/delay. Once World/DST pins Queued, Direct is not admitted.

#![forbid(unsafe_code)]

macro_rules! allow_direct_rpc_body {
    ($dst_pin:expr, $want_direct:expr) => {
        if $want_direct && $dst_pin {
            false
        } else {
            true
        }
    };
}

macro_rules! allow_direct_rpc_as_is_body {
    ($dst_pin:expr, $want_direct:expr) => {{
        let _ = ($dst_pin, $want_direct);
        true
    }};
}

/// Admit a request to run Direct RPC.
///
/// Direct is allowed only when the caller wants Direct **and** DST has not
/// pinned Queued. Queued is always admitted.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn allow_direct_rpc(dst_pin: bool, want_direct: bool) -> bool {
    allow_direct_rpc_body!(dst_pin, want_direct)
}

/// AS-IS: pin does not stick (the 0067 hole — `set_rpc_mode(Direct)` after
/// `World::run` skips Net).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn allow_direct_rpc_as_is(_dst_pin: bool, _want_direct: bool) -> bool {
    allow_direct_rpc_as_is_body!(_dst_pin, _want_direct)
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn allow_direct_rpc_spec(dst_pin: bool, want_direct: bool) -> bool {
    !(want_direct && dst_pin)
}

/// AS-IS: pin does not stick — Direct is always admitted.
pub open spec fn allow_direct_rpc_as_is_spec(_dst_pin: bool, _want_direct: bool) -> bool {
    true
}

pub fn allow_direct_rpc(dst_pin: bool, want_direct: bool) -> (ok: bool)
    ensures
        ok == allow_direct_rpc_spec(dst_pin, want_direct),
{
    allow_direct_rpc_body!(dst_pin, want_direct)
}

pub fn allow_direct_rpc_as_is(_dst_pin: bool, _want_direct: bool) -> (ok: bool)
    ensures
        ok == allow_direct_rpc_as_is_spec(_dst_pin, _want_direct),
{
    allow_direct_rpc_as_is_body!(_dst_pin, _want_direct)
}

proof fn lemma_pin_refuses_direct()
    ensures
        !allow_direct_rpc_spec(true, true),
        allow_direct_rpc_as_is_spec(true, true),
{
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_refuses_direct_as_is_would_allow() {
        assert!(!allow_direct_rpc(true, true));
        assert!(allow_direct_rpc_as_is(true, true));
        assert!(allow_direct_rpc(false, true));
        assert!(allow_direct_rpc(true, false));
        assert!(allow_direct_rpc(false, false));
    }
}
