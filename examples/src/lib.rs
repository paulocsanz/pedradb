//! PedraDB example gallery.
//!
//! This crate exists so the programs under [`examples/`](https://github.com/paulocsanz/pedradb/tree/main/examples)
//! can share workspace dependencies. Each example is a standalone binary:
//!
//! ```sh
//! cargo run -p pedradb-examples --example hello
//! cargo test  -p pedradb-examples --examples
//! ```
//!
//! Start at `README.md` in this directory for the ladder (hello → bank → layers).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Unique scratch directory for one example run (safe under parallel tests).
#[must_use]
pub fn scratch(name: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("pedradb-ex-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Best-effort recursive delete (examples always try to leave `/tmp` clean).
pub fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}
