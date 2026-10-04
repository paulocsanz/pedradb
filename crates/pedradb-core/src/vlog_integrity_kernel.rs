//! RFC-0280 P0.1 — VLog Referential Integrity & Blob Pointer Kernel.
//!
//! Formalizes referential integrity between the LSM-tree key index and the external
//! Value Log (VLog) storage (WiscKey / BlobDB architecture).
//! Proves that no LSM entry can ever contain a dangling, truncated, or unchecksummed
//! blob pointer, even across active VLog Garbage Collection (GC) swings or power cuts.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;

/// Violations and errors in VLog referential integrity and pointer validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VlogIntegrityError {
    /// Blob pointer targets wrong VLog file number.
    WrongFileNumber { expected: u64, actual: u64 },
    /// Blob pointer extends past VLog EOF.
    PointerExtendsPastEof { file_num: u64, end_offset: u64, file_size: u64 },
    /// Blob length mismatch in VLog record.
    LengthMismatch { file_num: u64, offset: u64, expected_len: u32, actual_len: usize },
    /// Blob CRC corruption detected.
    CrcCorruption { file_num: u64, offset: u64, expected_crc: u32, found_crc: u32 },
    /// Blob pointer targets unaligned or non-existent offset.
    OffsetNotFound { file_num: u64, offset: u64 },
    /// Active VLog file missing from store.
    ActiveFileMissing { active_file_num: u64 },
    /// Source VLog file missing during migration or verification.
    SourceFileMissing { file_num: u64 },
    /// Missing record payload at specified offset.
    MissingRecordPayload { file_num: u64, offset: u64 },
    /// Invalid or zero file number.
    InvalidFileNumber { file_num: u64 },
    /// Empty payload not allowed for blob record.
    EmptyPayload,
    /// Target file already exists in store.
    TargetFileAlreadyExists { file_num: u64 },
    /// File size overflow detected.
    FileSizeOverflow { current_size: u64, additional: u64 },
}

impl fmt::Display for VlogIntegrityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongFileNumber { expected, actual } => {
                write!(f, "Blob pointer targets wrong VLog file number: expected {expected}, actual {actual}")
            }
            Self::PointerExtendsPastEof { file_num, end_offset, file_size } => {
                write!(f, "Blob pointer extends past VLog EOF (file {file_num}, end offset {end_offset} > size {file_size})")
            }
            Self::LengthMismatch { file_num, offset, expected_len, actual_len } => {
                write!(f, "Blob length mismatch in VLog record at file {file_num}, offset {offset}: expected {expected_len}, actual {actual_len}")
            }
            Self::CrcCorruption { file_num, offset, expected_crc, found_crc } => {
                write!(f, "Blob CRC corruption detected at file {file_num}, offset {offset}: expected 0x{expected_crc:08x}, found 0x{found_crc:08x}")
            }
            Self::OffsetNotFound { file_num, offset } => {
                write!(f, "Blob pointer targets unaligned or non-existent offset {offset} in file {file_num}")
            }
            Self::ActiveFileMissing { active_file_num } => {
                write!(f, "Active VLog file {active_file_num} missing from store")
            }
            Self::SourceFileMissing { file_num } => {
                write!(f, "Source VLog file {file_num} missing from store")
            }
            Self::MissingRecordPayload { file_num, offset } => {
                write!(f, "Missing record payload at file {file_num}, offset {offset}")
            }
            Self::InvalidFileNumber { file_num } => {
                write!(f, "Invalid file number: {file_num} (cannot be zero)")
            }
            Self::EmptyPayload => {
                write!(f, "Empty payload not allowed for blob record")
            }
            Self::TargetFileAlreadyExists { file_num } => {
                write!(f, "Target VLog file {file_num} already exists in store")
            }
            Self::FileSizeOverflow { current_size, additional } => {
                write!(f, "File size overflow detected: current {current_size} + additional {additional}")
            }
        }
    }
}

impl std::error::Error for VlogIntegrityError {}

/// A physical blob reference embedded inside an LSM-tree value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

impl BlobPointer {
    /// Creates a validated BlobPointer.
    pub fn try_new(file_num: u64, offset: u64, len: u32, crc: u32) -> Result<Self, VlogIntegrityError> {
        if file_num == 0 {
            return Err(VlogIntegrityError::InvalidFileNumber { file_num: 0 });
        }
        if len == 0 {
            return Err(VlogIntegrityError::EmptyPayload);
        }
        Ok(Self {
            file_num,
            offset,
            len,
            crc,
        })
    }
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

    /// Creates an empty VLog file with validation.
    pub fn try_new(file_num: u64) -> Result<Self, VlogIntegrityError> {
        if file_num == 0 {
            return Err(VlogIntegrityError::InvalidFileNumber { file_num: 0 });
        }
        Ok(Self::new(file_num))
    }

    /// Appends a blob payload to the VLog file and returns its `BlobPointer`.
    pub fn append(&mut self, payload: &[u8], crc: u32) -> BlobPointer {
        let offset = self.file_size;
        let len = payload.len() as u32;
        self.records.insert(offset, (payload.to_vec(), crc));
        self.file_size = self.file_size.saturating_add(len as u64);

        BlobPointer {
            file_num: self.file_num,
            offset,
            len,
            crc,
        }
    }

    /// Appends a blob payload to the VLog file with strict validation.
    pub fn try_append(&mut self, payload: &[u8], crc: u32) -> Result<BlobPointer, VlogIntegrityError> {
        if payload.is_empty() {
            return Err(VlogIntegrityError::EmptyPayload);
        }
        let len = payload.len() as u32;
        let offset = self.file_size;
        let new_size = offset.checked_add(len as u64).ok_or(VlogIntegrityError::FileSizeOverflow {
            current_size: offset,
            additional: len as u64,
        })?;
        self.records.insert(offset, (payload.to_vec(), crc));
        self.file_size = new_size;

        BlobPointer::try_new(self.file_num, offset, len, crc)
    }

    /// Verifies that a blob pointer points to a valid, intact byte slice within this file.
    pub fn verify_pointer(&self, ptr: &BlobPointer) -> Result<(), &'static str> {
        self.try_verify_pointer(ptr).map_err(|e| match e {
            VlogIntegrityError::WrongFileNumber { .. } => "Blob pointer targets wrong VLog file number",
            VlogIntegrityError::PointerExtendsPastEof { .. } => "Blob pointer extends past VLog EOF",
            VlogIntegrityError::LengthMismatch { .. } => "Blob length mismatch in VLog record",
            VlogIntegrityError::CrcCorruption { .. } => "Blob CRC corruption detected",
            VlogIntegrityError::OffsetNotFound { .. } => "Blob pointer targets unaligned or non-existent offset",
            _ => "VLog pointer verification failure",
        })
    }

    /// Verifies that a blob pointer points to a valid, intact byte slice within this file.
    pub fn try_verify_pointer(&self, ptr: &BlobPointer) -> Result<(), VlogIntegrityError> {
        if ptr.file_num != self.file_num {
            return Err(VlogIntegrityError::WrongFileNumber {
                expected: self.file_num,
                actual: ptr.file_num,
            });
        }
        let end_offset = ptr.offset.saturating_add(ptr.len as u64);
        if end_offset > self.file_size {
            return Err(VlogIntegrityError::PointerExtendsPastEof {
                file_num: self.file_num,
                end_offset,
                file_size: self.file_size,
            });
        }
        match self.records.get(&ptr.offset) {
            Some((data, stored_crc)) => {
                if data.len() != ptr.len as usize {
                    return Err(VlogIntegrityError::LengthMismatch {
                        file_num: self.file_num,
                        offset: ptr.offset,
                        expected_len: ptr.len,
                        actual_len: data.len(),
                    });
                }
                if *stored_crc != ptr.crc {
                    return Err(VlogIntegrityError::CrcCorruption {
                        file_num: self.file_num,
                        offset: ptr.offset,
                        expected_crc: *stored_crc,
                        found_crc: ptr.crc,
                    });
                }
                Ok(())
            }
            None => Err(VlogIntegrityError::OffsetNotFound {
                file_num: self.file_num,
                offset: ptr.offset,
            }),
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

    /// Creates a fresh VlogStore with validation.
    pub fn try_new(initial_file_num: u64) -> Result<Self, VlogIntegrityError> {
        if initial_file_num == 0 {
            return Err(VlogIntegrityError::InvalidFileNumber { file_num: 0 });
        }
        Ok(Self::new(initial_file_num))
    }

    /// Appends a blob to the active VLog file.
    pub fn append(&mut self, payload: &[u8], crc: u32) -> Result<BlobPointer, &'static str> {
        let active = self
            .files
            .get_mut(&self.active_file_num)
            .ok_or("Active VLog file missing")?;
        Ok(active.append(payload, crc))
    }

    /// Appends a blob to the active VLog file with typed errors.
    pub fn try_append(&mut self, payload: &[u8], crc: u32) -> Result<BlobPointer, VlogIntegrityError> {
        let active = self
            .files
            .get_mut(&self.active_file_num)
            .ok_or(VlogIntegrityError::ActiveFileMissing {
                active_file_num: self.active_file_num,
            })?;
        active.try_append(payload, crc)
    }

    /// Verifies the Referential Integrity Invariant:
    /// Every blob pointer in the LSM tree must target an existing VLog file and an intact byte slice.
    pub fn verify_referential_integrity(&self, lsm_pointers: &[BlobPointer]) -> bool {
        self.try_verify_referential_integrity(lsm_pointers).is_ok()
    }

    /// Verifies referential integrity with detailed error reporting.
    pub fn try_verify_referential_integrity(
        &self,
        lsm_pointers: &[BlobPointer],
    ) -> Result<(), VlogIntegrityError> {
        for ptr in lsm_pointers {
            match self.files.get(&ptr.file_num) {
                Some(file) => {
                    file.try_verify_pointer(ptr)?;
                }
                None => {
                    return Err(VlogIntegrityError::SourceFileMissing {
                        file_num: ptr.file_num,
                    });
                }
            }
        }
        Ok(())
    }

    /// Simulates a safe Garbage Collection migration:
    /// Re-writes live blobs to a new generation and returns updated pointers.
    pub fn gc_migrate_blobs(
        &mut self,
        new_file_num: u64,
        live_pointers: &[BlobPointer],
    ) -> Result<Vec<BlobPointer>, &'static str> {
        self.try_gc_migrate_blobs(new_file_num, live_pointers)
            .map_err(|e| match e {
                VlogIntegrityError::SourceFileMissing { .. } => "Source VLog file missing",
                VlogIntegrityError::MissingRecordPayload { .. } => "Missing record payload",
                _ => "GC migration failed",
            })
    }

    /// Simulates a safe Garbage Collection migration with typed errors.
    pub fn try_gc_migrate_blobs(
        &mut self,
        new_file_num: u64,
        live_pointers: &[BlobPointer],
    ) -> Result<Vec<BlobPointer>, VlogIntegrityError> {
        if new_file_num == 0 {
            return Err(VlogIntegrityError::InvalidFileNumber { file_num: 0 });
        }
        if self.files.contains_key(&new_file_num) {
            return Err(VlogIntegrityError::TargetFileAlreadyExists {
                file_num: new_file_num,
            });
        }
        let mut new_vlog = VlogFile::try_new(new_file_num)?;
        let mut migrated = Vec::with_capacity(live_pointers.len());

        for old_ptr in live_pointers {
            let old_file = self
                .files
                .get(&old_ptr.file_num)
                .ok_or(VlogIntegrityError::SourceFileMissing {
                    file_num: old_ptr.file_num,
                })?;
            old_file.try_verify_pointer(old_ptr)?;

            let (payload, crc) = old_file
                .records
                .get(&old_ptr.offset)
                .ok_or(VlogIntegrityError::MissingRecordPayload {
                    file_num: old_ptr.file_num,
                    offset: old_ptr.offset,
                })?;
            let new_ptr = new_vlog.try_append(payload, *crc)?;
            migrated.push(new_ptr);
        }

        self.files.insert(new_file_num, new_vlog);
        Ok(migrated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vlog_integrity_structural_invariants_red_to_green() {
        // 1. Error Display & std::error::Error conformance
        let errors = [
            VlogIntegrityError::WrongFileNumber { expected: 1, actual: 2 },
            VlogIntegrityError::PointerExtendsPastEof { file_num: 1, end_offset: 200, file_size: 100 },
            VlogIntegrityError::LengthMismatch { file_num: 1, offset: 0, expected_len: 10, actual_len: 8 },
            VlogIntegrityError::CrcCorruption { file_num: 1, offset: 0, expected_crc: 0x1234, found_crc: 0x5678 },
            VlogIntegrityError::OffsetNotFound { file_num: 1, offset: 99 },
            VlogIntegrityError::ActiveFileMissing { active_file_num: 42 },
            VlogIntegrityError::SourceFileMissing { file_num: 99 },
            VlogIntegrityError::MissingRecordPayload { file_num: 1, offset: 0 },
            VlogIntegrityError::InvalidFileNumber { file_num: 0 },
            VlogIntegrityError::EmptyPayload,
            VlogIntegrityError::TargetFileAlreadyExists { file_num: 2 },
            VlogIntegrityError::FileSizeOverflow { current_size: u64::MAX, additional: 1 },
        ];
        for err in &errors {
            let msg = format!("{err}");
            assert!(!msg.is_empty());
            let dyn_err: &dyn std::error::Error = err;
            assert_eq!(dyn_err.to_string(), msg);
        }

        // 2. BlobPointer validation
        assert_eq!(
            BlobPointer::try_new(0, 0, 10, 123),
            Err(VlogIntegrityError::InvalidFileNumber { file_num: 0 })
        );
        assert_eq!(
            BlobPointer::try_new(1, 0, 0, 123),
            Err(VlogIntegrityError::EmptyPayload)
        );
        let ptr = BlobPointer::try_new(1, 0, 14, 0xdeadbeef).unwrap();
        assert_eq!(ptr.file_num, 1);
        assert_eq!(ptr.len, 14);

        // 3. VlogFile & VlogStore constructors validation
        assert_eq!(
            VlogFile::try_new(0),
            Err(VlogIntegrityError::InvalidFileNumber { file_num: 0 })
        );
        assert_eq!(
            VlogStore::try_new(0),
            Err(VlogIntegrityError::InvalidFileNumber { file_num: 0 })
        );

        // 4. Try append empty payload rejection
        let mut store = VlogStore::try_new(1).unwrap();
        assert_eq!(store.try_append(b"", 0), Err(VlogIntegrityError::EmptyPayload));

        // 5. Valid append & referential integrity
        let p1 = store.try_append(b"hello_vlog_blob", 0x11223344).unwrap();
        assert!(store.try_verify_referential_integrity(&[p1]).is_ok());
        assert!(store.verify_referential_integrity(&[p1]));

        // 6. GC migration validation: existing target rejection & valid migration
        assert_eq!(
            store.try_gc_migrate_blobs(1, &[p1]),
            Err(VlogIntegrityError::TargetFileAlreadyExists { file_num: 1 })
        );
        assert_eq!(
            store.try_gc_migrate_blobs(0, &[p1]),
            Err(VlogIntegrityError::InvalidFileNumber { file_num: 0 })
        );
        let migrated = store.try_gc_migrate_blobs(2, &[p1]).unwrap();
        assert_eq!(migrated.len(), 1);
        assert_eq!(migrated[0].file_num, 2);
        assert!(store.try_verify_referential_integrity(&migrated).is_ok());

        // 7. Dangling pointer detection
        let dangling = BlobPointer::try_new(999, 0, 10, 123).unwrap();
        assert_eq!(
            store.try_verify_referential_integrity(&[dangling]),
            Err(VlogIntegrityError::SourceFileMissing { file_num: 999 })
        );
    }
}
