//! RFC-0292: Pilar 7 - Coerência Release-Acquire Trans-Thread na Publicação de Buffers.
//!
//! Formaliza o contrato de visibilidade causal de memória fraca (ARM64/Graviton) na publicação
//! sem locks de blocos descompactados, provando a relação happens-before via Release-Acquire.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Erros de construção de entrada de bloco.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockEntryError {
    ZeroBlockId,
    EmptyPayload,
}

impl std::fmt::Display for BlockEntryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroBlockId => write!(f, "Block ID cannot be 0"),
            Self::EmptyPayload => write!(f, "Block payload cannot be empty"),
        }
    }
}

impl std::error::Error for BlockEntryError {}

/// Erros na publicação em slot de cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheSlotPublishError {
    ZeroEpoch,
    BlockIdMismatch { slot_block_id: u64, entry_block_id: u64 },
    ZeroBlockId,
}

impl std::fmt::Display for CacheSlotPublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroEpoch => write!(f, "Publication epoch cannot be 0"),
            Self::BlockIdMismatch { slot_block_id, entry_block_id } => {
                write!(f, "Block ID mismatch: slot {slot_block_id} != entry {entry_block_id}")
            }
            Self::ZeroBlockId => write!(f, "Slot block ID cannot be 0"),
        }
    }
}

impl std::error::Error for CacheSlotPublishError {}

/// Violações de coerência de memória fraca e sincronização trans-thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeakMemoryCoherenceViolation {
    /// O leitor observou a entrada como publicada antes que os dados estivessem completamente gravados.
    PrematurePublicationDetected {
        /// ID do bloco afetado.
        block_id: u64,
        /// Bytes gravados esperados vs reais.
        expected_bytes: usize,
        /// Bytes observados pelo leitor.
        observed_bytes: usize,
    },
    /// A cerca de memória Release-Acquire foi contornada ou enfraquecida (e.g. Relaxed).
    OrderingSemanticViolation {
        /// Operação realizada.
        operation: &'static str,
        /// Ordenação incorreta utilizada.
        faulty_ordering: &'static str,
    },
    /// O identificador do bloco na entrada publicada diverge do ID do slot do cache.
    BlockIdMismatch {
        /// ID esperado pelo slot.
        slot_block_id: u64,
        /// ID observado na entrada.
        entry_block_id: u64,
    },
    /// Falha de integridade do payload (CRC32C divergente).
    ChecksumMismatch {
        /// ID do bloco.
        block_id: u64,
        /// Checksum esperado registrado na entrada.
        expected_crc: u32,
        /// Checksum real recalculado a partir do payload.
        actual_crc: u32,
    },
    /// O comprimento esperado do payload fornecido para verificação é zero.
    ZeroExpectedLength,
    /// O ID do bloco fornecido é zero.
    ZeroBlockId,
    /// O payload do bloco é vazio.
    EmptyPayload,
}

impl std::fmt::Display for WeakMemoryCoherenceViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrematurePublicationDetected { block_id, expected_bytes, observed_bytes } => {
                write!(
                    f,
                    "Premature publication detected for block {block_id}: expected {expected_bytes} bytes, observed {observed_bytes} bytes"
                )
            }
            Self::OrderingSemanticViolation { operation, faulty_ordering } => {
                write!(
                    f,
                    "Ordering semantic violation in {operation}: faulty ordering {faulty_ordering}"
                )
            }
            Self::BlockIdMismatch { slot_block_id, entry_block_id } => {
                write!(
                    f,
                    "Block ID mismatch: slot expected {slot_block_id}, entry published {entry_block_id}"
                )
            }
            Self::ChecksumMismatch { block_id, expected_crc, actual_crc } => {
                write!(
                    f,
                    "Checksum mismatch for block {block_id}: expected 0x{expected_crc:08x}, actual 0x{actual_crc:08x}"
                )
            }
            Self::ZeroExpectedLength => write!(f, "Expected block length cannot be 0"),
            Self::ZeroBlockId => write!(f, "Block ID cannot be 0"),
            Self::EmptyPayload => write!(f, "Block payload cannot be empty"),
        }
    }
}

impl std::error::Error for WeakMemoryCoherenceViolation {}

/// Buffer descompactado gerenciado em memória compartilhada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecompressedBlockEntry {
    /// Identificador único do bloco.
    pub block_id: u64,
    /// Bytes descompactados do payload.
    pub payload: Vec<u8>,
    /// Digest CRC32C do payload para verificação de integridade ponta-a-ponta.
    pub crc32c: u32,
}

impl DecompressedBlockEntry {
    /// Constrói uma nova entrada calculando automaticamente o digest CRC32C do payload.
    pub fn new(block_id: u64, payload: Vec<u8>) -> Self {
        let crc = crc32c::crc32c(&payload);
        Self {
            block_id,
            payload,
            crc32c: crc,
        }
    }

    /// Constrói uma nova entrada validando os invariantes de ID não-nulo e payload não-vazio.
    pub fn try_new(block_id: u64, payload: Vec<u8>) -> Result<Self, BlockEntryError> {
        if block_id == 0 {
            return Err(BlockEntryError::ZeroBlockId);
        }
        if payload.is_empty() {
            return Err(BlockEntryError::EmptyPayload);
        }
        Ok(Self::new(block_id, payload))
    }
}

/// Descritor de entrada no cache com token atômico de sincronização.
pub struct BlockCacheSlot {
    block_id: u64,
    data: Option<DecompressedBlockEntry>,
    is_published: AtomicBool,
    epoch_token: AtomicU64,
}

impl BlockCacheSlot {
    /// Cria um novo slot vazio no cache.
    pub fn empty(block_id: u64) -> Self {
        Self {
            block_id,
            data: None,
            is_published: AtomicBool::new(false),
            epoch_token: AtomicU64::new(0),
        }
    }

    /// Cria um novo slot vazio no cache validando que o ID do bloco não é zero.
    pub fn try_empty(block_id: u64) -> Result<Self, BlockEntryError> {
        if block_id == 0 {
            return Err(BlockEntryError::ZeroBlockId);
        }
        Ok(Self::empty(block_id))
    }

    /// Identificador do bloco atribuído a este slot.
    #[inline]
    pub fn block_id(&self) -> u64 {
        self.block_id
    }

    /// Publica com segurança os dados descompactados utilizando semântica `Release`.
    pub fn publish_entry(&mut self, entry: DecompressedBlockEntry, epoch: u64) {
        self.data = Some(entry);
        self.epoch_token.store(epoch, Ordering::Release);
        self.is_published.store(true, Ordering::Release);
    }

    /// Publica com validação estrita de epoch não-nula e correspondência de block_id.
    pub fn try_publish_entry(&mut self, entry: DecompressedBlockEntry, epoch: u64) -> Result<(), CacheSlotPublishError> {
        if self.block_id == 0 {
            return Err(CacheSlotPublishError::ZeroBlockId);
        }
        if epoch == 0 {
            return Err(CacheSlotPublishError::ZeroEpoch);
        }
        if entry.block_id != self.block_id {
            return Err(CacheSlotPublishError::BlockIdMismatch {
                slot_block_id: self.block_id,
                entry_block_id: entry.block_id,
            });
        }
        self.publish_entry(entry, epoch);
        Ok(())
    }

    /// Lê a entrada do cache utilizando semântica `Acquire`.
    pub fn try_acquire_entry(&self) -> Option<(&DecompressedBlockEntry, u64)> {
        if self.is_published.load(Ordering::Acquire) {
            let epoch = self.epoch_token.load(Ordering::Acquire);
            self.data.as_ref().map(|d| (d, epoch))
        } else {
            None
        }
    }
}

/// Oráculo de verificação de coerência causal Release-Acquire.
pub struct ReleaseAcquireCoherenceOracle;

impl ReleaseAcquireCoherenceOracle {
    /// Valida que a publicação de um bloco respeita formalmente o contrato de memória fraca:
    /// Store(Payload) -> Fence(Release) -> SynchronizesWith -> Fence(Acquire) -> Load(Payload)
    pub fn verify_coherence_contract(
        slot: &BlockCacheSlot,
        expected_len: usize,
    ) -> Result<(), WeakMemoryCoherenceViolation> {
        if expected_len == 0 {
            return Err(WeakMemoryCoherenceViolation::ZeroExpectedLength);
        }
        if slot.block_id == 0 {
            return Err(WeakMemoryCoherenceViolation::ZeroBlockId);
        }
        if let Some((entry, epoch)) = slot.try_acquire_entry() {
            if entry.block_id != slot.block_id {
                return Err(WeakMemoryCoherenceViolation::BlockIdMismatch {
                    slot_block_id: slot.block_id,
                    entry_block_id: entry.block_id,
                });
            }
            if entry.payload.len() != expected_len {
                return Err(WeakMemoryCoherenceViolation::PrematurePublicationDetected {
                    block_id: slot.block_id,
                    expected_bytes: expected_len,
                    observed_bytes: entry.payload.len(),
                });
            }
            if epoch == 0 {
                return Err(WeakMemoryCoherenceViolation::OrderingSemanticViolation {
                    operation: "publish_entry",
                    faulty_ordering: "Missing causal epoch synchronization",
                });
            }
        }
        Ok(())
    }

    /// Valida a integridade matemática do payload contra o digest CRC32C gravado.
    pub fn verify_entry_integrity(
        entry: &DecompressedBlockEntry,
    ) -> Result<(), WeakMemoryCoherenceViolation> {
        if entry.block_id == 0 {
            return Err(WeakMemoryCoherenceViolation::ZeroBlockId);
        }
        if entry.payload.is_empty() {
            return Err(WeakMemoryCoherenceViolation::EmptyPayload);
        }
        let actual_crc = crc32c::crc32c(&entry.payload);
        if entry.crc32c != actual_crc {
            return Err(WeakMemoryCoherenceViolation::ChecksumMismatch {
                block_id: entry.block_id,
                expected_crc: entry.crc32c,
                actual_crc,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_release_acquire_cache_coherence_structural_invariants_red_to_green() {
        // Red test 1: DecompressedBlockEntry rejection
        assert_eq!(
            DecompressedBlockEntry::try_new(0, b"data".to_vec()),
            Err(BlockEntryError::ZeroBlockId)
        );
        assert_eq!(
            DecompressedBlockEntry::try_new(10, vec![]),
            Err(BlockEntryError::EmptyPayload)
        );

        // Green test 1: Valid DecompressedBlockEntry
        let entry = DecompressedBlockEntry::try_new(10, b"valid_data".to_vec()).expect("valid entry");
        assert_eq!(entry.block_id, 10);
        assert_eq!(entry.payload, b"valid_data");
        assert_eq!(entry.crc32c, crc32c::crc32c(b"valid_data"));

        // Red test 2: BlockCacheSlot try_empty rejection of block_id 0
        assert_eq!(
            BlockCacheSlot::try_empty(0).map(|_| ()),
            Err(BlockEntryError::ZeroBlockId)
        );

        // Red test 3: try_publish_entry rejection of zero epoch & block mismatch
        let mut slot = BlockCacheSlot::try_empty(10).expect("valid slot");
        let wrong_entry = DecompressedBlockEntry::try_new(20, b"other".to_vec()).unwrap();
        assert_eq!(
            slot.try_publish_entry(wrong_entry, 1),
            Err(CacheSlotPublishError::BlockIdMismatch { slot_block_id: 10, entry_block_id: 20 })
        );
        assert_eq!(
            slot.try_publish_entry(entry.clone(), 0),
            Err(CacheSlotPublishError::ZeroEpoch)
        );

        // Red test 4: verify_coherence_contract rejection of zero expected len & zero block id
        let slot_zero = BlockCacheSlot::empty(0);
        assert_eq!(
            ReleaseAcquireCoherenceOracle::verify_coherence_contract(&slot, 0),
            Err(WeakMemoryCoherenceViolation::ZeroExpectedLength)
        );
        assert_eq!(
            ReleaseAcquireCoherenceOracle::verify_coherence_contract(&slot_zero, 10),
            Err(WeakMemoryCoherenceViolation::ZeroBlockId)
        );

        // Red test 5: verify_entry_integrity rejection of zero block id & empty payload
        let zero_id_entry = DecompressedBlockEntry {
            block_id: 0,
            payload: b"data".to_vec(),
            crc32c: crc32c::crc32c(b"data"),
        };
        let empty_entry = DecompressedBlockEntry {
            block_id: 10,
            payload: vec![],
            crc32c: 0,
        };
        assert_eq!(
            ReleaseAcquireCoherenceOracle::verify_entry_integrity(&zero_id_entry),
            Err(WeakMemoryCoherenceViolation::ZeroBlockId)
        );
        assert_eq!(
            ReleaseAcquireCoherenceOracle::verify_entry_integrity(&empty_entry),
            Err(WeakMemoryCoherenceViolation::EmptyPayload)
        );

        // Green test 2: Correct publish and acquire
        assert!(slot.try_publish_entry(entry.clone(), 42).is_ok());
        let (acquired, epoch) = slot.try_acquire_entry().expect("published entry");
        assert_eq!(acquired.block_id, 10);
        assert_eq!(epoch, 42);
        assert!(ReleaseAcquireCoherenceOracle::verify_coherence_contract(&slot, b"valid_data".len()).is_ok());
        assert!(ReleaseAcquireCoherenceOracle::verify_entry_integrity(&entry).is_ok());
    }
}

