//! RFC-0020 P0.2 overnight soak entry.
//!
//! ```text
//! PEDRA_SOAK_TRIALS=1000 cargo run -p pedradb-dst --bin volume_soak -- /path/to/report.json
//! ```

use pedradb_dst::{run_volume_soak, soak_trials_from_env, temp_parent};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let trials = soak_trials_from_env();
    let report = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("volume_report.json"));
    let parent = temp_parent("volume-soak-bin");
    eprintln!("volume_soak trials={trials} report={}", report.display());
    match run_volume_soak(&parent, trials, Some(&report)) {
        Ok(r) => {
            print!("{}", r.to_json());
            eprintln!(
                "volume_soak OK silent_wrong={} trials={}",
                r.silent_wrong, r.trials
            );
            let _ = std::fs::remove_dir_all(&parent);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("volume_soak FAIL: {e}");
            let _ = std::fs::remove_dir_all(&parent);
            ExitCode::FAILURE
        }
    }
}
