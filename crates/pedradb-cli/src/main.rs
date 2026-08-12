//! PedraDB CLI — thin command-line front-end over `pedradb-core`.

use pedradb_core::wal::Wal;
use pedradb_core::Db;

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: pedra <demo|wal|version> [args...]");
        return std::process::ExitCode::from(2);
    }
    match args[1].as_str() {
        "version" => {
            println!("pedra {}", env!("CARGO_PKG_VERSION"));
            std::process::ExitCode::SUCCESS
        }
        "wal" => wal_cmd(&args[2..]),
        "demo" => demo_cmd(&args[2..]),
        other => {
            eprintln!("unknown command: {other}");
            std::process::ExitCode::from(2)
        }
    }
}

fn demo_cmd(args: &[String]) -> std::process::ExitCode {
    let path = args.first().map(String::as_str).unwrap_or("/tmp/pedra-demo");
    if let Err(e) = run_db_demo(path) {
        eprintln!("error: {e}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}

fn run_db_demo(path: &str) -> pedradb_core::Result<()> {
    let mut db = Db::open(path)?;
    {
        let mut tx = db.begin();
        tx.put(b"u/1", br#"{"name":"ada"}"#)?;
        tx.put(b"idx/name/ada", b"1")?;
        tx.commit()?;
    }
    println!("opened {path}");
    println!(
        "  u/1 = {:?}",
        db.get(b"u/1").map(|b| String::from_utf8_lossy(&b).into_owned())
    );
    println!(
        "  idx/name/ada = {:?}",
        db.get(b"idx/name/ada")
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    );
    println!("  last_sequence = {}", db.last_sequence());
    db.close()?;
    // Prove recover
    let db2 = Db::open(path)?;
    assert_eq!(db2.get(b"u/1").as_deref(), Some(br#"{"name":"ada"}"#.as_ref()));
    println!("reopen ok — multi-key TX still present");
    Ok(())
}

fn wal_cmd(args: &[String]) -> std::process::ExitCode {
    if args.is_empty() {
        eprintln!("usage: pedra wal <path>         # create a WAL, append a demo record, fsync, and recover it");
        return std::process::ExitCode::from(2);
    }
    let path = &args[0];
    if let Err(e) = run_wal_demo(path) {
        eprintln!("error: {e}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}

fn run_wal_demo(path: &str) -> pedradb_core::Result<()> {
    {
        let mut wal = Wal::create(path)?;
        wal.append_record(b"the first durable write")?;
        wal.append_record(b"and the second")?;
        wal.sync_all()?;
        wal.close()?;
    }
    let records = Wal::recover(path)?;
    println!("recovered {} records from {path}:", records.len());
    for (i, rec) in records.iter().enumerate() {
        println!("  [{i}] {:>6} bytes: {:?}", rec.len(), String::from_utf8_lossy(rec));
    }
    Ok(())
}
