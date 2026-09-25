//! Test Suite for RFC-0285: Cinco Pilares Fundamentais de Verificação do Motor Puro PedraDB.
//!
//! Validates the 5 formal pillars:
//! 1. Hot-Path Zero-Allocation Static Slab Arena
//! 2. Concrete MemTable-to-SST Flush Bisimulation & Strict Monotonicity
//! 3. Superblock Cryptographic Identity & Anti-Inode-Reuse Handshake
//! 4. Memory-Mapped Region Quiescence & Safe Anti-SIGBUS Unmap Barrier
//! 5. Block Cache Key Disambiguation & Anti-Incarnation Collision

#![forbid(unsafe_code)]

use std::sync::Arc;

use pedradb_core::block_cache_disambiguation_kernel::{
    CacheCollisionOracle, DisambiguatedCacheKey,
};
use pedradb_core::file_identity_superblock_kernel::{
    FileSuperblock, SuperblockError, SUPERBLOCK_SIZE,
};
use pedradb_core::hot_path_zero_alloc_kernel::StaticSlabArena;
use pedradb_core::memtable_flush_bisimulation_kernel::{
    ConcreteMemTable, FlushBisimulationViolation, MemTableEntry,
};
use pedradb_core::mmap_quiescence_barrier_kernel::{
    MmapQuiescenceCoordinator, MmapRegionControl, RegionState,
};

#[test]
fn test_pilar1_hot_path_zero_alloc_slab_arena() {
    let arena = StaticSlabArena::with_capacity(3);
    assert_eq!(arena.capacity(), 3);
    assert_eq!(arena.active_leases(), 0);

    // 1. Lease slot 1 and write payload
    let mut lease1 = arena.acquire_lease().expect("Lease 1 should succeed");
    assert_eq!(arena.active_leases(), 1);
    let payload = b"commit_wal_staged_record_zero_alloc";
    lease1.write_payload(payload, 100).expect("Write should succeed");

    let mut read_buf = [0u8; 128];
    let n = lease1.read_payload(&mut read_buf).expect("Read should succeed");
    assert_eq!(&read_buf[..n], payload);
    assert_eq!(lease1.seq_num().unwrap(), 100);

    // 2. Lease remaining slots until arena is full
    let _lease2 = arena.acquire_lease().expect("Lease 2 should succeed");
    let _lease3 = arena.acquire_lease().expect("Lease 3 should succeed");
    assert_eq!(arena.active_leases(), 3);

    // 3. 4th lease should return None (backpressure triggered without heap allocation)
    assert!(arena.acquire_lease().is_none());

    // 4. Dropping a lease frees the slot
    drop(lease1);
    assert_eq!(arena.active_leases(), 2);

    let _lease_reclaimed = arena.acquire_lease().expect("Slot should be reusable");
    assert_eq!(arena.active_leases(), 3);
}

#[test]
fn test_pilar2_memtable_flush_bisimulation_monotonicity() {
    let mut memtable = ConcreteMemTable::new();

    // Insert keys out of order with different sequence numbers
    memtable.put(b"zebra".to_vec(), Some(b"z1".to_vec()), 10);
    memtable.put(b"apple".to_vec(), Some(b"a1".to_vec()), 15);
    memtable.put(b"mango".to_vec(), Some(b"m1".to_vec()), 20);
    // Overwrite apple at higher seq
    memtable.put(b"apple".to_vec(), Some(b"a2_updated".to_vec()), 35);
    // Future write beyond freeze watermark
    memtable.put(b"banana".to_vec(), Some(b"b_future".to_vec()), 99);

    let freeze_seq = 50;
    let stream = memtable.generate_flush_stream(freeze_seq);

    // 1. Flush stream must emit strictly increasing sorted keys
    assert_eq!(stream.len(), 3);
    assert_eq!(stream[0].key, b"apple");
    assert_eq!(stream[0].value, Some(b"a2_updated".to_vec()));
    assert_eq!(stream[0].seq_num, 35);

    assert_eq!(stream[1].key, b"mango");
    assert_eq!(stream[1].value, Some(b"m1".to_vec()));
    assert_eq!(stream[1].seq_num, 20);

    assert_eq!(stream[2].key, b"zebra");
    assert_eq!(stream[2].value, Some(b"z1".to_vec()));
    assert_eq!(stream[2].seq_num, 10);

    // 2. Verification oracle proves bisimulation and completeness
    assert!(memtable.verify_flush_bisimulation(&stream, freeze_seq).is_ok());

    // 3. Test violation detection: out of order stream
    let corrupted_order = vec![
        stream[1].clone(),
        stream[0].clone(),
        stream[2].clone(),
    ];
    assert!(matches!(
        memtable.verify_flush_bisimulation(&corrupted_order, freeze_seq),
        Err(FlushBisimulationViolation::OutOrOrderKey { .. })
    ));

    // 4. Test violation detection: leaked future sequence (maintains sort order)
    let mut leaked_stream = stream.clone();
    leaked_stream.push(MemTableEntry {
        key: b"zzz_future".to_vec(),
        value: Some(b"z_future".to_vec()),
        seq_num: 99,
    });
    assert!(matches!(
        memtable.verify_flush_bisimulation(&leaked_stream, freeze_seq),
        Err(FlushBisimulationViolation::FutureSequenceLeaked { .. })
    ));
}

#[test]
fn test_pilar3_superblock_identity_and_anti_inode_reuse() {
    let uuid_a = [0x42u8; 16];
    let file_num = 105;
    let epoch = 2026_09_25;

    let sb = FileSuperblock::new(uuid_a, file_num, epoch);
    let encoded = sb.encode();
    assert_eq!(encoded.len(), SUPERBLOCK_SIZE);

    // 1. Valid decode and CRC check
    let decoded = FileSuperblock::decode(&encoded).expect("Decode should succeed");
    assert_eq!(decoded.file_uuid, uuid_a);
    assert_eq!(decoded.file_number, file_num);
    assert_eq!(decoded.creation_epoch, epoch);

    // 2. Identity handshake matches MANIFEST record
    assert!(decoded.verify_handshake(file_num, uuid_a).is_ok());

    // 3. File number mismatch fails
    assert_eq!(
        decoded.verify_handshake(106, uuid_a),
        Err(SuperblockError::FileNumberMismatch {
            expected: 106,
            actual: 105,
        })
    );

    // 4. UUID mismatch (inode reused for different table) fails
    let uuid_reused = [0x99u8; 16];
    assert_eq!(
        decoded.verify_handshake(file_num, uuid_reused),
        Err(SuperblockError::UuidMismatch)
    );

    // 5. Tampered CRC fails
    let mut corrupted = encoded;
    corrupted[10] ^= 0xFF; // Bit flip in UUID payload
    assert!(matches!(
        FileSuperblock::decode(&corrupted),
        Err(SuperblockError::CrcMismatch { .. })
    ));
}

#[test]
fn test_pilar4_mmap_quiescence_barrier_anti_sigbus() {
    let control = Arc::new(MmapRegionControl::new(55));
    assert_eq!(control.state(), RegionState::Active);

    // 1. Readers acquire leases
    let reader1 = MmapQuiescenceCoordinator::acquire_lease(&control).expect("Reader 1 lease");
    let reader2 = MmapQuiescenceCoordinator::acquire_lease(&control).expect("Reader 2 lease");
    assert_eq!(control.reader_count(), 2);
    assert_eq!(reader1.file_number(), 55);

    // 2. Compaction requests unmap
    let state = MmapQuiescenceCoordinator::request_unmap(&control);
    assert_eq!(state, RegionState::PendingUnmap);

    // 3. Unmap is NOT safe yet (active readers present)
    assert!(MmapQuiescenceCoordinator::verify_unmap_safety(&control).is_err());

    // 4. New readers are rejected during pending unmap
    assert!(MmapQuiescenceCoordinator::acquire_lease(&control).is_none());

    // 5. Readers release leases one by one
    drop(reader1);
    assert_eq!(control.reader_count(), 1);
    assert_eq!(control.state(), RegionState::PendingUnmap);
    assert!(MmapQuiescenceCoordinator::verify_unmap_safety(&control).is_err());

    drop(reader2);
    assert_eq!(control.reader_count(), 0);
    assert_eq!(control.state(), RegionState::QuiescedSafeToUnmap);

    // 6. Safe to physically unmap without SIGBUS hazard
    assert!(MmapQuiescenceCoordinator::verify_unmap_safety(&control).is_ok());
}

#[test]
fn test_pilar5_block_cache_key_disambiguation_collision_free() {
    let uuid_incarnation_1 = [0x11u8; 16];
    let uuid_incarnation_2 = [0x22u8; 16];
    let offset = 4096;
    let epoch = 1;

    let key1 = DisambiguatedCacheKey::new(uuid_incarnation_1, offset, epoch);
    let key2 = DisambiguatedCacheKey::new(uuid_incarnation_2, offset, epoch);

    // 1. Same offset and epoch, but different UUID -> Distinct keys
    assert_ne!(key1, key2);

    // 2. Byte encoding roundtrip
    let bytes1 = key1.to_bytes();
    let recovered1 = DisambiguatedCacheKey::from_bytes(&bytes1);
    assert_eq!(key1, recovered1);

    // 3. Oracle proves zero cross-incarnation collision
    assert!(CacheCollisionOracle::verify_disjoint_incarnations(&key1, &key2).is_ok());

    // 4. Equal keys with different epoch
    let key3 = DisambiguatedCacheKey::new(uuid_incarnation_1, offset, 2);
    assert_ne!(key1, key3);
}
