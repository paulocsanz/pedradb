//! PedraDB CLI — demo, WAL smoke, backup / PITR / migrate.

#![forbid(unsafe_code)]

use pedradb_core::scale_kernel::{scale_forecast, SCALE_HAPPY_NOISY_BPS, SCALE_WORST_NOISY_BPS};
use pedradb_core::wal::Wal;
use pedradb_core::{
    verified_admits_ring, verify_at_rest, BlobGcCandidate, CompactOptions, Db, DbStats, Env,
    OpenOptions, SequenceNumber, StdEnv, VlogRewriteStats, PROFILE_VERSION,
};
use pedradb_io_uring::{open_with as open_db_with, production_env, IoUringEnv};
use pedradb_ops::{inspect_format, migrate_to_latest, restore_history_from_remote, BackupEngine};

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "usage: pedra <demo|wal|version|backup|restore|pitr|ship-wal|list-backups|verify-backup|verify|archive|inspect|stats|scale-model|compact|reclaim|maintain|compact-vlog|compact-blob|blob-gc|migrate> [args...]"
        );
        eprintln!(
            "env: PEDRA_VERIFIED=1 runs every command on the verified profile (RFC-0058 P2.3)"
        );
        return std::process::ExitCode::from(2);
    }
    if verified_requested() {
        eprintln!(
            "pedra: PEDRA_VERIFIED=1 — verified profile {PROFILE_VERSION} (StdEnv, no io_uring ring, posix(); verified_admits_ring={}; RFC-0080)",
            u8::from(verified_admits_ring(true)),
        );
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
        "verify" => verify_cmd(&args[2..]),
        "archive" => archive_cmd(&args[2..]),
        "inspect" => inspect_cmd(&args[2..]),
        "stats" => stats_cmd(&args[2..]),
        "scale-model" => scale_model_cmd(&args[2..]),
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
    if verified_requested() {
        demo_body(path, open_verified_db)
    } else {
        demo_body(path, open_full_db)
    }
}

fn demo_body<E: Env>(
    path: &str,
    mut open: impl FnMut(&str) -> pedradb_core::Result<Db<E>>,
) -> pedradb_core::Result<()> {
    let mut db = open(path)?;
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
    let db2 = open(path)?;
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

/// RFC-0058 P2.3 — `PEDRA_VERIFIED=1` is the one-line product switch:
/// every CLI command that opens a live database runs the verified
/// profile (`StdEnv` — no io_uring ring, P2.2 — plus
/// `OpenOptions::verified()`: `sync`, full-WAL fsync, fail-closed
/// recovery). Any other value (or unset) keeps the full mode
/// (`IoUringEnv` on Linux, `PosixFallback` where the ring is
/// unavailable).
fn verified_requested() -> bool {
    std::env::var_os("PEDRA_VERIFIED").is_some_and(|v| v == "1")
}

fn open_full_db(path: &str) -> pedradb_core::Result<Db<IoUringEnv>> {
    open_db_with(
        path,
        OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
            sst_payload_budget_bytes: None,
        },
    )
}

fn open_verified_db(path: &str) -> pedradb_core::Result<Db<IoUringEnv>> {
    // RFC-0080 P1.1: ring only if `verified_admits_ring` (always false).
    // Same `Db<IoUringEnv>` type as full mode — a second `Db<StdEnv>`
    // monomorph in this binary SIGSEGV'd TX commit under release
    // (`Vec<WriteOp>::as_slice` on a garbage pointer). `posix()` is
    // StdEnv underneath.
    let env = if verified_admits_ring(true) {
        production_env()
    } else {
        IoUringEnv::posix()
    };
    Db::open_with_env(path, OpenOptions::verified(), env)
}

/// The handle every live-open command uses: full mode or verified
/// mode, chosen by `PEDRA_VERIFIED` (RFC-0058 P2.3).
enum LiveDb {
    Full(Db<IoUringEnv>),
    Verified(Db<IoUringEnv>),
}

macro_rules! for_both {
    ($self:ident, $db:ident => $body:expr) => {
        match $self {
            LiveDb::Full($db) => $body,
            LiveDb::Verified($db) => $body,
        }
    };
}

impl LiveDb {
    fn close(self) -> pedradb_core::Result<()> {
        for_both!(self, db => db.close())
    }

    fn create_base_backup(
        &mut self,
        backup_root: &str,
    ) -> pedradb_ops::Result<pedradb_ops::BackupMeta> {
        match self {
            LiveDb::Full(db) => {
                let mut eng = BackupEngine::open(backup_root)?;
                eng.create_base_backup(db)
            }
            LiveDb::Verified(db) => {
                let mut eng = BackupEngine::open_with_env(backup_root, IoUringEnv::posix())?;
                eng.create_base_backup(db)
            }
        }
    }

    fn ship_wal(&self, backup_root: &str) -> pedradb_ops::Result<pedradb_ops::WalShipMeta> {
        match self {
            LiveDb::Full(db) => {
                let mut eng = BackupEngine::open(backup_root)?;
                eng.ship_wal(db)
            }
            LiveDb::Verified(db) => {
                let mut eng = BackupEngine::open_with_env(backup_root, IoUringEnv::posix())?;
                eng.ship_wal(db)
            }
        }
    }

    fn stats(&self) -> DbStats {
        for_both!(self, db => db.stats())
    }

    fn last_sequence(&self) -> SequenceNumber {
        for_both!(self, db => db.last_sequence())
    }

    fn sst_count(&self) -> usize {
        for_both!(self, db => db.sst_count())
    }

    fn snapshot_pin_count(&self) -> usize {
        for_both!(self, db => db.snapshot_pin_count())
    }

    fn earliest_readable_sequence(&self) -> SequenceNumber {
        for_both!(self, db => db.earliest_readable_sequence())
    }

    fn auto_blob_gc_min_ratio(&self) -> Option<f64> {
        for_both!(self, db => db.auto_blob_gc_min_ratio())
    }

    fn scan_prefetch(&self) -> usize {
        for_both!(self, db => db.scan_prefetch())
    }

    fn blob_active(&self) -> u32 {
        for_both!(self, db => db.blob_active())
    }

    fn blob_gc_candidates(&self) -> pedradb_core::Result<Vec<BlobGcCandidate>> {
        for_both!(self, db => db.blob_gc_candidates())
    }

    fn flush(&mut self) -> pedradb_core::Result<()> {
        for_both!(self, db => db.flush())
    }

    fn compact(&mut self) -> pedradb_core::Result<()> {
        for_both!(self, db => db.compact())
    }

    fn compact_with(&mut self, options: CompactOptions) -> pedradb_core::Result<()> {
        for_both!(self, db => db.compact_with(options))
    }

    fn compact_reclaim(&mut self) -> pedradb_core::Result<()> {
        for_both!(self, db => db.compact_reclaim())
    }

    fn compact_vlog(&mut self) -> pedradb_core::Result<VlogRewriteStats> {
        for_both!(self, db => db.compact_vlog())
    }

    fn compact_blob_auto(
        &mut self,
        min_dead_ratio: f64,
    ) -> pedradb_core::Result<Option<(u32, VlogRewriteStats)>> {
        for_both!(self, db => db.compact_blob_auto(min_dead_ratio))
    }

    fn compact_blob(&mut self, file_num: u32) -> pedradb_core::Result<VlogRewriteStats> {
        for_both!(self, db => db.compact_blob(file_num))
    }
}

fn open_live(path: &str) -> pedradb_core::Result<LiveDb> {
    if verified_requested() {
        Ok(LiveDb::Verified(open_verified_db(path)?))
    } else {
        Ok(LiveDb::Full(open_full_db(path)?))
    }
}

fn backup_cmd(args: &[String]) -> std::process::ExitCode {
    // pedra backup <db_path> <backup_root>
    if args.len() < 2 {
        eprintln!("usage: pedra backup <db_path> <backup_root>");
        return std::process::ExitCode::from(2);
    }
    match (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut db = open_live(&args[0])?;
        let meta = db.create_base_backup(&args[1])?;
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
        let ship = db.ship_wal(&args[1])?;
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

/// RFC-0046 P1.4: remote history-tier inspection and restore.
fn archive_cmd(args: &[String]) -> std::process::ExitCode {
    // pedra archive status <remote_root>
    // pedra archive verify <remote_root>
    // pedra archive restore <remote_root> <dest> [target_seq]
    match args.first().map(String::as_str) {
        Some("status") if args.len() >= 2 => {
            match (|| -> Result<(), Box<dyn std::error::Error>> {
                let tier = pedradb_core::history::RemoteTier::new(&args[1]);
                match tier.latest_summary(&production_env())? {
                    Some(s) => {
                        println!(
                        "segments={} bytes={} seq_range={}-{} archive_floor={} next_generation={}",
                        s.segments, s.bytes, s.from_seq, s.through_seq, s.archive_floor,
                        s.next_generation
                    )
                    }
                    None => println!("no manifest — remote tier is empty"),
                }
                Ok(())
            })() {
                Ok(()) => std::process::ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::ExitCode::FAILURE
                }
            }
        }
        Some("verify") if args.len() >= 2 => match (|| -> Result<(), Box<dyn std::error::Error>> {
            let tier = pedradb_core::history::RemoteTier::new(&args[1]);
            let r = tier.verify(&production_env())?;
            println!("{}", r.summary_line());
            for (file, msg) in &r.failures {
                println!("FAIL {file} {msg}");
            }
            if r.is_clean() {
                Ok(())
            } else {
                Err(format!("archive verify {}", r.summary_line()).into())
            }
        })() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::ExitCode::FAILURE
            }
        },
        Some("restore") if args.len() >= 3 => {
            match (|| -> Result<(), Box<dyn std::error::Error>> {
                let target = match args.get(3) {
                    Some(s) => Some(s.parse::<u64>()?),
                    None => None,
                };
                let rep =
                    restore_history_from_remote(&production_env(), &args[1], &args[2], target)?;
                println!(
                    "restored {} segments / {} records -> {} last_sequence={} (target={:?})",
                    rep.segments, rep.records, args[2], rep.last_sequence, target
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
        _ => {
            eprintln!(
                "usage: pedra archive status <remote_root> | pedra archive verify <remote_root> | pedra archive restore <remote_root> <dest> [target_seq]"
            );
            std::process::ExitCode::from(2)
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

fn scale_model_cmd(args: &[String]) -> std::process::ExitCode {
    let mut keys: Option<u64> = None;
    let mut ram: Option<u64> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--keys" => {
                let Some(v) = args.get(i + 1).and_then(|s| s.parse().ok()) else {
                    eprintln!("usage: pedra scale-model --keys N --ram R");
                    return std::process::ExitCode::from(2);
                };
                keys = Some(v);
                i += 2;
            }
            "--ram" => {
                let Some(v) = args.get(i + 1).and_then(|s| s.parse().ok()) else {
                    eprintln!("usage: pedra scale-model --keys N --ram R");
                    return std::process::ExitCode::from(2);
                };
                ram = Some(v);
                i += 2;
            }
            other => {
                eprintln!("unknown scale-model flag: {other}");
                eprintln!("usage: pedra scale-model --keys N --ram R");
                return std::process::ExitCode::from(2);
            }
        }
    }
    let (Some(keys), Some(ram)) = (keys, ram) else {
        eprintln!("usage: pedra scale-model --keys N --ram R");
        return std::process::ExitCode::from(2);
    };
    print_scale_forecast(keys, ram);
    std::process::ExitCode::SUCCESS
}

/// RFC-0176 P1.2: numbers come from [`scale_forecast`], not a second formula.
fn print_scale_forecast(keys: u64, ram: u64) {
    let f = scale_forecast(keys, ram);
    let mode = if f.hot { "hot" } else { "bounded-cache" };
    let hot_bit = u8::from(f.hot);
    println!("scale-model keys={} ram={}", f.keys, f.ram_bytes);
    println!("S={} L={}", f.store_bytes, f.levels);
    println!("P_best={} P_worst={}", f.p_best, f.p_worst);
    println!("n_files={} warm_cap={}", f.n_files, f.warm_cap);
    println!("hot={hot_bit} mode={mode}");
    println!(
        "T_ns best={} happy={} worst={}",
        f.best_ns, f.happy_ns, f.worst_ns
    );
    println!(
        "eta_happy_bps={} eta_worst_bps={} happy_hot_bps={}",
        SCALE_HAPPY_NOISY_BPS, SCALE_WORST_NOISY_BPS, f.happy_hot_bps
    );
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
            if !pedradb_core::write_admission_kernel::batch_is_empty(
                s.last_auto_compact_error.len() as u64,
            ) {
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
/// pedra maintain <db> [--blob-theta 0.5] [--no-reclaim] [--vlog] [--verify] [--every SECS]
/// ```
///
/// One pass: flush → optional compact_reclaim → compact_blob_auto(θ) → optional
/// compact_vlog → optional at-rest scrub (`--verify`, RFC-0060). With `--every N`,
/// repeat every N seconds until SIGINT (cron substitute; still outside pedradb-core).
fn maintain_cmd(args: &[String]) -> std::process::ExitCode {
    if args.is_empty() {
        eprintln!(
            "usage: pedra maintain <db_path> [--blob-theta 0.5] [--no-reclaim] [--vlog] [--verify] [--every SECS]"
        );
        return std::process::ExitCode::from(2);
    }
    let path = args[0].clone();
    let mut blob_theta: f64 = 0.5;
    let mut do_reclaim = true;
    let mut do_vlog = false;
    let mut do_verify = false;
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
            "--verify" => do_verify = true,
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
        match maintain_once(&path, do_reclaim, do_vlog, blob_theta, pass, do_verify) {
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
    do_verify: bool,
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
    if do_verify {
        let r = emit_verify(path);
        if !r.is_clean() {
            return Err(std::process::ExitCode::FAILURE);
        }
    }
    Ok(())
}

/// RFC-0060 P0.1: at-rest CRC scrub. Does not open a writer (no LOCK).
/// RFC-0060 P2.14: a backup root (`CATALOG` present) also CRC-walks
/// `wal/*.warch` via [`BackupEngine::verify_wal_archive`].
fn verify_cmd(args: &[String]) -> std::process::ExitCode {
    if args.is_empty() {
        eprintln!("usage: pedra verify <db_path>");
        return std::process::ExitCode::from(2);
    }
    let r = emit_verify(&args[0]);
    let mut ok = r.is_clean();
    let catalog = std::path::Path::new(&args[0]).join("CATALOG");
    if catalog.exists() {
        match BackupEngine::open(&args[0]).and_then(|eng| eng.verify_wal_archive()) {
            Ok(n) => println!("warch_segments={n}"),
            Err(e) => {
                eprintln!("error: {e}");
                ok = false;
            }
        }
    }
    if ok {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

fn emit_verify(path: &str) -> pedradb_core::VerifyReport {
    let r = verify_at_rest(&StdEnv, path);
    println!("{}", r.summary_line());
    for f in &r.failures {
        println!("FAIL {} offset={} {}", f.file, f.offset, f.message);
    }
    r
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
                if pedradb_core::write_admission_kernel::batch_is_empty(cands.len() as u64) {
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
            println!("current_crc={}", r.current_crc);
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
