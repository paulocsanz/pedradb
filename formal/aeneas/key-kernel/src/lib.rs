//! Shim: production `key.rs` uses `crate::error::{CoreError, Result}`.
//! Charon walks this crate; the production files are the source of truth —
//! never edit copies.

#[path = "../../../../crates/pedradb-core/src/error.rs"]
pub mod error;

#[path = "../../../../crates/pedradb-core/src/key.rs"]
pub mod key;

pub use key::*;
