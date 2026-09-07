//! Product CLI for the official scale ladder (RFC-0178 P0.10) and
//! RFC-0184 surgical diagnose.
//!
//! ```text
//! pedra scale [--entries N] [--cache BYTES] [--backends NAME] [dir]
//! pedra diagnose write --pedra-ns N --rocks-ns N [--read-pct N] [...]
//! pedra diagnose get --keys N --ram BYTES --measured-ns N
//! pedra diagnose probes --per-get N --p-best N
//! pedra diagnose balance --cut TOKEN --cell lever[:diag|:named] [...]
//! ```

#![forbid(unsafe_code)]

use pedradb_core::bench_gap_kernel::{
    balance_admits, classify_get, classify_probes, diagnose_write, BalanceCell, WriteGapInput,
    WriteLever, WritePhases, BALANCE_SHAPES,
};
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
                "       pedra diagnose write --pedra-ns N --rocks-ns N [--read-pct N] [--wal-ns N] [--mem-ns N] [--flush-ns N] [--lock-ns N] [--prepare-ns N] [--publish-ns N] [--clients N] [--avg-group X]"
            );
            eprintln!("       pedra diagnose get --keys N --ram BYTES --measured-ns N");
            eprintln!("       pedra diagnose probes --per-get N --p-best N");
            eprintln!("       pedra diagnose balance --cut TOKEN --cell TOKEN[:diag|:named] [...]");
            eprintln!("       balance shapes: {}", BALANCE_SHAPES.join(","));
            std::process::exit(2);
        }
    }
}

fn diagnose_cmd(args: &[String]) -> Result<(), ()> {
    match args.first().map(String::as_str) {
        Some("write") => diagnose_write_cmd(&args[1..]),
        Some("get") => diagnose_get_cmd(&args[1..]),
        Some("probes") => diagnose_probes_cmd(&args[1..]),
        Some("balance") => diagnose_balance_cmd(&args[1..]),
        _ => {
            eprintln!("usage: pedra diagnose write|get|probes|balance …");
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
        read_pct: flag_u64(args, "--read-pct").unwrap_or(0),
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

fn diagnose_probes_cmd(args: &[String]) -> Result<(), ()> {
    let Some(per_get) = flag_u64(args, "--per-get") else {
        eprintln!("pedra diagnose probes: --per-get is required");
        return Err(());
    };
    let Some(p_best) = flag_u64(args, "--p-best") else {
        eprintln!("pedra diagnose probes: --p-best is required");
        return Err(());
    };
    let class = classify_probes(per_get, p_best);
    println!(
        "pedra diagnose probes per_get={per_get} p_best={p_best} class={}",
        class.token()
    );
    Ok(())
}

fn diagnose_balance_cmd(args: &[String]) -> Result<(), ()> {
    let Some(cut_tok) = args
        .windows(2)
        .find(|w| w[0] == "--cut")
        .map(|w| w[1].as_str())
    else {
        eprintln!("pedra diagnose balance: --cut TOKEN is required");
        return Err(());
    };
    let Some(cut) = WriteLever::from_token(cut_tok) else {
        eprintln!("unknown cut token {cut_tok}");
        return Err(());
    };
    let mut cells = Vec::new();
    let mut i = 0;
    while i + 1 < args.len() {
        if args[i] == "--cell" {
            let spec = &args[i + 1];
            let mut parts = spec.split(':');
            let tok = parts.next().unwrap_or("");
            let Some(lever) = WriteLever::from_token(tok) else {
                eprintln!("unknown cell lever {spec}");
                return Err(());
            };
            let mut linux_named_loss = false;
            let mut diag_only = false;
            for tag in parts {
                match tag {
                    "named" => linux_named_loss = true,
                    "diag" => diag_only = true,
                    _ => {
                        eprintln!("unknown cell tag {tag} (want named|diag)");
                        return Err(());
                    }
                }
            }
            cells.push(BalanceCell {
                linux_named_loss,
                diag_only,
                lever,
            });
            i += 2;
            continue;
        }
        i += 1;
    }
    if cells.is_empty() {
        eprintln!("pedra diagnose balance: at least one --cell TOKEN[:diag|:named]");
        return Err(());
    }
    let admits = balance_admits(cut, &cells);
    println!(
        "pedra diagnose balance cut={} cells={} admits={}",
        cut.token(),
        cells.len(),
        u8::from(admits)
    );
    println!("shapes={}", BALANCE_SHAPES.join(","));
    Ok(())
}
