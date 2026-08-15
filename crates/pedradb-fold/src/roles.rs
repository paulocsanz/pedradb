//! Named fold roles (RFC-0024 P2.3).

/// How a fold node stores data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldRole {
    /// Full dataset replica (values + keys). Not a Raft voter.
    Storage,
    /// Connection aggregator: cursor only, no user payload.
    Relay,
    /// Persistent cache. `evict_values` keeps keys and drops values.
    Proxy {
        /// When true, apply stores keys only (value miss → L3).
        evict_values: bool,
    },
}

impl Default for FoldRole {
    fn default() -> Self {
        Self::Storage
    }
}
