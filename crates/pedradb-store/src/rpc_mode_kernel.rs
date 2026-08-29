//! DST Queued-RPC pin (RFC-0067 P0).
//!
//! `RpcMode::Direct` is a same-`PeerMsg` sync pump that skips Net
//! drop/reorder/delay. Once World/DST pins Queued, Direct is not admitted.

#![forbid(unsafe_code)]

/// Admit a request to run Direct RPC.
///
/// Direct is allowed only when the caller wants Direct **and** DST has not
/// pinned Queued. Queued is always admitted.
#[must_use]
pub fn allow_direct_rpc(dst_pin: bool, want_direct: bool) -> bool {
    if want_direct && dst_pin {
        false
    } else {
        true
    }
}

/// AS-IS: pin does not stick (the 0067 hole — `set_rpc_mode(Direct)` after
/// `World::run` skips Net).
#[must_use]
pub fn allow_direct_rpc_as_is(_dst_pin: bool, _want_direct: bool) -> bool {
    true
}

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
