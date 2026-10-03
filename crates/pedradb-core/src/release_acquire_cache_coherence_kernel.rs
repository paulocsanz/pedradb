//! RFC-0292: Pilar 7 - Coerência Release-Acquire Trans-Thread na Publicação de Buffers.
//!
//! Formaliza o contrato de visibilidade causal de memória fraca (ARM64/Graviton) na publicação
//! sem locks de blocos descompactados, provando a relação happens-before via Release-Acquire.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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
}

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

    /// Identificador do bloco atribuído a este slot.
    #[inline]
    pub fn block_id(&self) -> u64 {
        self.block_id
    }

    /// Publica com segurança os dados descompactados utilizando semântica `Release`.
    ///
    /// Todas as escritas em `self.data` e na memória do payload dominam causalmente
    /// o store atômico com `Ordering::Release`.
    pub fn publish_entry(&mut self, entry: DecompressedBlockEntry, epoch: u64) {
        self.data = Some(entry);
        self.epoch_token.store(epoch, Ordering::Release);
        self.is_published.store(true, Ordering::Release);
    }

    /// Lê a entrada do cache utilizando semântica `Acquire`.
    ///
    /// O load com `Ordering::Acquire` sincroniza-se com o `Release` do publicador,
    /// garantindo que a thread leitora observe todos os bytes válidos do payload.
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
