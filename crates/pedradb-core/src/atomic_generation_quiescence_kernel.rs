//! RFC-0292: Pilar 6 - Quiescência Atômica de Geração em MemTables Concorrentes.
//!
//! Garante que a transição de MemTable ativa para congelada aguarde a quiescência de todos os
//! escritores com tickets pendentes na geração, erradicando a omissão silenciosa de chaves no flusher.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Violações de quiescência atômica de geração.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuiescenceViolation {
    /// O flusher iniciou o corte do SST enquanto ainda existiam escritores ativos na geração.
    PrematureFlushBeforeQuiescence {
        /// ID da geração.
        generation: u64,
        /// Quantidade de escritores pendentes.
        pending_writers: usize,
    },
    /// Uma chave com sequence number da geração foi omitida do SST gerado pelo flush.
    KeyOmittedFromFlushedSst {
        /// Chave perdida.
        key: Vec<u8>,
        /// Sequence number da chave.
        seq: u64,
        /// Geração da chave.
        generation: u64,
    },
}

/// Estado do ciclo de vida de uma geração de MemTable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationState {
    /// Ativa para novas escritas.
    Active,
    /// Em rotação: não aceita novos escritores, aguarda drenagem dos já admitidos.
    Quiescing,
    /// Quiescente: todos os escritores concluíram suas inserções. Pronta para flush.
    Quiesced,
    /// Já persistida em arquivo SST imutável.
    Flushed,
}

/// Token de escrita adquirido por uma thread cliente.
pub struct WriteTicket {
    generation: u64,
    active_counter: Arc<AtomicUsize>,
}

impl Drop for WriteTicket {
    fn drop(&mut self) {
        self.active_counter.fetch_sub(1, Ordering::Release);
    }
}

/// Descritor de uma geração de MemTable.
#[derive(Debug)]
pub struct MemTableGeneration {
    /// Número ordinal da geração.
    pub generation_id: u64,
    /// Contador atômico de escritores ativos na geração.
    pub active_writers: Arc<AtomicUsize>,
    /// Estado atual da geração.
    pub state: GenerationState,
    /// Chaves inseridas nesta geração.
    pub inserted_keys: Vec<(Vec<u8>, u64)>,
}

impl MemTableGeneration {
    /// Cria uma nova geração ativa.
    pub fn new(generation_id: u64) -> Self {
        Self {
            generation_id,
            active_writers: Arc::new(AtomicUsize::new(0)),
            state: GenerationState::Active,
            inserted_keys: Vec::new(),
        }
    }

    /// Tenta admitir um novo escritor. Retorna Some(WriteTicket) se estiver ativa.
    pub fn acquire_write_ticket(&self) -> Option<WriteTicket> {
        if self.state != GenerationState::Active {
            return None;
        }
        self.active_writers.fetch_add(1, Ordering::SeqCst);
        Some(WriteTicket {
            generation: self.generation_id,
            active_counter: Arc::clone(&self.active_writers),
        })
    }

    /// Registra a inserção de uma chave na MemTable (realizada sob posse de um ticket).
    pub fn record_insertion(&mut self, key: Vec<u8>, seq: u64) {
        self.inserted_keys.push((key, seq));
    }

    /// Inicia o processo de quiescência (congelamento da MemTable).
    pub fn begin_quiescence(&mut self) {
        self.state = GenerationState::Quiescing;
    }

    /// Tenta transicionar para `Quiesced` verificando se o contador de escritores chegou a zero.
    pub fn try_quiesce(&mut self) -> bool {
        if self.state == GenerationState::Quiescing {
            if self.active_writers.load(Ordering::Acquire) == 0 {
                self.state = GenerationState::Quiesced;
                return true;
            }
        }
        self.state == GenerationState::Quiesced
    }
}

/// Oráculo de verificação do pipeline de quiescência de geração.
pub struct GenerationQuiescenceOracle;

impl GenerationQuiescenceOracle {
    /// Valida que a entrega de uma MemTable ao flusher respeitou estritamente a quiescência
    /// e que 100% das chaves inseridas na geração foram incluídas no SST persistido.
    pub fn verify_flush_safety(
        generation: &MemTableGeneration,
        flushed_sst_keys: &[(Vec<u8>, u64)],
    ) -> Result<(), QuiescenceViolation> {
        let pending = generation.active_writers.load(Ordering::Acquire);
        if pending > 0 || generation.state != GenerationState::Quiesced {
            return Err(QuiescenceViolation::PrematureFlushBeforeQuiescence {
                generation: generation.generation_id,
                pending_writers: pending,
            });
        }

        let sst_set: HashSet<_> = flushed_sst_keys.iter().collect();

        // Prova de completude: nenhuma chave inserida na geração pode estar ausente do SST
        for (key, seq) in &generation.inserted_keys {
            if !sst_set.contains(&(key.clone(), *seq)) {
                return Err(QuiescenceViolation::KeyOmittedFromFlushedSst {
                    key: key.clone(),
                    seq: *seq,
                    generation: generation.generation_id,
                });
            }
        }

        Ok(())
    }
}
