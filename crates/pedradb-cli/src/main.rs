//! PedraDB CLI — demo, WAL smoke, backup / PITR / migrate.

use pedradb_core::wal::Wal;
use pedradb_core::{Db, OpenOptions};
use pedradb_ops::{inspect_format, migrate_to_latest, BackupEngine};

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "usage: pedra <demo|wal|version|backup|restore|pitr|ship-wal|list-backups|verify-backup|inspect|stats|compact|reclaim|maintain|compact-vlog|compact-blob|blob-gc|migrate> [args...]"
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
        "compact" => compact_cmd(&args[2..]),
        "reclaim" => reclaim_cmd(&args[2..]),
        "maintain" => maintain_cmd(&args[2..]),
        "compact-vlog" => compact_vlog_cmd(&args[2..]),
        "compact-blob" => compact_blob_cmd(&args[2..]),
        "blob-gc" => blob_gc_cmd(&args[2..]),
        "migrate" => migrate_cmd(&args[2..]),
        other => {
            eprintln!("unknown command: {other}");
            std::process::ExitCode::from(2)
        }
    }
}

fn demo_cmd(args: &[String]) -> std::process::ExitCode {
    let path = args
        .first()
        .map(String::as_str)
        .unwrap_or("/tmp/pedra-demo");
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
        db.get(b"u/1")
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    );
    println!(
        "  idx/name/ada = {:?}",
        db.get(b"idx/name/ada")
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    );
    println!("  last_sequence = {}", db.last_sequence());
    db.close()?;
    let db2 = Db::open(path)?;
    assert_eq!(
        db2.get(b"u/1").as_deref(),
        Some(br#"{"name":"ada"}"#.as_ref())
    );
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
            wal_recovery: Default::default(),
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
            "base backup id={} seq={} ssts={} earliest_readable={} path={}",
            meta.id,
            meta.base_sequence,
            meta.sst_count,
            meta.earliest_readable_seq,
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
            ship.records, ship.last_shipped_sequence, ship.segment
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
        let rep = inspect_format(&args[2])?;
        println!(
            "restored backup {id} -> {} earliest_readable={} ssts={}",
            args[2], rep.earliest_readable_seq, rep.sst_count
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
        let rep = inspect_format(&args[3])?;
        println!(
            "pitr backup {id} to seq {seq} -> {} earliest_readable={} ssts={}",
            args[3], rep.earliest_readable_seq, rep.sst_count
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

fn list_backups_cmd(args: &[String]) -> std::process::ExitCode {
    if args.is_empty() {
        eprintln!("usage: pedra list-backups <backup_root>");
        return std::process::ExitCode::from(2);
    }
    match (|| -> Result<(), Box<dyn std::error::Error>> {
        let eng = BackupEngine::open(&args[0])?;
        for b in eng.list_backups()? {
            println!(
                "id={} base_seq={} ssts={} earliest_readable={} path={}",
                b.id,
                b.base_sequence,
                b.sst_count,
                b.earliest_readable_seq,
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
            "ok id={id} seq={} ssts={} earliest_readable={}",
            m.last_sequence, m.sst_count, m.earliest_readable_seq
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
            println!("{}", s.gc_line());
            println!("auto_blob_gc={:?}", db.auto_blob_gc_min_ratio());
            if !s.last_auto_compact_error.is_empty() {
                println!("last_auto_compact_error={}", s.last_auto_compact_error);
            }
            println!("{}", s.vlog_line());
            println!(
                "scan_prefetch={} blob_active={}",
                db.scan_prefetch(),
                db.blob_active()
            );
            if let Ok(cands) = db.blob_gc_candidates() {
                for c in cands {
                    println!(
                        "blob file={} bytes={} live={}B records={} dead_ratio={:.3} active={}",
                        c.file_num,
                        c.bytes,
                        c.live_bytes,
                        c.live_records,
                        c.dead_ratio,
                        c.is_active
                    );
                }
            }
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn compact_cmd(args: &[String]) -> std::process::ExitCode {
    // pedra compact <db>              — leveled merge (history-preserving)
    // pedra compact <db> --latest-only — aggressive version GC
    if args.is_empty() {
        eprintln!("usage: pedra compact <db_path> [--latest-only]");
        return std::process::ExitCode::from(2);
    }
    let latest = args.iter().any(|a| a == "--latest-only");
    match open_live(&args[0]) {
        Ok(mut db) => {
            let before = db.earliest_readable_sequence();
            let r = if latest {
                db.compact_with(pedradb_core::CompactOptions::latest_only())
            } else {
                db.compact()
            };
            match r {
                Ok(()) => {
                    println!(
                        "compact ok latest_only={latest} earliest_readable {} → {} sst_count={}",
                        before,
                        db.earliest_readable_sequence(),
                        db.sst_count()
                    );
                    std::process::ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::ExitCode::FAILURE
                }
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn reclaim_cmd(args: &[String]) -> std::process::ExitCode {
    // pedra reclaim <db> — pin-aware version GC (compact_reclaim)
    if args.is_empty() {
        eprintln!("usage: pedra reclaim <db_path>");
        return std::process::ExitCode::from(2);
    }
    match open_live(&args[0]) {
        Ok(mut db) => {
            let before = db.earliest_readable_sequence();
            let pins = db.snapshot_pin_count();
            match db.compact_reclaim() {
                Ok(()) => {
                    println!(
                        "reclaim ok pins={pins} earliest_readable {} → {} last_sequence={} sst_count={}",
                        before,
                        db.earliest_readable_sequence(),
                        db.last_sequence(),
                        db.sst_count()
                    );
                    std::process::ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::ExitCode::FAILURE
                }
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Operator-side maintenance (open-items §2.2 residual: no bg thread in core).
///
/// ```text
/// pedra maintain <db> [--blob-theta 0.5] [--no-reclaim] [--vlog] [--every SECS]
/// ```
///
/// One pass: flush → optional compact_reclaim → compact_blob_auto(θ) → optional
/// compact_vlog. With `--every N`, repeat every N seconds until SIGINT (cron
/// substitute; still outside pedradb-core).
fn maintain_cmd(args: &[String]) -> std::process::ExitCode {
    if args.is_empty() {
        eprintln!(
            "usage: pedra maintain <db_path> [--blob-theta 0.5] [--no-reclaim] [--vlog] [--every SECS]"
        );
        return std::process::ExitCode::from(2);
    }
    let path = args[0].clone();
    let mut blob_theta: f64 = 0.5;
    let mut do_reclaim = true;
    let mut do_vlog = false;
    let mut every: Option<u64> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--blob-theta" => {
                i += 1;
                let Some(t) = args.get(i).and_then(|s| s.parse().ok()) else {
                    eprintln!("usage: --blob-theta requires a number");
                    return std::process::ExitCode::from(2);
                };
                blob_theta = t;
            }
            "--no-reclaim" => do_reclaim = false,
            "--vlog" => do_vlog = true,
            "--every" => {
                i += 1;
                let Some(secs) = args.get(i).and_then(|s| s.parse().ok()) else {
                    eprintln!("usage: --every requires seconds ≥ 1");
                    return std::process::ExitCode::from(2);
                };
                if secs == 0 {
                    eprintln!("--every must be ≥ 1");
                    return std::process::ExitCode::from(2);
                }
                every = Some(secs);
            }
            other => {
                eprintln!("unknown flag: {other}");
                return std::process::ExitCode::from(2);
            }
        }
        i += 1;
    }

    let mut pass: u64 = 0;
    loop {
        pass = pass.saturating_add(1);
        match maintain_once(&path, do_reclaim, do_vlog, blob_theta, pass) {
            Ok(()) => {}
            Err(code) => return code,
        }
        let Some(secs) = every else {
            return std::process::ExitCode::SUCCESS;
        };
        println!("maintain: sleep {secs}s (pass={pass}, Ctrl-C to stop)");
        std::thread::sleep(std::time::Duration::from_secs(secs));
    }
}

fn maintain_once(
    path: &str,
    do_reclaim: bool,
    do_vlog: bool,
    blob_theta: f64,
    pass: u64,
) -> Result<(), std::process::ExitCode> {
    let mut db = open_live(path).map_err(|e| {
        eprintln!("error: {e}");
        std::process::ExitCode::FAILURE
    })?;
    let early0 = db.earliest_readable_sequence();
    db.flush().map_err(|e| {
        eprintln!("error flush: {e}");
        std::process::ExitCode::FAILURE
    })?;
    if do_reclaim {
        db.compact_reclaim().map_err(|e| {
            eprintln!("error reclaim: {e}");
            std::process::ExitCode::FAILURE
        })?;
    }
    let blob = db.compact_blob_auto(blob_theta).map_err(|e| {
        eprintln!("error blob-gc: {e}");
        std::process::ExitCode::FAILURE
    })?;
    let mut vlog_line = String::new();
    if do_vlog {
        match db.compact_vlog() {
            Ok(st) => {
                vlog_line = format!(" vlog_rewrite {}B→{}B", st.bytes_before, st.bytes_after);
            }
            Err(e) => {
                eprintln!("error compact-vlog: {e}");
                return Err(std::process::ExitCode::FAILURE);
            }
        }
    }
    let blob_s = match blob {
        Some((n, st)) => format!(" blob_gc file={n} {}B→{}B", st.bytes_before, st.bytes_after),
        None => " blob_gc=skip".into(),
    };
    println!(
        "maintain pass={pass} reclaim={do_reclaim} earliest {}→{} sst={}{}{} {}",
        early0,
        db.earliest_readable_sequence(),
        db.sst_count(),
        blob_s,
        vlog_line,
        db.stats().vlog_line()
    );
    db.close().map_err(|e| {
        eprintln!("error close: {e}");
        std::process::ExitCode::FAILURE
    })?;
    Ok(())
}

fn compact_vlog_cmd(args: &[String]) -> std::process::ExitCode {
    if args.is_empty() {
        eprintln!("usage: pedra compact-vlog <db_path>");
        return std::process::ExitCode::from(2);
    }
    match open_live(&args[0]) {
        Ok(mut db) => match db.compact_vlog() {
            Ok(st) => {
                println!(
                    "compact_vlog before={}B after={}B live_records={}",
                    st.bytes_before, st.bytes_after, st.live_records
                );
                println!("{}", db.stats().vlog_line());
                std::process::ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                std::process::ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn compact_blob_cmd(args: &[String]) -> std::process::ExitCode {
    // pedra compact-blob <db> <file_num>
    // pedra compact-blob <db> --auto [min_dead_ratio]
    if args.len() < 2 {
        eprintln!(
            "usage: pedra compact-blob <db_path> <file_num>\n       pedra compact-blob <db_path> --auto [min_dead_ratio]"
        );
        return std::process::ExitCode::from(2);
    }
    let path = &args[0];
    match open_live(path) {
        Ok(mut db) => {
            if args[1] == "--auto" {
                let theta: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.5);
                match db.compact_blob_auto(theta) {
                    Ok(Some((num, st))) => {
                        println!(
                            "compact_blob_auto file={} before={}B after={}B live_records={} theta={theta}",
                            num, st.bytes_before, st.bytes_after, st.live_records
                        );
                        println!("{}", db.stats().vlog_line());
                        std::process::ExitCode::SUCCESS
                    }
                    Ok(None) => {
                        println!("compact_blob_auto: nothing to do (theta={theta})");
                        std::process::ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        std::process::ExitCode::FAILURE
                    }
                }
            } else {
                let Ok(num) = args[1].parse::<u32>() else {
                    eprintln!("bad file_num: {}", args[1]);
                    return std::process::ExitCode::from(2);
                };
                match db.compact_blob(num) {
                    Ok(st) => {
                        println!(
                            "compact_blob file={} before={}B after={}B live_records={}",
                            num, st.bytes_before, st.bytes_after, st.live_records
                        );
                        println!("{}", db.stats().vlog_line());
                        std::process::ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        std::process::ExitCode::FAILURE
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn blob_gc_cmd(args: &[String]) -> std::process::ExitCode {
    // Alias: pedra blob-gc <db> [--auto [theta]]  (list candidates or auto)
    if args.is_empty() {
        eprintln!("usage: pedra blob-gc <db_path> [--auto [min_dead_ratio]]");
        return std::process::ExitCode::from(2);
    }
    if args.get(1).map(String::as_str) == Some("--auto") {
        let mut a = vec![args[0].clone(), "--auto".into()];
        if let Some(t) = args.get(2) {
            a.push(t.clone());
        }
        return compact_blob_cmd(&a);
    }
    match open_live(&args[0]) {
        Ok(db) => match db.blob_gc_candidates() {
            Ok(cands) => {
                if cands.is_empty() {
                    println!("no blob files");
                }
                for c in cands {
                    println!(
                        "file={} bytes={} live={}B records={} dead_ratio={:.3} active={}",
                        c.file_num,
                        c.bytes,
                        c.live_bytes,
                        c.live_records,
                        c.dead_ratio,
                        c.is_active
                    );
                }
                std::process::ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                std::process::ExitCode::FAILURE
            }
        },
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
            println!(
                "earliest_readable={} vlog_use_new={}",
                r.earliest_readable_seq, r.vlog_use_new
            );
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
