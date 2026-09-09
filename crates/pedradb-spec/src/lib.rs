//! Property specs of the Pedra product (RFC-0166): the four end-to-end
//! properties as pure functions over abstract models, each with an AS-IS
//! weak version and an anti-vacuity tooth (a proven witness where the
//! weak version holds and the property does not).
//!
//! This crate is deliberately dependency-free: the specs are the top of
//! the refinement chain (property → invariant → kernel atom), compiled
//! like any other Rust and single-artifact proved — the Charon+Aeneas
//! extract of `properties_kernel.rs` (`scripts/aeneas_properties.sh`).

pub mod properties_kernel;
