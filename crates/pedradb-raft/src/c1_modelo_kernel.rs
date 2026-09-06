//! Named C1 corollary (RFC-0166 P2.3): **a served index is committed by
//! majority of every active config** (joint: both; single: C-old).
//!
//! Composes the production consensus kernels already on the catalog:
//!
//! - [`crate::joint_election_ok`]: majority of C-old **and** C-new when a
//!   joint config is in flight (Raft §6).
//! - [`crate::may_commit_at`]: commit index `N` only if that majority
//!   holds **and** `log[N].term == current_term`.
//! - [`crate::propose_ack_ok`]: client `Ok(index)` only if `commit_index`
//!   already covers `index`.
//!
//! - [`c1_advance_commit`]: the honest commit step — the index moves only
//!   when the joint quorum and the term match.
//! - [`c1_modelo`]: after that step, a served index is covered by commit
//!   (T1-style: the corollary of the atoms, not a campaign gate).
//! - AS-IS: old-majority-only election ([`crate::joint_election_ok_as_is`])
//!   plus ack-without-commit ([`crate::propose_ack_ok_as_is`]) — a joint
//!   add elects/commits/serves on C-old alone.
//!
//! Verus twin: `crates/pedradb-raft/verus/c1_modelo.rs`
//! (`scripts/verus_c1_modelo.sh`).

#![forbid(unsafe_code)]

use crate::{
    joint_election_ok, joint_election_ok_as_is, may_commit_at, may_commit_at_as_is, propose_ack_ok,
    propose_ack_ok_as_is,
};

/// Abstract cluster cut the C1 corollary talks about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct C1State {
    /// C-old voter count.
    pub old_n: u64,
    /// C-old votes / matching replicas.
    pub old_yes: u64,
    /// Joint config in flight.
    pub joint: bool,
    /// C-new voter count (ignored when `!joint`).
    pub new_n: u64,
    /// C-new votes / matching replicas.
    pub new_yes: u64,
    /// Leader current term.
    pub current_term: u64,
    /// Term of the proposed index.
    pub index_term: u64,
    /// Durable commit index.
    pub commit_index: u64,
    /// Proposed log index the client wants acked / served.
    pub proposed: u64,
    /// Whether a replica served `proposed`.
    pub served: bool,
}

fn new_cfg(s: &C1State) -> Option<(u64, u64)> {
    if s.joint {
        Some((s.new_yes, s.new_n))
    } else {
        None
    }
}

/// Joint (or single) quorum of every active config.
#[must_use]
pub fn c1_quorum(s: &C1State) -> bool {
    joint_election_ok(s.old_yes, s.old_n, new_cfg(s))
}

/// Honest commit step: the proposed index becomes `commit_index` only
/// when the joint quorum holds and the entry is from this term.
#[must_use]
pub fn c1_advance_commit(s: C1State) -> C1State {
    if may_commit_at(s.index_term, s.current_term, c1_quorum(&s)) {
        C1State {
            commit_index: s.proposed.max(s.commit_index),
            ..s
        }
    } else {
        s
    }
}

/// AS-IS commit: old majority alone (ignore C-new) and no term check.
#[must_use]
pub fn c1_advance_commit_as_is(s: C1State) -> C1State {
    let majority = joint_election_ok_as_is(s.old_yes, s.old_n, new_cfg(&s));
    if may_commit_at_as_is(s.index_term, s.current_term, majority) {
        C1State {
            commit_index: s.proposed.max(s.commit_index),
            ..s
        }
    } else {
        s
    }
}

/// Named corollary: a served index is covered by the honest commit.
#[must_use]
pub fn c1_modelo(s: C1State) -> bool {
    let t = c1_advance_commit(s);
    !t.served || propose_ack_ok(t.proposed, t.commit_index)
}

/// AS-IS corollary: ack without waiting for commit, gated only on C-old.
#[must_use]
pub fn c1_modelo_as_is(s: C1State) -> bool {
    let t = c1_advance_commit_as_is(s);
    !t.served || propose_ack_ok_as_is(t.proposed, t.commit_index)
}

/// Joint-add shape: C-old 2/3, C-new 1/4, same-term proposed index,
/// a replica wants to serve. Honest path refuses; AS-IS C-old elects.
#[must_use]
pub fn joint_add_shape() -> C1State {
    C1State {
        old_n: 3,
        old_yes: 2,
        joint: true,
        new_n: 4,
        new_yes: 1,
        current_term: 7,
        index_term: 7,
        commit_index: 0,
        proposed: 5,
        served: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honest_joint_add_does_not_commit_or_ack() {
        let s = joint_add_shape();
        assert!(!c1_quorum(&s), "C-new 1/4 is not a majority of 4");
        let t = c1_advance_commit(s);
        assert_eq!(t.commit_index, 0);
        assert!(!propose_ack_ok(t.proposed, t.commit_index));
        assert!(!c1_modelo(s));
    }

    #[test]
    fn as_is_old_majority_commits_during_joint() {
        let s = joint_add_shape();
        assert!(
            joint_election_ok_as_is(s.old_yes, s.old_n, new_cfg(&s)),
            "AS-IS dente: C-old 2/3 elects during joint add"
        );
        let t = c1_advance_commit_as_is(s);
        assert_eq!(t.commit_index, 5);
        assert!(c1_modelo_as_is(s), "AS-IS acks without the new majority");
        assert!(!c1_modelo(s));
    }

    #[test]
    fn single_config_majority_commits() {
        let s = C1State {
            old_n: 3,
            old_yes: 2,
            joint: false,
            new_n: 0,
            new_yes: 0,
            current_term: 1,
            index_term: 1,
            commit_index: 0,
            proposed: 3,
            served: true,
        };
        assert!(c1_quorum(&s));
        let t = c1_advance_commit(s);
        assert_eq!(t.commit_index, 3);
        assert!(c1_modelo(s));
    }

    #[test]
    fn c1_modelo_on_live_joint_is_not_ok() {
        let s = joint_add_shape();
        assert!(!c1_modelo(s));
        assert!(c1_modelo_as_is(s));
        let _ = c1_advance_commit(s);
        let _ = c1_advance_commit_as_is(s);

        let parent = std::env::temp_dir().join(format!("pedra-c1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        let mut cluster = crate::RaftCluster::open(&parent, 3).unwrap();
        let leader = cluster.elect_leader(50).unwrap();
        assert!(cluster.node(leader).unwrap().is_leader());
        let idx = cluster
            .propose_puts([(b"c1k".to_vec(), b"c1v".to_vec())])
            .unwrap();
        assert!(
            propose_ack_ok(idx, idx),
            "honest 3-node propose returns a committed index"
        );
        for id in cluster.ids() {
            assert_eq!(
                cluster.node(*id).unwrap().get(b"c1k").as_deref(),
                Some(b"c1v".as_ref()),
                "node {id} served only after majority commit"
            );
        }
        let _ = std::fs::remove_dir_all(&parent);
    }
}
