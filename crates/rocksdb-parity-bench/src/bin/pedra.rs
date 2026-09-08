//! Product CLI for the official scale ladder (RFC-0178 P0.10) and
//! RFC-0184 surgical diagnose.
//!
//! ```text
//! pedra scale [--entries N] [--cache BYTES] [--backends NAME] [dir]
//! pedra diagnose write [--clients N] [--read-pct N] [--sync 0|1] [--avg-group X]
//! pedra diagnose write --pedra-ns N --rocks-ns N [--read-pct N] [...]
//! pedra diagnose get --keys N --ram BYTES [--measured-ns N] [--cache happy|capacity|cold]
//! pedra diagnose probes --per-get N --p-best N
//! pedra diagnose balance --cut TOKEN --cell lever[:diag|:named] [...]
//! ```

#![forbid(unsafe_code)]

use pedradb_core::bench_gap_kernel::{
    balance_admits, classify_get, classify_probes, diagnose_write, scale_bottleneck, BalanceCell,
    WriteGapInput, WriteLever, WritePhases, BALANCE_SHAPES,
};
use pedradb_core::get_cost_kernel::{predict_get_composed, CacheCase, INTEL_SERVER_4GHZ};
use pedradb_core::scale_kernel::{
    predict_write_mix, write_forecast_cut, write_forecast_next, WritePredictIn,
    SCALE_BYTES_PER_ENTRY,
};

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
                "       pedra diagnose write [--clients N] [--read-pct N] [--sync 0|1] [--avg-group X]"
            );
            eprintln!(
                "       pedra diagnose write --pedra-ns N --rocks-ns N [--read-pct N] [--wal-ns N] [--mem-ns N] [--flush-ns N] [--lock-ns N] [--prepare-ns N] [--publish-ns N] [--clients N] [--avg-group X]"
            );
            eprintln!(
                "       pedra diagnose get --keys N --ram BYTES [--measured-ns N] [--bytes-per-key B] [--cache happy|capacity|cold]"
            );
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

fn flag_str<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|w| w[0] == name)
        .map(|w| w[1].as_str())
}

fn avg_group_bps_from_args(args: &[String]) -> u64 {
    flag_u64(args, "--avg-group-bps").unwrap_or_else(|| {
        args.windows(2)
            .find(|w| w[0] == "--avg-group")
            .and_then(|w| w[1].parse::<f64>().ok())
            .map(|g| (g * 10_000.0) as u64)
            .unwrap_or(0)
    })
}

fn diagnose_write_cmd(args: &[String]) -> Result<(), ()> {
    if flag_u64(args, "--pedra-ns").is_none() {
        let clients = flag_u64(args, "--clients").unwrap_or(1);
        let read_pct = flag_u64(args, "--read-pct").unwrap_or(0);
        let sync = flag_u64(args, "--sync").unwrap_or(0) != 0;
        let avg_group_bps = avg_group_bps_from_args(args);
        let w = predict_write_mix(WritePredictIn {
            clients,
            read_pct,
            sync,
            avg_group_bps,
        });
        println!(
            "pedra diagnose write predict=1 clients={clients} read_pct={read_pct} sync={}",
            u8::from(sync)
        );
        println!(
            "expected_group={} distinguishable={} cut={} next={}",
            w.expected_group,
            u8::from(w.distinguishable),
            write_forecast_cut(w),
            write_forecast_next(w)
        );
        println!(
            "T_ns best={} as_is={} lock_hold={}",
            w.best_ns, w.as_is_ns, w.lock_hold_ns
        );
        println!(
            r#"{{"cut":"{}","next":"{}","distinguishable":{},"clients":{},"expected_group":{},"best":{},"as_is":{},"lock_hold_ns":{},"read_pct":{},"sync":{}}}"#,
            write_forecast_cut(w),
            write_forecast_next(w),
            u8::from(w.distinguishable),
            w.clients,
            w.expected_group,
            w.best_ns,
            w.as_is_ns,
            w.lock_hold_ns,
            w.read_pct,
            u8::from(w.sync)
        );
        return Ok(());
    }
    let Some(pedra_ns) = flag_u64(args, "--pedra-ns") else {
        eprintln!("pedra diagnose write: --pedra-ns is required");
        return Err(());
    };
    let rocks_ns = flag_u64(args, "--rocks-ns").unwrap_or(0);
    let clients = flag_u64(args, "--clients").unwrap_or(1);
    let avg_group_bps = avg_group_bps_from_args(args);
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
    println!("{}", d.json_object());
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
    let bpe = flag_u64(args, "--bytes-per-key").unwrap_or(SCALE_BYTES_PER_ENTRY);
    let cache = match flag_str(args, "--cache") {
        None => CacheCase::Capacity,
        Some(s) => match CacheCase::from_token(s) {
            Some(c) => c,
            None => {
                eprintln!("pedra diagnose get: --cache must be happy|capacity|cold");
                return Err(());
            }
        },
    };
    let b = scale_bottleneck(keys, ram, bpe);
    let f = &b.legal;
    let as_is = &b.as_is;
    let composed = predict_get_composed(keys, ram, bpe, INTEL_SERVER_4GHZ, cache);
    let (measured_ns, class, predict) = match flag_u64(args, "--measured-ns") {
        Some(ns) => (
            ns,
            classify_get(ns, f.best_ns, f.happy_ns, f.worst_ns, as_is.best_ns),
            false,
        ),
        None => (as_is.best_ns, b.walk_class, true),
    };
    let mode = if f.hot { "hot" } else { "bounded-cache" };
    println!("pedra diagnose get keys={keys} ram={ram} mode={mode} bpe={bpe}");
    if predict {
        println!(
            "predict=1 distinguishable={} cut={} (probes; no get ran)",
            u8::from(b.distinguishable),
            b.cut_token()
        );
    }
    println!(
        "P_best={} P_worst={} n_files={}",
        f.p_best, f.p_worst, f.n_files
    );
    println!(
        "work legal_probes={} as_is_files={} bloom_k={} index_cmps={} block_cmps={} key_bytes={}",
        composed.work_legal.probes,
        composed.work_as_is.probes,
        composed.work_legal.bloom_k,
        composed.work_legal.index_cmps,
        composed.work_legal.block_cmps,
        composed.work_legal.key_bytes
    );
    println!(
        "T_ns best={} happy={} worst={} as_is={}",
        f.best_ns, f.happy_ns, f.worst_ns, as_is.best_ns
    );
    println!("{}", composed.line());
    println!("measured_ns={measured_ns} class={}", class.token());
    println!(
        r#"{{"class":"{}","measured_ns":{},"best":{},"happy":{},"worst":{},"as_is":{},"distinguishable":{},"cut":"{}","composed_legal":{},"composed_as_is":{},"bloom_ns":{},"index_ns":{},"block_ns":{},"pread_ns":{},"bloom_level":"{}","block_level":"{}","dominant":"{}","cache":"{}"}}"#,
        class.token(),
        measured_ns,
        f.best_ns,
        f.happy_ns,
        f.worst_ns,
        as_is.best_ns,
        u8::from(b.distinguishable),
        b.cut_token(),
        composed.cost_legal.total_ns,
        composed.cost_as_is.total_ns,
        composed.cost_legal.bloom_ns,
        composed.cost_legal.index_ns,
        composed.cost_legal.block_ns,
        composed.cost_legal.pread_ns,
        composed.cost_legal.bloom_level.token(),
        composed.cost_legal.block_level.token(),
        composed.cost_legal.dominant.token(),
        cache.token()
    );
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
    println!(
        r#"{{"class":"{}","per_get":{},"p_best":{}}}"#,
        class.token(),
        per_get,
        p_best
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
    let shapes_json = BALANCE_SHAPES
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(",");
    println!(
        r#"{{"admits":{},"cut":"{}","cells":{},"shapes":[{}]}}"#,
        u8::from(admits),
        cut.token(),
        cells.len(),
        shapes_json
    );
    Ok(())
}
