//! RFC-0291 Fronteira 5: Refinamento Causal e Eliminação de Dangling Pointers no GC do vLog / Blob Storage.
//!
//! Implementa formalmente o protocolo Two-Phase Safe Purge para o armazenamento
//! desacoplado de valores (Value Log / BlobDB), garantindo ausência de referências
//! pendentes (dangling pointers) e preservação estrita de integridade causal sob crashes.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};

/// Ponteiro imutável para um registro de valor no Value Log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlobHandle {
    pub file_number: u64,
    pub offset: u64,
    pub size: u32,
    pub value_crc: u32,
}

/// Estado do arquivo de vLog no catálogo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VlogFileMeta {
    pub file_number: u64,
    pub total_size_bytes: u64,
    pub active_blobs_count: usize,
    pub is_marked_for_gc: bool,
    pub is_physically_deleted: bool,
}

/// Ação de transição no protocolo de Two-Phase Safe Purge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobGcPhase {
    /// Fase 1: Blobs ativos reescritos em novo arquivo de vLog.
    BlobsRelocated {
        source_vlog: u64,
        target_vlog: u64,
        relocations: HashMap<BlobHandle, BlobHandle>,
    },
    /// Fase 2: LSM Version atualizada no Manifest com novos ponteiros.
    LsmPointersCommitted {
        source_vlog: u64,
    },
    /// Fase 3: Arquivo antigo descartado com zero referências remanescentes.
    VlogPhysicallyPurged {
        purged_file: u64,
    },
}

/// Violações de integridade causal e segurança de descarte no vLog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobStorageViolation {
    DanglingPointerDetected {
        key: Vec<u8>,
        handle: BlobHandle,
        reason: &'static str,
    },
    PrematureVlogDeletion {
        vlog_file: u64,
        active_references: usize,
    },
    RelocationMismatch {
        old_handle: BlobHandle,
        key: Vec<u8>,
    },
    InvalidTargetVlog {
        target_vlog: u64,
        reason: &'static str,
    },
    AlreadyPurged {
        vlog_file: u64,
    },
    Phase1NotExecuted {
        source_vlog: u64,
    },
    InvalidRelocationHandle {
        expected_file: u64,
        actual_file: u64,
    },
    RelocationIntegrityMismatch {
        old_handle: BlobHandle,
        new_handle: BlobHandle,
        reason: &'static str,
    },
}

impl std::fmt::Display for BlobStorageViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DanglingPointerDetected { key, handle, reason } => {
                write!(f, "Dangling pointer detected for key {key:?} (handle {handle:?}): {reason}")
            }
            Self::PrematureVlogDeletion { vlog_file, active_references } => {
                write!(f, "Premature deletion of vlog {vlog_file} with {active_references} active references")
            }
            Self::RelocationMismatch { old_handle, key } => {
                write!(f, "Relocation mismatch for key {key:?} with old handle {old_handle:?}")
            }
            Self::InvalidTargetVlog { target_vlog, reason } => {
                write!(f, "Invalid target vlog {target_vlog}: {reason}")
            }
            Self::AlreadyPurged { vlog_file } => {
                write!(f, "Vlog file {vlog_file} was already purged")
            }
            Self::Phase1NotExecuted { source_vlog } => {
                write!(f, "Phase 1 not executed or pending for vlog {source_vlog}")
            }
            Self::InvalidRelocationHandle { expected_file, actual_file } => {
                write!(f, "Invalid relocation handle: expected file {expected_file}, got {actual_file}")
            }
            Self::RelocationIntegrityMismatch { old_handle, new_handle, reason } => {
                write!(f, "Relocation integrity mismatch from {old_handle:?} to {new_handle:?}: {reason}")
            }
        }
    }
}

impl std::error::Error for BlobStorageViolation {}

/// Gerenciador de refinamento e ciclo de vida de GC do vLog.
pub struct VlogBlobGcCoordinator {
    vlog_files: HashMap<u64, VlogFileMeta>,
    lsm_index: HashMap<Vec<u8>, BlobHandle>,
    pending_relocations: HashMap<u64, HashMap<BlobHandle, BlobHandle>>,
    committed_gc_vlogs: HashSet<u64>,
}

impl Default for VlogBlobGcCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl VlogBlobGcCoordinator {
    pub fn new() -> Self {
        Self {
            vlog_files: HashMap::new(),
            lsm_index: HashMap::new(),
            pending_relocations: HashMap::new(),
            committed_gc_vlogs: HashSet::new(),
        }
    }

    /// Registra um novo arquivo de vLog.
    pub fn register_vlog(&mut self, file_number: u64, size: u64) {
        self.vlog_files.insert(
            file_number,
            VlogFileMeta {
                file_number,
                total_size_bytes: size,
                active_blobs_count: 0,
                is_marked_for_gc: false,
                is_physically_deleted: false,
            },
        );
    }

    /// Retorna os metadados do arquivo de vLog, se existir.
    pub fn get_vlog_meta(&self, file_number: u64) -> Option<&VlogFileMeta> {
        self.vlog_files.get(&file_number)
    }

    /// Insere ou atualiza um ponteiro no índice da LSM.
    pub fn put_pointer(&mut self, key: Vec<u8>, handle: BlobHandle) {
        if let Some(meta) = self.vlog_files.get_mut(&handle.file_number) {
            meta.active_blobs_count += 1;
        }
        if let Some(old) = self.lsm_index.insert(key, handle) {
            if let Some(meta) = self.vlog_files.get_mut(&old.file_number) {
                if meta.active_blobs_count > 0 {
                    meta.active_blobs_count -= 1;
                }
            }
        }
    }

    /// Valida e insere um ponteiro no índice da LSM, prevenindo ponteiros corrompidos ou pendentes.
    pub fn try_put_pointer(&mut self, key: Vec<u8>, handle: BlobHandle) -> Result<(), BlobStorageViolation> {
        if handle.size == 0 {
            return Err(BlobStorageViolation::DanglingPointerDetected {
                key,
                handle,
                reason: "Blob handle size cannot be zero",
            });
        }
        let meta = self.vlog_files.get(&handle.file_number).ok_or(
            BlobStorageViolation::DanglingPointerDetected {
                key: key.clone(),
                handle,
                reason: "Referenced vlog file does not exist",
            },
        )?;
        if meta.is_physically_deleted {
            return Err(BlobStorageViolation::DanglingPointerDetected {
                key: key.clone(),
                handle,
                reason: "Referenced vlog file was physically deleted",
            });
        }
        if meta.is_marked_for_gc {
            return Err(BlobStorageViolation::DanglingPointerDetected {
                key: key.clone(),
                handle,
                reason: "Referenced vlog file is marked for GC",
            });
        }
        if meta.total_size_bytes > 0 && handle.offset.saturating_add(handle.size as u64) > meta.total_size_bytes {
            return Err(BlobStorageViolation::DanglingPointerDetected {
                key,
                handle,
                reason: "Blob handle offset plus size exceeds vlog file size",
            });
        }
        self.put_pointer(key, handle);
        Ok(())
    }

    /// Executa a Fase 1 do GC: reescreve blobs e gera mapa de relocalização.
    pub fn execute_phase1_rewrite(
        &mut self,
        source_vlog: u64,
        target_vlog: u64,
        relocations: HashMap<BlobHandle, BlobHandle>,
    ) -> Result<BlobGcPhase, BlobStorageViolation> {
        if source_vlog == target_vlog {
            return Err(BlobStorageViolation::InvalidTargetVlog {
                target_vlog,
                reason: "Target vlog cannot be the same as source vlog",
            });
        }

        let target_meta = self.vlog_files.get(&target_vlog).ok_or(
            BlobStorageViolation::InvalidTargetVlog {
                target_vlog,
                reason: "Target vlog does not exist in catalog",
            },
        )?;

        if target_meta.is_marked_for_gc || target_meta.is_physically_deleted {
            return Err(BlobStorageViolation::InvalidTargetVlog {
                target_vlog,
                reason: "Target vlog is marked for GC or physically deleted",
            });
        }

        let target_total_size = target_meta.total_size_bytes;

        let source_meta = self.vlog_files.get_mut(&source_vlog).ok_or(
            BlobStorageViolation::DanglingPointerDetected {
                key: vec![],
                handle: BlobHandle {
                    file_number: source_vlog,
                    offset: 0,
                    size: 0,
                    value_crc: 0,
                },
                reason: "Source vlog does not exist in catalog",
            },
        )?;

        if source_meta.is_physically_deleted {
            return Err(BlobStorageViolation::AlreadyPurged {
                vlog_file: source_vlog,
            });
        }

        if source_meta.is_marked_for_gc || self.pending_relocations.contains_key(&source_vlog) {
            return Err(BlobStorageViolation::InvalidTargetVlog {
                target_vlog: source_vlog,
                reason: "Source vlog is already marked for GC",
            });
        }

        // Valida que todos os handles no mapa de relocalização pertencem a source e target e preservam integridade
        for (old_h, new_h) in &relocations {
            if old_h.file_number != source_vlog {
                return Err(BlobStorageViolation::InvalidRelocationHandle {
                    expected_file: source_vlog,
                    actual_file: old_h.file_number,
                });
            }
            if new_h.file_number != target_vlog {
                return Err(BlobStorageViolation::InvalidRelocationHandle {
                    expected_file: target_vlog,
                    actual_file: new_h.file_number,
                });
            }
            if new_h.size == 0 {
                return Err(BlobStorageViolation::RelocationIntegrityMismatch {
                    old_handle: *old_h,
                    new_handle: *new_h,
                    reason: "Relocated blob size cannot be zero",
                });
            }
            if old_h.size != new_h.size {
                return Err(BlobStorageViolation::RelocationIntegrityMismatch {
                    old_handle: *old_h,
                    new_handle: *new_h,
                    reason: "Relocated blob size must match original blob size",
                });
            }
            if old_h.value_crc != new_h.value_crc {
                return Err(BlobStorageViolation::RelocationIntegrityMismatch {
                    old_handle: *old_h,
                    new_handle: *new_h,
                    reason: "Relocated blob CRC must match original blob CRC",
                });
            }
            if target_total_size > 0 && new_h.offset.saturating_add(new_h.size as u64) > target_total_size {
                return Err(BlobStorageViolation::RelocationIntegrityMismatch {
                    old_handle: *old_h,
                    new_handle: *new_h,
                    reason: "Relocated blob offset plus size exceeds target vlog file size",
                });
            }
        }

        source_meta.is_marked_for_gc = true;

        self.pending_relocations
            .insert(source_vlog, relocations.clone());

        Ok(BlobGcPhase::BlobsRelocated {
            source_vlog,
            target_vlog,
            relocations,
        })
    }

    /// Executa a Fase 2 do GC: comita atomicamente os novos ponteiros na LSM.
    pub fn execute_phase2_commit_lsm(
        &mut self,
        source_vlog: u64,
    ) -> Result<BlobGcPhase, BlobStorageViolation> {
        let relocations = self
            .pending_relocations
            .remove(&source_vlog)
            .ok_or(BlobStorageViolation::Phase1NotExecuted { source_vlog })?;

        let mut affected_targets = HashSet::new();

        for handle in self.lsm_index.values_mut() {
            if handle.file_number == source_vlog {
                if let Some(&new_handle) = relocations.get(handle) {
                    affected_targets.insert(new_handle.file_number);
                    *handle = new_handle;
                }
            }
        }

        self.committed_gc_vlogs.insert(source_vlog);

        // Recalcula contagem de blobs ativos no vlog de origem
        if let Some(meta) = self.vlog_files.get_mut(&source_vlog) {
            meta.active_blobs_count = self
                .lsm_index
                .values()
                .filter(|h| h.file_number == source_vlog)
                .count();
        }

        // Recalcula contagem de blobs ativos nos vlogs de destino afetados
        for target_file in affected_targets {
            if let Some(target_meta) = self.vlog_files.get_mut(&target_file) {
                target_meta.active_blobs_count = self
                    .lsm_index
                    .values()
                    .filter(|h| h.file_number == target_file)
                    .count();
            }
        }

        Ok(BlobGcPhase::LsmPointersCommitted { source_vlog })
    }

    /// Executa a Fase 3 do GC: descarta com segurança física o arquivo de vLog.
    pub fn execute_phase3_safe_purge(
        &mut self,
        file_number: u64,
    ) -> Result<BlobGcPhase, BlobStorageViolation> {
        let meta = self
            .vlog_files
            .get_mut(&file_number)
            .ok_or(BlobStorageViolation::DanglingPointerDetected {
                key: vec![],
                handle: BlobHandle {
                    file_number,
                    offset: 0,
                    size: 0,
                    value_crc: 0,
                },
                reason: "File to purge not found",
            })?;

        if meta.is_physically_deleted {
            return Err(BlobStorageViolation::AlreadyPurged {
                vlog_file: file_number,
            });
        }

        // Valida invariante de não-vazamento: nenhuma chave na LSM pode apontar para este arquivo
        let active_references = self
            .lsm_index
            .values()
            .filter(|h| h.file_number == file_number)
            .count();

        if active_references > 0 {
            return Err(BlobStorageViolation::PrematureVlogDeletion {
                vlog_file: file_number,
                active_references,
            });
        }

        // Se o arquivo tiver uma Fase 1 de relocalização pendente (ainda não comitada na LSM),
        // o descarte físico violaria o protocolo de 2 fases
        if self.pending_relocations.contains_key(&file_number) {
            return Err(BlobStorageViolation::Phase1NotExecuted {
                source_vlog: file_number,
            });
        }

        meta.is_physically_deleted = true;
        self.committed_gc_vlogs.remove(&file_number);

        Ok(BlobGcPhase::VlogPhysicallyPurged {
            purged_file: file_number,
        })
    }

    /// Verifica estrita ausência de dangling pointers em 100% do catálogo.
    pub fn verify_causal_soundness(&self) -> Result<(), BlobStorageViolation> {
        for (key, handle) in &self.lsm_index {
            if handle.size == 0 {
                return Err(BlobStorageViolation::DanglingPointerDetected {
                    key: key.clone(),
                    handle: *handle,
                    reason: "Blob handle size cannot be zero",
                });
            }

            let meta = self.vlog_files.get(&handle.file_number);
            match meta {
                None => {
                    return Err(BlobStorageViolation::DanglingPointerDetected {
                        key: key.clone(),
                        handle: *handle,
                        reason: "Referenced vlog file does not exist",
                    });
                }
                Some(m) if m.is_physically_deleted => {
                    return Err(BlobStorageViolation::DanglingPointerDetected {
                        key: key.clone(),
                        handle: *handle,
                        reason: "Referenced vlog file was physically deleted",
                    });
                }
                Some(m)
                    if m.total_size_bytes > 0
                        && handle.offset.saturating_add(handle.size as u64) > m.total_size_bytes =>
                {
                    return Err(BlobStorageViolation::DanglingPointerDetected {
                        key: key.clone(),
                        handle: *handle,
                        reason: "Blob handle offset plus size exceeds vlog file size",
                    });
                }
                Some(_) => continue,
            }
        }
        Ok(())
    }
}
