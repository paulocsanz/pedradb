//! Shim: production `product_crown_kernel.rs` uses d1/env/wal kernels
//! and `pedradb_spec::properties_kernel::d1_holds`. The spec kernel is
//! path-included as `pedradb_spec::properties_kernel` via a local crate
//! alias module.

#[path = "../../../../crates/pedradb-core/src/group_commit_kernel.rs"]
pub mod group_commit_kernel;

#[path = "../../../../crates/pedradb-core/src/env_crash_kernel.rs"]
pub mod env_crash_kernel;

#[path = "../../../../crates/pedradb-core/src/d1_modelo_kernel.rs"]
pub mod d1_modelo_kernel;

pub mod wal;

#[path = "../../../../crates/pedradb-spec/src/properties_kernel.rs"]
pub mod properties_kernel;

/// Stand-in for the `pedradb_spec` crate so the production file's
/// `use pedradb_spec::properties_kernel::d1_holds` resolves.
pub mod pedradb_spec {
    pub use super::properties_kernel;
}

#[path = "../../../../crates/pedradb-core/src/product_crown_kernel.rs"]
pub mod product_crown_kernel;

pub use product_crown_kernel::*;
