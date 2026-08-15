//! PedraDB CLI — demo, WAL smoke, backup / PITR / migrate.

use pedradb_core::wal::Wal;
use pedradb_core::{Db, OpenOptions};
use pedradb_ops::{inspect_format, migrate_to_latest, BackupEngine};

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "usage: pedra <demo|wal|version|backup|restore|pitr|ship-wal|list-backups|verify-backup|inspect|stats|migrate> [args...]"
        );
        return std::process::ExitCode::from(2);
    }
    match args[1].as_str() {
        "version" => {
            println!("pedra {}", env!("CARGO_PKG_VERSION"));
            std::process::ExitCode::SUCCESS
        }
        "wal" => wal_cmd(&args[2..]),
        "demo" => demo_cmd(&args[2..]),
        "backup" => backup_cmd(&args[2..]),
        "ship-wal" => ship_wal_cmd(&args[2..]),
        "restore" => restore_cmd(&args[2..]),
        "pitr" => pitr_cmd(&args[2..]),
        "list-backups" => list_backups_cmd(&args[2..]),
        "verify-backup" => verify_backup_cmd(&args[2..]),
        "inspect" => inspect_cmd(&args[2..]),
        "stats" => stats_cmd(&args[2..]),
        "migrate" => migrate_cmd(&args[2..]),
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
    let db2 = Db::open(path)?;
    assert_eq!(db2.get(b"u/1").as_deref(), Some(br#"{"name":"ada"}"#.as_ref()));
    println!("reopen ok — multi-key TX still present");
    Ok(())
}

fn wal_cmd(args: &[String]) -> std::process::ExitCode {
    if args.is_empty() {
        eprintln!(
            "usage: pedra wal <path>         # create a WAL, append a demo record, fsync, and recover it"
        );
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
    let recs = Wal::recover(path)?;
    println!("recovered {} records from {path}", recs.len());
    for (i, r) in recs.iter().enumerate() {
        println!("  [{i}] {:?}", String::from_utf8_lossy(r));
    }
    Ok(())
}

fn open_live(path: &str) -> pedradb_core::Result<Db> {
    Db::open_with(
        path,
        OpenOptions {
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
                large_value_threshold: None,
        },
    )
}

fn backup_cmd(args: &[String]) -> std::process::ExitCode {
    // pedra backup <db_path> <backup_root>
    if args.len() < 2 {
        eprintln!("usage: pedra backup <db_path> <backup_root>");
        return std::process::ExitCode::from(2);
    }
    match (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut db = open_live(&args[0])?;
        let mut eng = BackupEngine::open(&args[1])?;
        let meta = eng.create_base_backup(&mut db)?;
        println!(
            "base backup id={} seq={} ssts={} path={}",
            meta.id,
            meta.base_sequence,
            meta.sst_count,
            meta.path.display()
        );
        db.close()?;
        Ok(())
    })() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn ship_wal_cmd(args: &[String]) -> std::process::ExitCode {
    // pedra ship-wal <db_path> <backup_root>
    if args.len() < 2 {
        eprintln!("usage: pedra ship-wal <db_path> <backup_root>");
        return std::process::ExitCode::from(2);
    }
    match (|| -> Result<(), Box<dyn std::error::Error>> {
        let db = open_live(&args[0])?;
        let mut eng = BackupEngine::open(&args[1])?;
        let ship = eng.ship_wal(&db)?;
        println!(
            "shipped records={} last_seq={} segment={:?}",
            ship.records,
            ship.last_shipped_sequence,
            ship.segment
        );
        db.close()?;
        Ok(())
    })() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn restore_cmd(args: &[String]) -> std::process::ExitCode {
    // pedra restore <backup_root> <backup_id> <dest>
    if args.len() < 3 {
        eprintln!("usage: pedra restore <backup_root> <backup_id> <dest>");
        return std::process::ExitCode::from(2);
    }
    match (|| -> Result<(), Box<dyn std::error::Error>> {
        let eng = BackupEngine::open(&args[0])?;
        let id: u64 = args[1].parse()?;
        eng.restore(id, &args[2])?;
        println!("restored backup {id} -> {}", args[2]);
        Ok(())
    })() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn pitr_cmd(args: &[String]) -> std::process::ExitCode {
    // pedra pitr <backup_root> <backup_id> <target_seq> <dest>
    if args.len() < 4 {
        eprintln!("usage: pedra pitr <backup_root> <backup_id> <target_seq> <dest>");
        return std::process::ExitCode::from(2);
    }
    match (|| -> Result<(), Box<dyn std::error::Error>> {
        let eng = BackupEngine::open(&args[0])?;
        let id: u64 = args[1].parse()?;
        let seq: u64 = args[2].parse()?;
        eng.restore_pitr(id, &args[3], Some(seq))?;
        println!("pitr backup {id} to seq {seq} -> {}", args[3]);
        Ok(())
    })() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn list_backups_cmd(args: &[String]) -> std::process::ExitCode {
    if args.is_empty() {
        eprintln!("usage: pedra list-backups <backup_root>");
        return std::process::ExitCode::from(2);
    }
    match (|| -> Result<(), Box<dyn std::error::Error>> {
        let eng = BackupEngine::open(&args[0])?;
        for b in eng.list_backups()? {
            println!(
                "id={} base_seq={} ssts={} path={}",
                b.id,
                b.base_sequence,
                b.sst_count,
                b.path.display()
            );
        }
        println!("last_shipped_seq={}", eng.last_shipped_sequence());
        Ok(())
    })() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn verify_backup_cmd(args: &[String]) -> std::process::ExitCode {
    if args.len() < 2 {
        eprintln!("usage: pedra verify-backup <backup_root> <backup_id>");
        return std::process::ExitCode::from(2);
    }
    match (|| -> Result<(), Box<dyn std::error::Error>> {
        let eng = BackupEngine::open(&args[0])?;
        let id: u64 = args[1].parse()?;
        let m = eng.verify_backup(id)?;
        println!(
            "ok id={id} seq={} ssts={}",
            m.last_sequence, m.sst_count
        );
        Ok(())
    })() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn stats_cmd(args: &[String]) -> std::process::ExitCode {
    if args.is_empty() {
        eprintln!("usage: pedra stats <db_path>");
        return std::process::ExitCode::from(2);
    }
    match open_live(&args[0]) {
        Ok(db) => {
            let s = db.stats();
            println!("last_sequence={}", s.last_sequence);
            println!("sst_count={} sst_bytes={}", s.sst_count, s.sst_bytes);
            println!("wal_bytes={} wal_syncs={}", s.wal_bytes, s.wal_sync_count);
            println!("{}", s.vlog_line());
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn inspect_cmd(args: &[String]) -> std::process::ExitCode {
    if args.is_empty() {
        eprintln!("usage: pedra inspect <db_path>");
        return std::process::ExitCode::from(2);
    }
    match inspect_format(&args[0]) {
        Ok(r) => {
            println!("has_manifest={}", r.has_manifest);
            println!("sst_count={}", r.sst_count);
            println!("needs_migration={}", r.needs_migration);
            for (num, ver) in &r.sst_versions {
                println!("  sst {num:06} version={ver}");
            }
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn migrate_cmd(args: &[String]) -> std::process::ExitCode {
    if args.is_empty() {
        eprintln!("usage: pedra migrate <db_path>");
        return std::process::ExitCode::from(2);
    }
    match migrate_to_latest(&args[0]) {
        Ok(m) => {
            println!(
                "migrated ssts_rewritten={} last_seq={} verified={}",
                m.ssts_rewritten, m.last_sequence, m.verified
            );
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
