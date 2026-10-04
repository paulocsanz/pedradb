//! Property specs and verification contracts of the Pedra product (RFC-0166 / RFC-0332 / RFC-0333 / RFC-0334 / RFC-0336).
//!
//! Exposes:
//! - `properties_kernel`: Product properties D1, R1, T1, C1 (RFC-0166)
//! - `syscall_glue_kernel`: Contracted Syscall Glue for POSIX and io_uring (RFC-0332, AGENTS.md §4)
//! - `composition_m2_kernel`: End-to-End M2 Chained Invariant Composition Engine (RFC-0332, AGENTS.md §3)
//! - `fault_grid_crash_kernel`: Fault-Grid 42-Cell Matrix & Concurrent Crash Consistency (RFC-0333)
//! - `manifest_crash_kernel`: Manifest Two-Phase Crash-Consistency & Ghost SST Quarantine (RFC-0334)
//! - `unsafe_provenance_kernel`: Memory Alignment, Bounds, and Pointer Provenance for Unsafe Islands (RFC-0336)

pub mod properties_kernel;
pub mod syscall_glue_kernel;
pub mod composition_m2_kernel;
pub mod fault_grid_crash_kernel;
pub mod manifest_crash_kernel;
pub mod pacing_poison_kernel;
pub mod unsafe_provenance_kernel;

