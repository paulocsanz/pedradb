//! Shim: production `key.rs` uses `crate::error::{CoreError, Result}`.
//! `error.rs` carries thiserror (`Error::source` unsized-cast — refused).
//! The decision fns are production via `#[path]`; CoreError here is the
//! `Internal` constructor `unpack`/`decode` use, without thiserror.

pub mod error {
    pub enum CoreError {
        /// Internal invariant (`key.rs` is the only constructor Charon sees).
        Internal(String),
    }
    pub type Result<T> = std::result::Result<T, CoreError>;
}

pub mod mutation_switch_kernel {
    pub const MUTANT_INVERT_COMPARATOR: u32 = 1004;
}

#[macro_export]
macro_rules! mutate_switch {
    ($id:expr, $baseline:expr, $mutant:expr) => {
        $baseline
    };
}

#[path = "../../../../crates/pedradb-core/src/key_kernel.rs"]
pub mod key;

pub use key::*;
