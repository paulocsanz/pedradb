//! Caixote-shaped prefix + intent/observed delta (RFC-0024 P1.3–P1.4).
//!
//! Federation primary path is **delta by seq**. A full dump is a timed backstop.

use crate::follow::PrefixSet;
use crate::{FoldCursor, FoldStore, FoldUpdate, Result};

/// Host fold prefixes: `/host/{id}/` plus owned `/vm/{id}` and `/assign/{id}`.
#[must_use]
pub fn caixote_host_filter(host_id: &str, vm_ids: &[&str]) -> PrefixSet {
    let mut s = PrefixSet::new();
    s.push(format!("/host/{host_id}/").into_bytes());
    for vm in vm_ids {
        // Isolated ids (F83): `/vm/vm-a` must not `starts_with`-match `/vm/vm-ab`.
        s.push_isolated(format!("/vm/{vm}").into_bytes());
        s.push_isolated(format!("/assign/{vm}").into_bytes());
    }
    s
}

/// Seq-stamped delta (intent or observed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentObservedDelta {
    /// Source sequence (exclusive lower bound was previous ack).
    pub seq: u64,
    /// Puts (key, value).
    pub puts: Vec<(Vec<u8>, Vec<u8>)>,
    /// Deleted keys.
    pub deletes: Vec<Vec<u8>>,
}

impl IntentObservedDelta {
    /// Empty delta at `seq`.
    #[must_use]
    pub fn empty(seq: u64) -> Self {
        Self {
            seq,
            puts: Vec::new(),
            deletes: Vec::new(),
        }
    }

    /// Convert to fold updates.
    #[must_use]
    pub fn to_updates(&self) -> Vec<FoldUpdate> {
        let mut out = Vec::new();
        for (k, v) in &self.puts {
            out.push(FoldUpdate::Put {
                key: k.clone(),
                value: v.clone(),
                seq: self.seq,
            });
        }
        for k in &self.deletes {
            out.push(FoldUpdate::Delete {
                key: k.clone(),
                seq: self.seq,
            });
        }
        out
    }
}

/// Local ack + backstop clock for federation sync.
#[derive(Debug, Clone)]
pub struct SeqSyncState {
    /// Last applied delta seq.
    pub acked: u64,
    /// Last time a full dump was taken (logical ms).
    pub last_full_dump_at: u64,
    /// Full-dump backstop interval (logical ms). Default 5000.
    pub backstop_ms: u64,
    /// Delta keys applied.
    pub sync_delta_keys: u64,
    /// Full dumps used.
    pub sync_full_dump: u64,
}

impl Default for SeqSyncState {
    fn default() -> Self {
        Self {
            acked: 0,
            last_full_dump_at: 0,
            backstop_ms: 5000,
            sync_delta_keys: 0,
            sync_full_dump: 0,
        }
    }
}

impl SeqSyncState {
    /// Primary path: apply a delta with `seq > acked`.
    ///
    /// # Errors
    /// Store apply.
    pub fn apply_delta(
        &mut self,
        store: &mut impl FoldStore,
        delta: &IntentObservedDelta,
    ) -> Result<FoldCursor> {
        if delta.seq <= self.acked {
            return Ok(FoldCursor(self.acked));
        }
        let updates = delta.to_updates();
        store.apply(&updates, FoldCursor(delta.seq))?;
        self.acked = delta.seq;
        self.sync_delta_keys = self.sync_delta_keys.saturating_add(updates.len() as u64);
        Ok(FoldCursor(self.acked))
    }

    /// Whether the 5 s (or configured) full dump should run as **backstop**.
    #[must_use]
    pub fn should_full_dump(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_full_dump_at) >= self.backstop_ms
    }

    /// Record that a full dump was applied (backstop only).
    pub fn note_full_dump(&mut self, now_ms: u64, seq: u64) {
        self.last_full_dump_at = now_ms;
        if seq > self.acked {
            self.acked = seq;
        }
        self.sync_full_dump = self.sync_full_dump.saturating_add(1);
    }
}

/// LocalApplied route/desired get — never `get_strong`.
///
/// # Errors
/// Store get.
pub fn fold_get_local(store: &impl FoldStore, key: &[u8]) -> Result<Option<Vec<u8>>> {
    store.get(key)
}
