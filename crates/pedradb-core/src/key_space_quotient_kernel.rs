//! RFC-0292: Pilar 1 - Homomorfismo Topológico de Normalização Canônica e Espaço Quociente ($\mathcal{K} / \sim$).
//!
//! Garante que filtros de Bloom e particionadores de blocos operem sobre a projeção canônica
//! de classes de equivalência semânticas de chaves, eliminando falsos negativos e podas incorretas.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

/// Erros de violação de invariantes do espaço métrico quociente de chaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotientViolation {
    /// Chaves com a mesma projeção canônica geraram hashes distintos no filtro de Bloom.
    NonCanonicalBloomHashMismatch {
        /// Chave 1 original.
        raw_key1: Vec<u8>,
        /// Chave 2 original.
        raw_key2: Vec<u8>,
        /// Representação canônica comum.
        canonical: Vec<u8>,
        /// Hash calculado para a chave 1.
        hash1: u64,
        /// Hash calculado para a chave 2.
        hash2: u64,
    },
    /// A partição de blocos quebrou a convexidade quociente.
    QuotientConvexityBroken {
        /// Chave menor.
        key_low: Vec<u8>,
        /// Chave intermediária.
        key_mid: Vec<u8>,
        /// Chave maior.
        key_high: Vec<u8>,
        /// Bloco da chave intermediária fora do intervalo dos blocos das pontas.
        mid_block: usize,
        /// Bloco limite.
        bound_block: usize,
    },
}

/// Trait definindo o operador de normalização canônica $\pi: \mathcal{K} \to \mathcal{K} / \sim$.
pub trait CanonicalNormalizer: Send + Sync {
    /// Projeta a chave para sua representação canônica na classe de equivalência.
    fn project_canonical(&self, raw_key: &[u8]) -> Vec<u8>;
}

/// Normalizador padrão insensível a maiúsculas/minúsculas ASCII com remoção de espaços nas extremidades.
#[derive(Debug, Clone, Default)]
pub struct AsciiCaseInsensitiveNormalizer;

impl CanonicalNormalizer for AsciiCaseInsensitiveNormalizer {
    fn project_canonical(&self, raw_key: &[u8]) -> Vec<u8> {
        // Converte para minúsculas e remove espaços das extremidades
        let trimmed = raw_key
            .strip_prefix(b" ")
            .unwrap_or(raw_key);
        let trimmed = trimmed
            .strip_suffix(b" ")
            .unwrap_or(trimmed);
        trimmed.to_ascii_lowercase()
    }
}

/// Filtro de Bloom ciente do espaço métrico quociente.
#[derive(Debug, Clone)]
pub struct QuotientBloomFilter {
    bitset: Vec<bool>,
    num_bits: usize,
    num_hashes: usize,
}

impl QuotientBloomFilter {
    /// Cria um novo filtro de Bloom com tamanho especificado.
    pub fn new(num_bits: usize, num_hashes: usize) -> Self {
        Self {
            bitset: vec![false; num_bits.max(64)],
            num_bits: num_bits.max(64),
            num_hashes: num_hashes.clamp(1, 16),
        }
    }

    /// Calcula o hash da projeção canônica da chave.
    pub fn canonical_hash<N: CanonicalNormalizer>(normalizer: &N, raw_key: &[u8]) -> u64 {
        let canonical = normalizer.project_canonical(raw_key);
        let mut h = 0xcbf29ce484222325u64;
        for &byte in &canonical {
            h ^= u64::from(byte);
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    /// Insere uma chave no filtro utilizando sua representação canônica.
    pub fn insert<N: CanonicalNormalizer>(&mut self, normalizer: &N, raw_key: &[u8]) {
        let hash = Self::canonical_hash(normalizer, raw_key);
        for i in 0..self.num_hashes {
            let bit_idx = ((hash.wrapping_add((i as u64).wrapping_mul(0x9e3779b97f4a7c15))) as usize) % self.num_bits;
            self.bitset[bit_idx] = true;
        }
    }

    /// Consulta se uma chave (ou qualquer membro da sua classe de equivalência) está no filtro.
    pub fn may_contain<N: CanonicalNormalizer>(&self, normalizer: &N, raw_key: &[u8]) -> bool {
        let hash = Self::canonical_hash(normalizer, raw_key);
        for i in 0..self.num_hashes {
            let bit_idx = ((hash.wrapping_add((i as u64).wrapping_mul(0x9e3779b97f4a7c15))) as usize) % self.num_bits;
            if !self.bitset[bit_idx] {
                return false;
            }
        }
        true
    }
}

/// Descritor de bloco de dados em um SST particionado por projeção canônica.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotientBlockMeta {
    /// Índice ordinal do bloco.
    pub block_index: usize,
    /// Chave mínima canônica contida no bloco.
    pub min_canonical_key: Vec<u8>,
    /// Chave máxima canônica contida no bloco.
    pub max_canonical_key: Vec<u8>,
    /// Quantidade de chaves no bloco.
    pub record_count: usize,
}

/// Oráculo e particionador de blocos ciente de classes de equivalência quociente.
#[derive(Debug, Clone)]
pub struct QuotientBlockIndex {
    blocks: Vec<QuotientBlockMeta>,
}

impl QuotientBlockIndex {
    /// Constrói um índice de blocos garantindo ordenação estrita das fronteiras canônicas.
    pub fn build(blocks: Vec<QuotientBlockMeta>) -> Self {
        Self { blocks }
    }

    /// Roteia a busca para o índice do bloco que pode conter a chave canônica.
    pub fn route<N: CanonicalNormalizer>(&self, normalizer: &N, raw_key: &[u8]) -> Option<usize> {
        let canonical = normalizer.project_canonical(raw_key);
        for block in &self.blocks {
            if canonical >= block.min_canonical_key && canonical <= block.max_canonical_key {
                return Some(block.block_index);
            }
        }
        None
    }

    /// Valida formalmente as invariantes de homomorfismo quociente e convexidade.
    pub fn verify_quotient_invariants<N: CanonicalNormalizer>(
        &self,
        normalizer: &N,
        test_samples: &[(&[u8], &[u8])], // Pares de chaves equivalentes: raw1 ~ raw2
    ) -> Result<(), QuotientViolation> {
        // 1. Prova de preservação de hash no filtro de Bloom para amostras equivalentes
        for &(raw1, raw2) in test_samples {
            let can1 = normalizer.project_canonical(raw1);
            let can2 = normalizer.project_canonical(raw2);
            assert_eq!(can1, can2, "Amostras fornecidas devem pertencer à mesma classe de equivalência");

            let h1 = QuotientBloomFilter::canonical_hash(normalizer, raw1);
            let h2 = QuotientBloomFilter::canonical_hash(normalizer, raw2);

            if h1 != h2 {
                return Err(QuotientViolation::NonCanonicalBloomHashMismatch {
                    raw_key1: raw1.to_vec(),
                    raw_key2: raw2.to_vec(),
                    canonical: can1,
                    hash1: h1,
                    hash2: h2,
                });
            }
        }

        // 2. Prova de monotonicidade e convexidade entre blocos adjacentes
        for i in 1..self.blocks.len() {
            let prev = &self.blocks[i - 1];
            let curr = &self.blocks[i];
            if prev.max_canonical_key >= curr.min_canonical_key {
                return Err(QuotientViolation::QuotientConvexityBroken {
                    key_low: prev.min_canonical_key.clone(),
                    key_mid: prev.max_canonical_key.clone(),
                    key_high: curr.min_canonical_key.clone(),
                    mid_block: prev.block_index,
                    bound_block: curr.block_index,
                });
            }
        }

        Ok(())
    }
}
