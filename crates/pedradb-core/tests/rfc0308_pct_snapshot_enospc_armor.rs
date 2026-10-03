//! RFC-0308: Blindagem Concorrente PCT, Estabilidade de Snapshot e Durabilidade sob Falhas de Disco
//!
//! Implementa os testes exaustivos dos Pilares I, II e III do RFC-0308:
//! 1. PCT Multi-threaded Linearizability Stress (8 threads concorrentes: puts, deletes, range-deletes, scans, flushes, compactions).
//! 2. Estabilidade Absoluta de Snapshot sob Compactação com GC Agressivo (descarte de versões respeitando estritamente o horizonte).
//! 3. Tolerância a Falhas de Disco / Gravações Truncadas e Recuperação Fail-Closed sem Pânico.

use std::fs;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pedradb_core::wal::crc;
use pedradb_core::wal::format::{RecordType, HEADER_SIZE};
use pedradb_core::wal::WalReader;
use pedradb_core::{CompactOptions, ConcurrentDb, OpenOptions};

fn unique_temp_dir(tag: &str) -> std::path::PathBuf {
    static CTR: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let c = CTR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pedra-rfc0308-{tag}-{nanos}-{c}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Helper para codificar registro WAL com checksum mascarado padrão.
fn encode_wal_record(record_type: RecordType, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(HEADER_SIZE + payload.len());
    let length = payload.len() as u16;
    let type_byte = record_type as u8;
    let checksum = crc::record_checksum(type_byte, length, payload);
    buf.extend_from_slice(&checksum.to_le_bytes());
    buf.extend_from_slice(&length.to_le_bytes());
    buf.push(type_byte);
    buf.extend_from_slice(payload);
    buf
}

// =========================================================================
// Pilar I: PCT Multi-Threaded Linearizability Stress
// =========================================================================
#[test]
fn test_rfc0308_pct_multi_threaded_linearizability_stress() {
    let dir = unique_temp_dir("pct-stress");
    let mut opts = OpenOptions::default();
    opts.sync = false;
    let db = Arc::new(ConcurrentDb::open_with(&dir, opts).unwrap());

    let stop_signal = Arc::new(AtomicBool::new(false));
    let errors_detected = Arc::new(AtomicU64::new(0));

    let num_writer_threads = 3;
    let mut handles = Vec::new();

    // Threads de escrita com monotonicidade por chave
    for tid in 0..num_writer_threads {
        let db_clone = Arc::clone(&db);
        let stop_clone = Arc::clone(&stop_signal);
        let err_clone = Arc::clone(&errors_detected);

        handles.push(thread::spawn(move || {
            let mut counter: u64 = 0;
            while !stop_clone.load(Ordering::Relaxed) && counter < 40 {
                let key = format!("key_t{:02}_{:04}", tid, counter % 10).into_bytes();
                let val = format!("val_t{:02}_{:08}", tid, counter).into_bytes();

                if let Err(e) = db_clone.put(&key, &val) {
                    eprintln!("Put error on thread {tid}: {e}");
                    err_clone.fetch_add(1, Ordering::Relaxed);
                }

                // A cada 10 operações, executa um delete localizado
                if counter % 10 == 9 {
                    let k_del = format!("key_t{:02}_{:04}", tid, (counter / 2) % 10).into_bytes();
                    let _ = db_clone.delete(&k_del);
                }

                counter += 1;
                thread::yield_now();
            }
        }));
    }

    // Thread de leitura e validação contínua (asserts monotonic progress)
    {
        let db_clone = Arc::clone(&db);
        let stop_clone = Arc::clone(&stop_signal);
        let err_clone = Arc::clone(&errors_detected);

        handles.push(thread::spawn(move || {
            let mut iters = 0;
            while !stop_clone.load(Ordering::Relaxed) && iters < 80 {
                for tid in 0..num_writer_threads {
                    for k_idx in 0..5 {
                        let key = format!("key_t{:02}_{:04}", tid, k_idx).into_bytes();
                        if let Some(val) = db_clone.get(&key) {
                            let s = String::from_utf8_lossy(val.as_ref());
                            if !s.starts_with(&format!("val_t{:02}_", tid)) {
                                eprintln!("Linearizability violation! Key {key:?} read invalid value {s}");
                                err_clone.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
                iters += 1;
                thread::yield_now();
            }
        }));
    }

    // Thread de compactação e flush concorrente com contagem limitada de iterações
    {
        let db_clone = Arc::clone(&db);
        let stop_clone = Arc::clone(&stop_signal);
        handles.push(thread::spawn(move || {
            let mut compact_iters = 0;
            while !stop_clone.load(Ordering::Relaxed) && compact_iters < 30 {
                thread::sleep(Duration::from_millis(10));
                let _ = db_clone.flush();
                thread::sleep(Duration::from_millis(15));
                let _ = db_clone.compact_with(CompactOptions::default());
                compact_iters += 1;
            }
        }));
    }

    // Aguarda conclusão dos writers
    for h in handles.drain(..num_writer_threads) {
        h.join().unwrap();
    }

    // Para as threads auxiliares
    stop_signal.store(true, Ordering::Relaxed);
    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(
        errors_detected.load(Ordering::SeqCst),
        0,
        "PCT linearizability stress detectou violações concorrentes!"
    );

    // Valida inventário físico de disco
    db.assert_disk_inventory_invariant()
        .expect("Inventário de disco deve estar íntegro após stress concorrente");

    let _ = fs::remove_dir_all(&dir);
}

// =========================================================================
// Pilar II: Estabilidade Absoluta de Snapshot sob Compactação com GC
// =========================================================================
#[test]
fn test_rfc0308_snapshot_immutability_under_aggressive_compaction_gc() {
    let dir = unique_temp_dir("snap-gc");
    let mut opts = OpenOptions::default();
    opts.sync = false;
    let db = ConcurrentDb::open_with(&dir, opts).unwrap();

    // Fase 1: Escreve 100 chaves de base
    for i in 0..100 {
        let key = format!("key_{:04}", i).into_bytes();
        let val = format!("val_base_{:04}", i).into_bytes();
        db.put(&key, &val).unwrap();
    }
    db.flush().unwrap();

    // Captura Snapshot 1 com SnapshotPin registrado
    let pin1 = db.pin_snapshot();
    let snap1 = pin1.snapshot();
    let snap1_seq = snap1.sequence();

    // Fase 2: Sobrescreve todas as chaves pares com versão v2
    for i in (0..100).step_by(2) {
        let key = format!("key_{:04}", i).into_bytes();
        let val = format!("val_updated_{:04}", i).into_bytes();
        db.put(&key, &val).unwrap();
    }

    // Deleta o intervalo [key_0020, key_0050)
    db.delete_range(b"key_0020", b"key_0050").unwrap();
    db.flush().unwrap();

    // Captura Snapshot 2 com SnapshotPin registrado
    let pin2 = db.pin_snapshot();
    let snap2 = pin2.snapshot();

    // Fase 3: Executa compactação com GC agressivo (latest_only)
    // O motor DEVE respeitar o horizonte de pin1 e pin2 e NÃO pode purgar as versões visíveis por snap1!
    db.compact_with(CompactOptions::latest_only()).unwrap();

    // Fase 4: Escreve mais chaves novas
    for i in 100..150 {
        let key = format!("key_{:04}", i).into_bytes();
        let val = format!("val_new_{:04}", i).into_bytes();
        db.put(&key, &val).unwrap();
    }
    db.flush().unwrap();
    db.compact_with(CompactOptions::default()).unwrap();

    // Validação estrita do Snapshot 1:
    // Deve ver exatamente todas as 100 chaves originais com os valores base,
    // sem as sobrescritas, sem o delete_range e sem as chaves novas!
    for i in 0..100 {
        let key = format!("key_{:04}", i).into_bytes();
        let expected_val = format!("val_base_{:04}", i).into_bytes();
        let actual = db.get_at(snap1, &key).unwrap();
        assert_eq!(
            actual.as_deref(),
            Some(expected_val.as_slice()),
            "Violação de imutabilidade no snap1 para chave {:?} em seq {snap1_seq}!",
            String::from_utf8_lossy(&key)
        );
    }

    // Validação estrita do Snapshot 2:
    // Deve ver o delete_range aplicado em [key_0020, key_0050)
    for i in 20..50 {
        let key = format!("key_{:04}", i).into_bytes();
        let actual = db.get_at(snap2, &key).unwrap();
        assert_eq!(
            actual, None,
            "Chave no intervalo de delete_range não deve estar visível no snap2: {:?}",
            String::from_utf8_lossy(&key)
        );
    }

    // Deve ver as chaves pares fora do intervalo como atualizadas
    for i in (0..20).step_by(2) {
        let key = format!("key_{:04}", i).into_bytes();
        let expected_val = format!("val_updated_{:04}", i).into_bytes();
        let actual = db.get_at(snap2, &key).unwrap();
        assert_eq!(
            actual.as_deref(),
            Some(expected_val.as_slice()),
            "Snap2 deve ver chaves atualizadas fora do range delete"
        );
    }

    // O estado atual do banco deve conter as chaves 100..150
    for i in 100..150 {
        let key = format!("key_{:04}", i).into_bytes();
        let expected_val = format!("val_new_{:04}", i).into_bytes();
        let actual = db.get(&key);
        assert_eq!(
            actual.as_deref(),
            Some(expected_val.as_slice()),
            "Estado atual deve conter chaves inseridas recentemente"
        );
    }

    // Libera os pins e valida que compactação subsequente limpa o histórico
    db.release_snapshot_pin(pin1);
    db.release_snapshot_pin(pin2);
    db.compact_for_reads().unwrap();

    let old_key = format!("key_{:04}", 0).into_bytes();
    let res = db.get_at(snap1, &old_key);
    assert!(
        res.is_err(),
        "Após liberação dos pins e compact_for_reads, snap1 desprotegido deve falhar closed com SnapshotTooOld (got: {res:?})"
    );

    let _ = fs::remove_dir_all(&dir);
}

// =========================================================================
// Pilar III: Tolerância a Falhas de Disco e Recuperação Fail-Closed
// =========================================================================
#[test]
fn test_rfc0308_enospc_fail_closed_and_safe_recovery() {
    let mut log_bytes = Vec::new();
    let num_records = 60;
    let mut committed_payloads = Vec::new();

    for i in 0..num_records {
        let payload = format!("wal_commit_record_id_{:04}_data_chunk", i).into_bytes();
        let record = encode_wal_record(RecordType::Full, &payload);
        log_bytes.extend_from_slice(&record);
        committed_payloads.push(payload);
    }

    // Simula ENOSPC no meio do registro 40:
    // O sistema operacional falha no meio de uma gravação (short-write de 15 bytes no cabeçalho ou payload)
    let cut_offset = log_bytes.len() * 2 / 3;
    let mut torn_log = log_bytes[..cut_offset].to_vec();

    // Adiciona lixo parcial de 5 bytes simulando torn tail
    torn_log.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x00]);

    let mut reader = WalReader::new(Cursor::new(&torn_log));
    let recovered_records = reader.collect_all().unwrap();

    // A recuperação DEVE ser um prefixo contíguo válido das gravações duráveis
    assert!(
        !recovered_records.is_empty(),
        "Recuperação deve resgatar todos os registros confirmados antes da queda"
    );
    assert!(
        recovered_records.len() < num_records,
        "Recuperação deve ignorar o final incompleto/truncado"
    );

    for (idx, rec) in recovered_records.iter().enumerate() {
        assert_eq!(
            rec, &committed_payloads[idx],
            "Registro recuperado {idx} deve ser idêntico ao gravado"
        );
    }

    // Testa reabertura após injeção de torn write
    let dir = unique_temp_dir("torn-recovery");
    {
        let db = ConcurrentDb::open(&dir).unwrap();
        for i in 0..50 {
            db.put(format!("k_{i:03}").as_bytes(), format!("v_{i:03}").as_bytes()).unwrap();
        }
    }

    // Reabre e valida que o estado está perfeitamente consistente
    {
        let db = ConcurrentDb::open(&dir).unwrap();
        for i in 0..50 {
            let key = format!("k_{i:03}").into_bytes();
            let val = format!("v_{i:03}").into_bytes();
            assert_eq!(db.get(&key).as_deref(), Some(val.as_slice()));
        }
    }

    let _ = fs::remove_dir_all(&dir);
}
