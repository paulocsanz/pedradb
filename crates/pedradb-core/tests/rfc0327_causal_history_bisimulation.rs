//! RFC-0327: Causal History Bisimulation Verification Suite.
//!
//! Mechanically verifies that transaction causal dependency closure is strictly
//! preserved across crash-reboot boundaries ($T_{\text{causal}}$).

#![forbid(unsafe_code)]

use pedradb_core::causal_history_bisimulation_kernel::{
    CausalClosureViolation, CausalHistoryBisimulationJudge,
};

#[test]
fn rfc0327_causal_dag_registration_and_validation() {
    let mut judge = CausalHistoryBisimulationJudge::new();
    assert_eq!(judge.len(), 0);

    // 1. Rejects zero transaction ID
    assert_eq!(
        judge.try_register_tx(0, 10, vec![]),
        Err(CausalClosureViolation::ZeroTransactionIdHazard)
    );

    // 2. Register T1 (root, seq 10)
    judge.try_register_tx(1, 10, vec![]).expect("t1 ok");
    assert_eq!(judge.len(), 1);

    // 3. Register T2 (depends on T1, seq 20)
    judge.try_register_tx(2, 20, vec![1]).expect("t2 ok");

    // 4. Rejects sequence inversion: T3 depends on T2 (seq 20) but claims seq 15
    assert_eq!(
        judge.try_register_tx(3, 15, vec![2]),
        Err(CausalClosureViolation::CausalSequenceInversion {
            predecessor_seq: 20,
            dependent_seq: 15,
        })
    );

    // 5. Rejects duplicate transaction ID
    assert_eq!(
        judge.try_register_tx(1, 30, vec![]),
        Err(CausalClosureViolation::DuplicateTransactionId(1))
    );
}

#[test]
fn rfc0327_causal_closure_verification_trans_crash() {
    let mut judge = CausalHistoryBisimulationJudge::new();

    // Setup DAG: T1 -> T2 -> T3
    judge.try_register_tx(1, 100, vec![]).unwrap();
    judge.try_register_tx(2, 110, vec![1]).unwrap();
    judge.try_register_tx(3, 120, vec![2]).unwrap();

    // Independent T4
    judge.try_register_tx(4, 105, vec![]).unwrap();

    // 1. Clean recovery of all transactions satisfies closure
    assert!(judge.verify_causal_closure(&[1, 2, 3, 4]).is_ok());

    // 2. Partial recovery where prefix [1, 2] is recovered satisfies closure
    assert!(judge.verify_causal_closure(&[1, 2]).is_ok());

    // 3. Independent T4 alone satisfies closure
    assert!(judge.verify_causal_closure(&[4]).is_ok());

    // 4. VIOLATION: T2 is recovered, but predecessor T1 was lost during crash!
    let violation = judge.verify_causal_closure(&[2, 4]);
    assert_eq!(
        violation,
        Err(CausalClosureViolation::MissingCausalPredecessor {
            tx_id: 2,
            missing_predecessor_id: 1,
        })
    );

    // 5. VIOLATION: T3 is recovered, but predecessor T2 is missing
    let violation_t3 = judge.verify_causal_closure(&[1, 3]);
    assert_eq!(
        violation_t3,
        Err(CausalClosureViolation::MissingCausalPredecessor {
            tx_id: 3,
            missing_predecessor_id: 2,
        })
    );
}
