//! RFC-0020 P0.1 silent_wrong CI gate (in-tree matrix).
//!
//! Honors `PEDRA_EXPLORE_OFFSET` (seed sweep start). Optional report path arg.
//!
//! ```text
//! PEDRA_EXPLORE_OFFSET=17 cargo run -p pedradb-dst --bin silent_wrong_gate -- report.json
//! ```

use pedradb_dst::{run_silent_wrong_gate, temp_parent};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let report = env::args().nth(1).map(PathBuf::from);
    let parent = temp_parent("silent-wrong-gate");
    match run_silent_wrong_gate(&parent) {
        Ok(r) => {
            print!("{}", r.to_json());
            if let Some(path) = report {
                if let Err(e) = r.write_to(&path) {
                    eprintln!("ci_silent_wrong_gate WARN write report: {e}");
                }
            }
            eprintln!(
                "ci_silent_wrong_gate OK silent_wrong={} trials={} explore_offset={} seed_base={}",
                r.silent_wrong, r.trials, r.explore_offset, r.seed_base
            );
            let _ = std::fs::remove_dir_all(&parent);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("ci_silent_wrong_gate FAIL: {e}");
            let _ = std::fs::remove_dir_all(&parent);
            ExitCode::FAILURE
        }
    }
}
