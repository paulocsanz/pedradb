//! RFC-0310: LSM Relational Metamorphic Algebra Oracle (10x Phase IV)
//!
//! Validates 5 fundamental algebraic invariants across memtable, L0 flushes,
//! and multi-level compactions without requiring an external reference engine.

use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use pedradb_core::{CompactOptions, ConcurrentDb, OpenOptions};

fn unique_temp_dir(tag: &str) -> std::path::PathBuf {
    static CTR: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let c = CTR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pedra-rfc0310-{tag}-{nanos}-{c}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Identidade E1: Idempotência de Sobrescrita
/// Para qualquer chave K, sobrescrever K com V2 após V1 sempre expõe V2,
/// tanto na memtable quanto após flush para L0 e após compactação.
#[test]
fn test_metamorphic_e1_overwriting_idempotence() {
    let dir = unique_temp_dir("e1-idempotence");
    let mut opts = OpenOptions::default();
    opts.sync = false;
    let db = ConcurrentDb::open_with(&dir, opts).unwrap();

    let key = b"metamorphic_key_e1";
    db.put(key, b"version_1").unwrap();
    db.put(key, b"version_2").unwrap();

    // 1. Em memória
    assert_eq!(db.get(key).as_deref(), Some(&b"version_2"[..]));

    // 2. Após Flush para L0
    db.flush().unwrap();
    assert_eq!(db.get(key).as_deref(), Some(&b"version_2"[..]));

    // 3. Após Sobrescrita adicional e Compactação
    db.put(key, b"version_3").unwrap();
    db.flush().unwrap();
    db.compact_with(CompactOptions::default()).unwrap();
    assert_eq!(db.get(key).as_deref(), Some(&b"version_3"[..]));

    let _ = fs::remove_dir_all(&dir);
}

/// Identidade E2: Aniquilação Canônica por Tombstone
/// Para qualquer chave K, Put(K, V) seguido de Delete(K) resulta estritamente em None,
/// sobrevivendo a flushes e compactações.
#[test]
fn test_metamorphic_e2_tombstone_annihilation() {
    let dir = unique_temp_dir("e2-tombstone");
    let mut opts = OpenOptions::default();
    opts.sync = false;
    let db = ConcurrentDb::open_with(&dir, opts).unwrap();

    for i in 0..20 {
        let k = format!("tombstone_k_{:03}", i).into_bytes();
        let v = format!("v_{:03}", i).into_bytes();
        db.put(&k, &v).unwrap();
        db.delete(&k).unwrap();
        assert!(db.get(&k).is_none());
    }

    db.flush().unwrap();
    for i in 0..20 {
        let k = format!("tombstone_k_{:03}", i).into_bytes();
        assert!(db.get(&k).is_none());
    }

    db.compact_with(CompactOptions::default()).unwrap();
    for i in 0..20 {
        let k = format!("tombstone_k_{:03}", i).into_bytes();
        assert!(db.get(&k).is_none());
    }

    let _ = fs::remove_dir_all(&dir);
}

/// Identidade E3: Comutatividade de Chaves Disjuntas Concorrentes
/// Múltiplas threads escrevendo conjuntos disjuntos de chaves convergem
/// para o mesmo estado exato independentemente da ordem de intercalação.
#[test]
fn test_metamorphic_e3_disjoint_key_commutativity() {
    let dir = unique_temp_dir("e3-disjoint");
    let mut opts = OpenOptions::default();
    opts.sync = false;
    let db = Arc::new(ConcurrentDb::open_with(&dir, opts).unwrap());

    let num_threads = 4;
    let keys_per_thread = 25;
    let mut handles = Vec::new();

    for tid in 0..num_threads {
        let db_clone = Arc::clone(&db);
        handles.push(thread::spawn(move || {
            for i in 0..keys_per_thread {
                let k = format!("thread_{}_k_{:03}", tid, i).into_bytes();
                let v = format!("val_{}_{:03}", tid, i).into_bytes();
                db_clone.put(&k, &v).unwrap();
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    // Todos os valores devem estar íntegros e observáveis
    for tid in 0..num_threads {
        for i in 0..keys_per_thread {
            let k = format!("thread_{}_k_{:03}", tid, i).into_bytes();
            let expected_v = format!("val_{}_{:03}", tid, i).into_bytes();
            assert_eq!(
                db.get(&k).as_deref(),
                Some(expected_v.as_slice()),
                "Falha na comutatividade disjunta para chave {:?}",
                String::from_utf8_lossy(&k)
            );
        }
    }

    let _ = fs::remove_dir_all(&dir);
}

/// Identidade E4: Invariância de Snapshot sob Compactação Agressiva
/// Um snapshot pinning capturado antes de sobrescritas e compactações com GC
/// preserva 100% de seus dados originais.
#[test]
fn test_metamorphic_e4_snapshot_preservation_under_compaction() {
    let dir = unique_temp_dir("e4-snapshot");
    let mut opts = OpenOptions::default();
    opts.sync = false;
    let db = ConcurrentDb::open_with(&dir, opts).unwrap();

    // 1. Grava 50 chaves originais
    for i in 0..50 {
        let k = format!("snap_k_{:03}", i).into_bytes();
        let v = format!("orig_val_{:03}", i).into_bytes();
        db.put(&k, &v).unwrap();
    }
    db.flush().unwrap();

    // 2. Captura Snapshot Pinning
    let pin = db.pin_snapshot();
    let snap = pin.snapshot();

    // 3. Modifica severamente o banco (sobrescreve todas as chaves e deleta metade)
    for i in 0..50 {
        let k = format!("snap_k_{:03}", i).into_bytes();
        let v = format!("overwritten_val_{:03}", i).into_bytes();
        db.put(&k, &v).unwrap();
    }
    for i in 0..25 {
        let k = format!("snap_k_{:03}", i).into_bytes();
        db.delete(&k).unwrap();
    }
    db.flush().unwrap();
    db.compact_with(CompactOptions::latest_only()).unwrap();

    // 4. Valida que o Snapshot lê estritamente os valores originais intactos
    for i in 0..50 {
        let k = format!("snap_k_{:03}", i).into_bytes();
        let expected_v = format!("orig_val_{:03}", i).into_bytes();
        assert_eq!(
            db.get_at(snap, &k).unwrap().as_deref(),
            Some(expected_v.as_slice()),
            "Snapshot vazou mutação pós-pinning para chave {:?}",
            String::from_utf8_lossy(&k)
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

/// Identidade E5: Mascaramento e Re-ativação por Range Tombstones
/// Range tombstone oculta todo intervalo até nova escrita com sequência superior.
#[test]
fn test_metamorphic_e5_range_tombstone_shadowing_and_reactivation() {
    let dir = unique_temp_dir("e5-range-tombstone");
    let mut opts = OpenOptions::default();
    opts.sync = false;
    let db = ConcurrentDb::open_with(&dir, opts).unwrap();

    // Grava chaves k_00 .. k_10
    for i in 0..10 {
        let k = format!("k_{:02}", i).into_bytes();
        db.put(&k, b"base").unwrap();
    }
    db.flush().unwrap();

    let pin_base = db.pin_snapshot();
    let snap_base = pin_base.snapshot();

    // Deleta range [k_03, k_07)
    db.delete_range(b"k_03", b"k_07").unwrap();
    db.flush().unwrap();

    let pin_del = db.pin_snapshot();
    let snap_del = pin_del.snapshot();

    // Reinsere k_05 com novo valor
    db.put(b"k_05", b"resurrected").unwrap();
    db.flush().unwrap();

    let pin_res = db.pin_snapshot();
    let snap_res = pin_res.snapshot();

    // Invariante 1: snap_base vê todas as chaves com "base"
    for i in 0..10 {
        let k = format!("k_{:02}", i).into_bytes();
        assert_eq!(db.get_at(snap_base, &k).unwrap().as_deref(), Some(&b"base"[..]));
    }

    // Invariante 2: snap_del tem [k_03, k_07) deletadas, outras presentes
    for i in 0..10 {
        let k = format!("k_{:02}", i).into_bytes();
        if (3..7).contains(&i) {
            assert_eq!(db.get_at(snap_del, &k).unwrap(), None);
        } else {
            assert_eq!(db.get_at(snap_del, &k).unwrap().as_deref(), Some(&b"base"[..]));
        }
    }

    // Invariante 3: snap_res vê k_05 ressuscitado, k_03, k_04, k_06 deletados
    assert_eq!(db.get_at(snap_res, b"k_05").unwrap().as_deref(), Some(&b"resurrected"[..]));
    assert_eq!(db.get_at(snap_res, b"k_03").unwrap(), None);
    assert_eq!(db.get_at(snap_res, b"k_04").unwrap(), None);
    assert_eq!(db.get_at(snap_res, b"k_06").unwrap(), None);

    let _ = fs::remove_dir_all(&dir);
}
