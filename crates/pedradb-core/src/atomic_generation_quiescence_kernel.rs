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
    /// Uma inserção foi tentada com um ticket de geração incompatível com a MemTable alvo.
    GenerationMismatch {
        /// Geração do ticket de escrita.
        ticket_generation: u64,
        /// Geração da MemTable alvo.
        memtable_generation: u64,
    },
    /// ID de geração não pode ser zero.
    ZeroGenerationId,
    /// Chave de inserção não pode ser vazia.
    KeyEmpty,
    /// Estado da geração é inválido para a operação solicitada.
    InvalidGenerationState {
        /// Estado atual da geração.
        state: GenerationState,
    },
}

impl std::fmt::Display for QuiescenceViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrematureFlushBeforeQuiescence { generation, pending_writers } => write!(
                f,
                "Premature flush before quiescence for generation {generation}: {pending_writers} pending writers"
            ),
            Self::KeyOmittedFromFlushedSst { key, seq, generation } => write!(
                f,
                "Key {:?} at seq {seq} omitted from flushed SST for generation {generation}",
                key
            ),
            Self::GenerationMismatch { ticket_generation, memtable_generation } => write!(
                f,
                "Generation mismatch: ticket generation {ticket_generation} != memtable generation {memtable_generation}"
            ),
            Self::ZeroGenerationId => write!(f, "Generation ID cannot be zero"),
            Self::KeyEmpty => write!(f, "Key cannot be empty"),
            Self::InvalidGenerationState { state } => write!(
                f,
                "Operation invalid under generation state {state:?}"
            ),
        }
    }
}

impl std::error::Error for QuiescenceViolation {}

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

impl WriteTicket {
    /// Identificador da geração associada a este ticket.
    #[inline]
    pub fn generation(&self) -> u64 {
        self.generation
    }
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
    /// Cria uma nova geração ativa com validação de limites.
    pub fn try_new(generation_id: u64) -> Result<Self, QuiescenceViolation> {
        if generation_id == 0 {
            return Err(QuiescenceViolation::ZeroGenerationId);
        }
        Ok(Self::new(generation_id))
    }

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

    /// Registra a inserção de uma chave com validação de estado e chave não vazia.
    pub fn try_record_insertion(&mut self, key: Vec<u8>, seq: u64) -> Result<(), QuiescenceViolation> {
        if key.is_empty() {
            return Err(QuiescenceViolation::KeyEmpty);
        }
        if self.state == GenerationState::Flushed || self.state == GenerationState::Quiesced {
            return Err(QuiescenceViolation::InvalidGenerationState { state: self.state });
        }
        self.record_insertion(key, seq);
        Ok(())
    }

    /// Registra a inserção de uma chave na MemTable (realizada sob posse de um ticket).
    pub fn record_insertion(&mut self, key: Vec<u8>, seq: u64) {
        self.inserted_keys.push((key, seq));
    }

    /// Registra a inserção de uma chave garantindo correspondência estrita com a geração do ticket.
    pub fn record_insertion_with_ticket(
        &mut self,
        ticket: &WriteTicket,
        key: Vec<u8>,
        seq: u64,
    ) -> Result<(), QuiescenceViolation> {
        if ticket.generation != self.generation_id {
            return Err(QuiescenceViolation::GenerationMismatch {
                ticket_generation: ticket.generation,
                memtable_generation: self.generation_id,
            });
        }
        if key.is_empty() {
            return Err(QuiescenceViolation::KeyEmpty);
        }
        if self.state == GenerationState::Flushed {
            return Err(QuiescenceViolation::PrematureFlushBeforeQuiescence {
                generation: self.generation_id,
                pending_writers: 0,
            });
        }
        self.inserted_keys.push((key, seq));
        Ok(())
    }

    /// Inicia o processo de quiescência (congelamento da MemTable).
    pub fn begin_quiescence(&mut self) {
        if self.state == GenerationState::Active {
            self.state = GenerationState::Quiescing;
        }
    }

    /// Tenta transicionar para `Quiesced` verificando se o contador de escritores chegou a zero.
    pub fn try_quiesce(&mut self) -> bool {
        if self.state == GenerationState::Quiescing {
            if self.active_writers.load(Ordering::Acquire) == 0 {
                self.state = GenerationState::Quiesced;
                return true;
            }
        }
        self.state == GenerationState::Quiesced || self.state == GenerationState::Flushed
    }

    /// Marca a geração como persistida com validação estrita de quiescência.
    pub fn try_mark_flushed(&mut self) -> Result<(), QuiescenceViolation> {
        let pending = self.active_writers.load(Ordering::Acquire);
        if pending > 0 {
            return Err(QuiescenceViolation::PrematureFlushBeforeQuiescence {
                generation: self.generation_id,
                pending_writers: pending,
            });
        }
        if self.state != GenerationState::Quiesced {
            return Err(QuiescenceViolation::InvalidGenerationState { state: self.state });
        }
        self.state = GenerationState::Flushed;
        Ok(())
    }

    /// Marca a geração como persistida (Flushed) em arquivo SST imutável.
    pub fn mark_flushed(&mut self) -> bool {
        self.try_mark_flushed().is_ok()
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
        if pending > 0
            || (generation.state != GenerationState::Quiesced
                && generation.state != GenerationState::Flushed)
        {
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

    /// Valida que um ticket pertence estritamente à geração informada.
    pub fn verify_ticket_belongs_to_generation(
        ticket: &WriteTicket,
        generation: &MemTableGeneration,
    ) -> Result<(), QuiescenceViolation> {
        if ticket.generation != generation.generation_id {
            return Err(QuiescenceViolation::GenerationMismatch {
                ticket_generation: ticket.generation,
                memtable_generation: generation.generation_id,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generation_quiescence_fail_closed_red_to_green() {
        assert_eq!(MemTableGeneration::try_new(0).err(), Some(QuiescenceViolation::ZeroGenerationId));

        let mut gen = MemTableGeneration::try_new(1).expect("valid gen");
        let ticket = gen.acquire_write_ticket().expect("ticket");
        assert_eq!(
            gen.record_insertion_with_ticket(&ticket, vec![], 1).err(),
            Some(QuiescenceViolation::KeyEmpty)
        );

        // Cannot mark flushed while writers are pending
        assert_eq!(
            gen.try_mark_flushed().err(),
            Some(QuiescenceViolation::PrematureFlushBeforeQuiescence {
                generation: 1,
                pending_writers: 1,
            })
        );

        drop(ticket);
        gen.begin_quiescence();
        assert!(gen.try_quiesce());
        assert!(gen.try_mark_flushed().is_ok());

        // Cannot insert into flushed memtable
        assert_eq!(
            gen.try_record_insertion(b"key".to_vec(), 2).err(),
            Some(QuiescenceViolation::InvalidGenerationState {
                state: GenerationState::Flushed,
            })
        );
    }
}
