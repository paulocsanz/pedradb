//! Product CLI for the official scale ladder (RFC-0178 P0.10).
//!
//! ```text
//! pedra scale [--entries N] [--cache BYTES] [--backends NAME] [dir]
//! ```
//!
//! One process, one `n`. 25M/50M/100M stay one backend per process.
//! Defaults: entries=1M, cache=256 MiB, backends=pedradb, dir=/tmp/pedra-scale.
//! `SCALE_*` env still works; flags win.

#![forbid(unsafe_code)]

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
        _ => {
            eprintln!("usage: pedra scale [--entries N] [--cache BYTES] [--backends NAME] [dir]");
            std::process::exit(2);
        }
    }
}
