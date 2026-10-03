//! RFC-0328: Integration Test for Metamorphic Kernel Grammar Fuzzer.
//!
//! Submits PedraDB core mathematical and storage kernels to adversarial grammar-based
//! mutations under randomized pseudo-random seeds, verifying invariant conservation.

#![forbid(unsafe_code)]

use pedradb_dst::kernel_grammar_fuzzer::run_kernel_grammar_fuzz;

#[test]
fn test_rfc0328_kernel_grammar_fuzz_seeds() {
    let seeds = [
        0x1111_2222_3333_4444,
        0x5555_6666_7777_8888,
        0x9999_AAAA_BBBB_CCCC,
        0xDDDD_EEEE_FFFF_0000,
        0xCAFE_BABE_DEAD_BEEF,
    ];

    for &seed in &seeds {
        let report = run_kernel_grammar_fuzz(seed, 2000);
        assert_eq!(report.iterations, 2000);
        assert!(
            report.rejected_invalid_inputs > 500,
            "Fuzzer must intercept and safely reject invalid boundary inputs (seed {seed:#x}, got {})",
            report.rejected_invalid_inputs
        );
        assert!(
            report.axioms_verified > 1000,
            "Fuzzer must formally verify algebraic axioms across valid space (seed {seed:#x}, got {})",
            report.axioms_verified
        );
    }
}
