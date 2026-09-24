//! RFC-0270 P1.1: Loom-compatible concurrency primitives shim.
//!
//! Replaces direct usage of `std::sync` and `parking_lot` in production code
//! so that `cargo test --test loom_*` can exhaustively permute the real AST
//! without requiring a duplicate "twin" implementation.

#[cfg(not(loom))]
pub mod atomic {
    pub use std::sync::atomic::*;
}
#[cfg(loom)]
pub mod atomic {
    pub use loom::sync::atomic::*;
}

#[cfg(not(loom))]
pub use parking_lot::{Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
#[cfg(loom)]
pub use loom::sync::{Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

#[cfg(not(loom))]
pub use std::sync::Arc;
#[cfg(loom)]
pub use loom::sync::Arc;

#[cfg(not(loom))]
pub mod mpsc {
    pub use std::sync::mpsc::*;
}
#[cfg(loom)]
pub mod mpsc {
    pub use loom::sync::mpsc::*;
}

#[cfg(not(loom))]
pub mod thread {
    pub use std::thread::*;
}
#[cfg(loom)]
pub mod thread {
    pub use loom::thread::*;
}
