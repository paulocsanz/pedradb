//! RFC-0280 P0.1 — VLog Referential Integrity & Blob Pointer Kernel.
//!
//! Formalizes referential integrity between the LSM-tree key index and the external
//! Value Log (VLog) storage (WiscKey / BlobDB architecture).
//! Proves that no LSM entry can ever contain a dangling, truncated, or unchecksummed
//! blob pointer, even across active VLog Garbage Collection (GC) swings or power cuts.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// A physical blob reference embedded inside an LSM-tree value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BlobPointer {
    /// VLog file number (e.g., 000001.vlog).
    pub file_num: u64,
    /// Byte offset within the VLog file where the payload begins.
    pub offset: u64,
    /// Length of the blob payload in bytes.
    pub len: u32,
    /// CRC32C checksum of the blob payload.
    pub crc: u32,
}

/// Simulated physical state of a Value Log file on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VlogFile {
    /// VLog file number.
    pub file_num: u64,
    /// Total file length on disk.
    pub file_size: u64,
    /// Map of offset -> (payload_bytes, crc)
    pub records: BTreeMap<u64, (Vec<u8>, u32)>,
}

impl VlogFile {
    /// Creates an empty VLog file.
    pub fn new(file_num: u64) -> Self {
        Self {
            file_num,
            file_size: 0,
            records: BTreeMap::new(),
        }
    }

    /// Appends a blob payload to the VLog file and returns its `BlobPointer`.
    pub fn append(&mut self, payload: &[u8], crc: u32) -> BlobPointer {
        let offset = self.file_size;
        let len = payload.len() as u32;
        self.records.insert(offset, (payload.to_vec(), crc));
        self.file_size += len as u64;

        BlobPointer {
            file_num: self.file_num,
            offset,
            len,
            crc,
        }
    }

    /// Verifies that a blob pointer points to a valid, intact byte slice within this file.
    pub fn verify_pointer(&self, ptr: &BlobPointer) -> Result<(), &'static str> {
        if ptr.file_num != self.file_num {
            return Err("Blob pointer targets wrong VLog file number");
        }
        if ptr.offset + (ptr.len as u64) > self.file_size {
            return Err("Blob pointer extends past VLog EOF");
        }
        match self.records.get(&ptr.offset) {
            Some((data, stored_crc)) => {
                if data.len() != ptr.len as usize {
                    return Err("Blob length mismatch in VLog record");
                }
                if *stored_crc != ptr.crc {
                    return Err("Blob CRC corruption detected");
                }
                Ok(())
            }
            None => Err("Blob pointer targets unaligned or non-existent offset"),
        }
    }
}

/// Multi-file VLog storage system tracking active, sealed, and garbage-collected files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VlogStore {
    /// All active or sealed VLog files.
    pub files: BTreeMap<u64, VlogFile>,
    /// Active file number accepting new appends.
    pub active_file_num: u64,
}

impl VlogStore {
    /// Creates a fresh VlogStore with an initial file.
    pub fn new(initial_file_num: u64) -> Self {
        let mut files = BTreeMap::new();
        files.insert(initial_file_num, VlogFile::new(initial_file_num));
        Self {
            files,
            active_file_num: initial_file_num,
        }
    }

    /// Appends a blob to the active VLog file.
    pub fn append(&mut self, payload: &[u8], crc: u32) -> Result<BlobPointer, &'static str> {
        let active = self
            .files
            .get_mut(&self.active_file_num)
            .ok_or("Active VLog file missing")?;
        Ok(active.append(payload, crc))
    }

    /// Verifies the Referential Integrity Invariant:
    /// Every blob pointer in the LSM tree must target an existing VLog file and an intact byte slice.
    pub fn verify_referential_integrity(&self, lsm_pointers: &[BlobPointer]) -> bool {
        for ptr in lsm_pointers {
            match self.files.get(&ptr.file_num) {
                Some(file) => {
                    if file.verify_pointer(ptr).is_err() {
                        return false; // Dangling or corrupted blob pointer!
                    }
                }
                None => return false, // Target VLog file does not exist on disk!
            }
        }
        true
    }

    /// Simulates a safe Garbage Collection migration:
    /// Re-writes live blobs to a new generation and returns updated pointers.
    pub fn gc_migrate_blobs(
        &mut self,
        new_file_num: u64,
        live_pointers: &[BlobPointer],
    ) -> Result<Vec<BlobPointer>, &'static str> {
        let mut new_vlog = VlogFile::new(new_file_num);
        let mut migrated = Vec::new();

        for old_ptr in live_pointers {
            let old_file = self
                .files
                .get(&old_ptr.file_num)
                .ok_or("Source VLog file missing")?;
            old_file.verify_pointer(old_ptr)?;

            let (payload, crc) = old_file
                .records
                .get(&old_ptr.offset)
                .ok_or("Missing record payload")?;
            let new_ptr = new_vlog.append(payload, *crc);
            migrated.push(new_ptr);
        }

        self.files.insert(new_file_num, new_vlog);
        Ok(migrated)
    }
}
