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
}

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

    /// Executa a Fase 1 do GC: reescreve blobs e gera mapa de relocalização.
    pub fn execute_phase1_rewrite(
        &mut self,
        source_vlog: u64,
        target_vlog: u64,
        relocations: HashMap<BlobHandle, BlobHandle>,
    ) -> Result<BlobGcPhase, BlobStorageViolation> {
        if let Some(source_meta) = self.vlog_files.get_mut(&source_vlog) {
            source_meta.is_marked_for_gc = true;
        } else {
            return Err(BlobStorageViolation::DanglingPointerDetected {
                key: vec![],
                handle: BlobHandle {
                    file_number: source_vlog,
                    offset: 0,
                    size: 0,
                    value_crc: 0,
                },
                reason: "Source vlog does not exist in catalog",
            });
        }

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
            .unwrap_or_default();

        for (key, handle) in self.lsm_index.iter_mut() {
            if handle.file_number == source_vlog {
                if let Some(&new_handle) = relocations.get(handle) {
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

        Ok(BlobGcPhase::LsmPointersCommitted { source_vlog })
    }

    /// Executa a Fase 3 do GC: descarta com segurança física o arquivo de vLog.
    pub fn execute_phase3_safe_purge(
        &mut self,
        file_number: u64,
    ) -> Result<BlobGcPhase, BlobStorageViolation> {
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

        meta.is_physically_deleted = true;
        self.committed_gc_vlogs.remove(&file_number);

        Ok(BlobGcPhase::VlogPhysicallyPurged {
            purged_file: file_number,
        })
    }

    /// Verifica estrita ausência de dangling pointers em 100% do catálogo.
    pub fn verify_causal_soundness(&self) -> Result<(), BlobStorageViolation> {
        for (key, handle) in &self.lsm_index {
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
                Some(_) => continue,
            }
        }
        Ok(())
    }
}
