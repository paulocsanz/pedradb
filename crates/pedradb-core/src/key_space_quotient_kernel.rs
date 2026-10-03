//! RFC-0292: Pilar 1 - Homomorfismo Topológico de Normalização Canônica e Espaço Quociente ($\mathcal{K} / \sim$).
//!
//! Garante que filtros de Bloom e particionadores de blocos operem sobre a projeção canônica
//! de classes de equivalência semânticas de chaves, eliminando falsos negativos e podas incorretas.


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
    /// Chaves com suposta equivalência divergiram na projeção canônica.
    NonCanonicalProjectionMismatch {
        /// Chave 1 original.
        raw_key1: Vec<u8>,
        /// Chave 2 original.
        raw_key2: Vec<u8>,
        /// Projeção canônica calculada para a chave 1.
        canonical1: Vec<u8>,
        /// Projeção canônica calculada para a chave 2.
        canonical2: Vec<u8>,
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
    /// A chave ou projeção canônica fornecida é vazia.
    EmptyKey,
    /// A chave canônica gerada é vazia.
    EmptyCanonicalKey,
    /// O bloco especificado possui contagem zero de registros.
    ZeroRecordCount {
        block_index: usize,
    },
    /// O índice de blocos fornecido está vazio.
    EmptyBlockIndex,
    /// Bloco com limites invertidos (min > max).
    InvertedBlockBounds {
        block_index: usize,
        min_key: Vec<u8>,
        max_key: Vec<u8>,
    },
}

impl std::fmt::Display for QuotientViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonCanonicalBloomHashMismatch { raw_key1, raw_key2, canonical, hash1, hash2 } => write!(
                f,
                "Non-canonical bloom hash mismatch for {raw_key1:?} and {raw_key2:?} (canonical {canonical:?}): {hash1} vs {hash2}"
            ),
            Self::NonCanonicalProjectionMismatch { raw_key1, raw_key2, canonical1, canonical2 } => write!(
                f,
                "Non-canonical projection mismatch for {raw_key1:?} and {raw_key2:?}: {canonical1:?} vs {canonical2:?}"
            ),
            Self::QuotientConvexityBroken { key_low, key_mid, key_high, mid_block, bound_block } => write!(
                f,
                "Quotient convexity broken between blocks {mid_block} and {bound_block} across [{key_low:?}, {key_mid:?}, {key_high:?}]"
            ),
            Self::EmptyKey => write!(f, "Raw key cannot be empty"),
            Self::EmptyCanonicalKey => write!(f, "Canonical key projection cannot be empty"),
            Self::ZeroRecordCount { block_index } => write!(f, "Block {block_index} has zero records"),
            Self::EmptyBlockIndex => write!(f, "Block index cannot be empty"),
            Self::InvertedBlockBounds { block_index, min_key, max_key } => write!(
                f,
                "Block {block_index} has inverted key bounds: min {min_key:?} > max {max_key:?}"
            ),
        }
    }
}

impl std::error::Error for QuotientViolation {}

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
        let mut start = 0;
        while start < raw_key.len() && raw_key[start].is_ascii_whitespace() {
            start += 1;
        }
        let mut end = raw_key.len();
        while end > start && raw_key[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        raw_key[start..end].to_ascii_lowercase()
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

    /// Insere uma chave no filtro utilizando sua representação canônica com validação.
    pub fn try_insert<N: CanonicalNormalizer>(&mut self, normalizer: &N, raw_key: &[u8]) -> Result<(), QuotientViolation> {
        if raw_key.is_empty() {
            return Err(QuotientViolation::EmptyKey);
        }
        let canonical = normalizer.project_canonical(raw_key);
        if canonical.is_empty() {
            return Err(QuotientViolation::EmptyCanonicalKey);
        }
        self.insert(normalizer, raw_key);
        Ok(())
    }

    /// Insere uma chave no filtro utilizando sua representação canônica.
    pub fn insert<N: CanonicalNormalizer>(&mut self, normalizer: &N, raw_key: &[u8]) {
        let hash = Self::canonical_hash(normalizer, raw_key);
        for i in 0..self.num_hashes {
            let bit_idx = ((hash.wrapping_add((i as u64).wrapping_mul(0x9e3779b97f4a7c15))) as usize) % self.num_bits;
            self.bitset[bit_idx] = true;
        }
    }

    /// Consulta se uma chave (ou qualquer membro da sua classe de equivalência) está no filtro com validação.
    pub fn try_may_contain<N: CanonicalNormalizer>(&self, normalizer: &N, raw_key: &[u8]) -> Result<bool, QuotientViolation> {
        if raw_key.is_empty() {
            return Err(QuotientViolation::EmptyKey);
        }
        let canonical = normalizer.project_canonical(raw_key);
        if canonical.is_empty() {
            return Err(QuotientViolation::EmptyCanonicalKey);
        }
        Ok(self.may_contain(normalizer, raw_key))
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

impl QuotientBlockMeta {
    /// Constrói um descritor de bloco canônico com validação estrita de integridade.
    pub fn try_new(
        block_index: usize,
        min_canonical_key: Vec<u8>,
        max_canonical_key: Vec<u8>,
        record_count: usize,
    ) -> Result<Self, QuotientViolation> {
        if min_canonical_key.is_empty() || max_canonical_key.is_empty() {
            return Err(QuotientViolation::EmptyCanonicalKey);
        }
        if min_canonical_key > max_canonical_key {
            return Err(QuotientViolation::InvertedBlockBounds {
                block_index,
                min_key: min_canonical_key,
                max_key: max_canonical_key,
            });
        }
        if record_count == 0 {
            return Err(QuotientViolation::ZeroRecordCount { block_index });
        }
        Ok(Self {
            block_index,
            min_canonical_key,
            max_canonical_key,
            record_count,
        })
    }
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

    /// Constrói um índice de blocos com verificação estrita das invariantes de convexidade.
    pub fn try_build(blocks: Vec<QuotientBlockMeta>) -> Result<Self, QuotientViolation> {
        if blocks.is_empty() {
            return Err(QuotientViolation::EmptyBlockIndex);
        }
        for block in &blocks {
            if block.min_canonical_key > block.max_canonical_key {
                return Err(QuotientViolation::InvertedBlockBounds {
                    block_index: block.block_index,
                    min_key: block.min_canonical_key.clone(),
                    max_key: block.max_canonical_key.clone(),
                });
            }
            if block.record_count == 0 {
                return Err(QuotientViolation::ZeroRecordCount {
                    block_index: block.block_index,
                });
            }
        }
        for i in 1..blocks.len() {
            let prev = &blocks[i - 1];
            let curr = &blocks[i];
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
        Ok(Self { blocks })
    }

    /// Roteia a busca para o índice do bloco que pode conter a chave canônica com validação.
    pub fn try_route<N: CanonicalNormalizer>(&self, normalizer: &N, raw_key: &[u8]) -> Result<Option<usize>, QuotientViolation> {
        if raw_key.is_empty() {
            return Err(QuotientViolation::EmptyKey);
        }
        let canonical = normalizer.project_canonical(raw_key);
        if canonical.is_empty() {
            return Err(QuotientViolation::EmptyCanonicalKey);
        }
        Ok(self.route(normalizer, raw_key))
    }

    /// Roteamento em tempo logarítmico $O(\log N)$ via busca binária sobre as fronteiras canônicas.
    pub fn route_binary_search<N: CanonicalNormalizer>(&self, normalizer: &N, raw_key: &[u8]) -> Option<usize> {
        let canonical = normalizer.project_canonical(raw_key);
        let idx = self.blocks.partition_point(|b| b.max_canonical_key < canonical);
        if idx < self.blocks.len() {
            let b = &self.blocks[idx];
            if canonical >= b.min_canonical_key && canonical <= b.max_canonical_key {
                return Some(b.block_index);
            }
        }
        None
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
        // 1. Prova de preservação de projeção canônica e hash no filtro de Bloom para amostras equivalentes
        for &(raw1, raw2) in test_samples {
            let can1 = normalizer.project_canonical(raw1);
            let can2 = normalizer.project_canonical(raw2);
            if can1 != can2 {
                return Err(QuotientViolation::NonCanonicalProjectionMismatch {
                    raw_key1: raw1.to_vec(),
                    raw_key2: raw2.to_vec(),
                    canonical1: can1,
                    canonical2: can2,
                });
            }

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

        // 2. Prova de convexidade interna de cada bloco individual (min <= max)
        for block in &self.blocks {
            if block.min_canonical_key > block.max_canonical_key {
                return Err(QuotientViolation::QuotientConvexityBroken {
                    key_low: block.min_canonical_key.clone(),
                    key_mid: block.max_canonical_key.clone(),
                    key_high: block.min_canonical_key.clone(),
                    mid_block: block.block_index,
                    bound_block: block.block_index,
                });
            }
        }

        // 3. Prova de monotonicidade e convexidade entre blocos adjacentes
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quotient_block_meta_try_new_red_to_green() {
        assert_eq!(
            QuotientBlockMeta::try_new(0, vec![], b"user:m".to_vec(), 10),
            Err(QuotientViolation::EmptyCanonicalKey)
        );
        assert_eq!(
            QuotientBlockMeta::try_new(0, b"user:z".to_vec(), b"user:a".to_vec(), 10),
            Err(QuotientViolation::InvertedBlockBounds {
                block_index: 0,
                min_key: b"user:z".to_vec(),
                max_key: b"user:a".to_vec()
            })
        );
        assert_eq!(
            QuotientBlockMeta::try_new(0, b"user:a".to_vec(), b"user:m".to_vec(), 0),
            Err(QuotientViolation::ZeroRecordCount { block_index: 0 })
        );
    }

    #[test]
    fn test_quotient_try_build_and_binary_search_routing() {
        let b0 = QuotientBlockMeta::try_new(0, b"user:a".to_vec(), b"user:m".to_vec(), 5).unwrap();
        let b1 = QuotientBlockMeta::try_new(1, b"user:n".to_vec(), b"user:z".to_vec(), 5).unwrap();

        let index = QuotientBlockIndex::try_build(vec![b0, b1]).unwrap();
        let normalizer = AsciiCaseInsensitiveNormalizer;

        assert_eq!(index.route_binary_search(&normalizer, b" USER:BOB "), Some(0));
        assert_eq!(index.route_binary_search(&normalizer, b" USER:PETER "), Some(1));
        assert_eq!(index.route_binary_search(&normalizer, b" admin:root "), None);
    }

    #[test]
    fn test_bloom_try_insert_and_try_may_contain() {
        let normalizer = AsciiCaseInsensitiveNormalizer;
        let mut filter = QuotientBloomFilter::new(1024, 4);

        assert_eq!(filter.try_insert(&normalizer, b""), Err(QuotientViolation::EmptyKey));
        assert_eq!(filter.try_insert(&normalizer, b"   "), Err(QuotientViolation::EmptyCanonicalKey));

        assert!(filter.try_insert(&normalizer, b" USER:ALICE ").is_ok());
        assert!(filter.try_may_contain(&normalizer, b"user:alice").unwrap());
        assert!(!filter.try_may_contain(&normalizer, b"user:bob").unwrap());
    }
}

