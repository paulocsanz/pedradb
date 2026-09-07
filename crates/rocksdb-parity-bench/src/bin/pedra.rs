//! Product CLI for the official scale ladder (RFC-0178 P0.10) and
//! RFC-0184 surgical diagnose.
//!
//! ```text
//! pedra scale [--entries N] [--cache BYTES] [--backends NAME] [dir]
//! pedra diagnose write --pedra-ns N --rocks-ns N [--wal-ns N] [...]
//! pedra diagnose get --keys N --ram BYTES --measured-ns N
//! ```

#![forbid(unsafe_code)]

use pedradb_core::bench_gap_kernel::{classify_get, diagnose_write, WriteGapInput, WritePhases};
use pedradb_core::scale_kernel::{scale_forecast, scale_forecast_as_is};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("scale") => {
            let cli = match rocksdb_parity_bench::scale::parse_pedra_scale(&args) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(2);
                }
            };
            rocksdb_parity_bench::scale::apply_pedra_scale(&cli);
            rocksdb_parity_bench::scale::run(&cli.dir);
        }
        Some("diagnose") => {
            if diagnose_cmd(&args[1..]).is_err() {
                std::process::exit(2);
            }
        }
        _ => {
            eprintln!("usage: pedra scale [--entries N] [--cache BYTES] [--backends NAME] [dir]");
            eprintln!(
                "       pedra diagnose write --pedra-ns N --rocks-ns N [--wal-ns N] [--mem-ns N] [--flush-ns N] [--lock-ns N] [--prepare-ns N] [--publish-ns N] [--clients N] [--avg-group X]"
            );
            eprintln!("       pedra diagnose get --keys N --ram BYTES --measured-ns N");
            std::process::exit(2);
        }
    }
}

fn diagnose_cmd(args: &[String]) -> Result<(), ()> {
    match args.first().map(String::as_str) {
        Some("write") => diagnose_write_cmd(&args[1..]),
        Some("get") => diagnose_get_cmd(&args[1..]),
        _ => {
            eprintln!("usage: pedra diagnose write|get …");
            Err(())
        }
    }
}

fn flag_u64(args: &[String], name: &str) -> Option<u64> {
    args.windows(2)
        .find_map(|w| (w[0] == name).then(|| w[1].parse().ok()).flatten())
}

fn diagnose_write_cmd(args: &[String]) -> Result<(), ()> {
    let Some(pedra_ns) = flag_u64(args, "--pedra-ns") else {
        eprintln!("pedra diagnose write: --pedra-ns is required");
        return Err(());
    };
    let rocks_ns = flag_u64(args, "--rocks-ns").unwrap_or(0);
    let clients = flag_u64(args, "--clients").unwrap_or(1);
    let avg_group_bps = flag_u64(args, "--avg-group-bps").unwrap_or_else(|| {
        args.windows(2)
            .find(|w| w[0] == "--avg-group")
            .and_then(|w| w[1].parse::<f64>().ok())
            .map(|g| (g * 10_000.0) as u64)
            .unwrap_or(0)
    });
    let inp = WriteGapInput {
        pedra_ns,
        rocks_ns,
        clients,
        avg_group_bps,
        phases: WritePhases {
            prepare_ns: flag_u64(args, "--prepare-ns").unwrap_or(0),
            wal_ns: flag_u64(args, "--wal-ns").unwrap_or(0),
            mem_ns: flag_u64(args, "--mem-ns").unwrap_or(0),
            publish_ns: flag_u64(args, "--publish-ns").unwrap_or(0),
            flush_check_ns: flag_u64(args, "--flush-ns").unwrap_or(0),
            lock_wait_ns: flag_u64(args, "--lock-ns").unwrap_or(0),
        },
    };
    let d = diagnose_write(inp);
    println!("pedra diagnose write");
    println!(
        "gap_ns={} timed_ns={} unattributed_ns={}",
        d.gap_ns, d.timed_ns, d.unattributed_ns
    );
    println!("{}", d.line());
    Ok(())
}

fn diagnose_get_cmd(args: &[String]) -> Result<(), ()> {
    let Some(keys) = flag_u64(args, "--keys") else {
        eprintln!("pedra diagnose get: --keys is required");
        return Err(());
    };
    let Some(ram) = flag_u64(args, "--ram") else {
        eprintln!("pedra diagnose get: --ram is required (bytes)");
        return Err(());
    };
    let Some(measured_ns) = flag_u64(args, "--measured-ns") else {
        eprintln!("pedra diagnose get: --measured-ns is required");
        return Err(());
    };
    let f = scale_forecast(keys, ram);
    let as_is = scale_forecast_as_is(keys, ram);
    let class = classify_get(
        measured_ns,
        f.best_ns,
        f.happy_ns,
        f.worst_ns,
        as_is.best_ns,
    );
    let mode = if f.hot { "hot" } else { "bounded-cache" };
    println!("pedra diagnose get keys={keys} ram={ram} mode={mode}");
    println!(
        "P_best={} P_worst={} n_files={}",
        f.p_best, f.p_worst, f.n_files
    );
    println!(
        "T_ns best={} happy={} worst={} as_is={}",
        f.best_ns, f.happy_ns, f.worst_ns, as_is.best_ns
    );
    println!("measured_ns={measured_ns} class={}", class.token());
    Ok(())
}
