//! RFC-0291 Fronteira 10: Refinamento de Confinamento de Falhas Físicas contra Bit-Rot em Background Scrubbing.
//!
//! Garante que a detecção de bit-rot ou corrupção parcial de setor em um bloco
//! físico seja estritamente confinada ao intervalo de chaves daquele bloco, preservando
//! 100% da integridade e capacidade de leitura/compactação dos blocos ortogonais saudáveis.

#![forbid(unsafe_code)]

/// Descritor de bloco de dados em uma SST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstDataBlockMeta {
    pub block_index: usize,
    pub min_key: Vec<u8>,
    pub max_key: Vec<u8>,
    pub expected_crc: u32,
    pub raw_data: Vec<u8>,
}

impl SstDataBlockMeta {
    /// Safely constructs SstDataBlockMeta, validating non-empty keys, non-inverted bounds, and non-empty raw payload.
    pub fn try_new(
        block_index: usize,
        min_key: Vec<u8>,
        max_key: Vec<u8>,
        raw_data: Vec<u8>,
    ) -> Result<Self, FaultIsolationError> {
        if min_key.is_empty() || max_key.is_empty() {
            return Err(FaultIsolationError::EmptyBoundaryKey { block_index });
        }
        if min_key > max_key {
            return Err(FaultIsolationError::InvertedBlockBounds {
                block_index,
                min_key,
                max_key,
            });
        }
        if raw_data.is_empty() {
            return Err(FaultIsolationError::ZeroBlockData { block_index });
        }
        let expected_crc = crc32c::crc32c(&raw_data);
        Ok(Self {
            block_index,
            min_key,
            max_key,
            expected_crc,
            raw_data,
        })
    }
}

/// Estado de integridade de um bloco verificado por scrubbing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockScrubStatus {
    Healthy {
        block_index: usize,
        record_count: usize,
    },
    Corrupted {
        block_index: usize,
        min_key: Vec<u8>,
        max_key: Vec<u8>,
        expected_crc: u32,
        calculated_crc: u32,
    },
}

/// Erros estruturais e de integridade em isolamento de falha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultIsolationError {
    KeyInQuarantineZone {
        key: Vec<u8>,
        quarantine_min: Vec<u8>,
        quarantine_max: Vec<u8>,
    },
    UnquarantinedBitrotDetected {
        key: Vec<u8>,
        block_index: usize,
        expected_crc: u32,
        actual_crc: u32,
    },
    EmptyQueryKey,
    ZeroBlockData {
        block_index: usize,
    },
    InvertedBlockBounds {
        block_index: usize,
        min_key: Vec<u8>,
        max_key: Vec<u8>,
    },
    EmptyBoundaryKey {
        block_index: usize,
    },
}

impl std::fmt::Display for FaultIsolationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KeyInQuarantineZone { key, quarantine_min, quarantine_max } => write!(
                f,
                "Key {key:?} resides within physical bit-rot quarantine zone [{quarantine_min:?}, {quarantine_max:?}]"
            ),
            Self::UnquarantinedBitrotDetected { key, block_index, expected_crc, actual_crc } => write!(
                f,
                "Unquarantined physical bit-rot detected in block {block_index} covering key {key:?}: expected CRC {expected_crc:#x}, found {actual_crc:#x}"
            ),
            Self::EmptyQueryKey => write!(f, "Query key cannot be empty"),
            Self::ZeroBlockData { block_index } => write!(f, "Block {block_index} has zero raw payload bytes"),
            Self::InvertedBlockBounds { block_index, min_key, max_key } => write!(
                f,
                "Block {block_index} has inverted key bounds: min {min_key:?} > max {max_key:?}"
            ),
            Self::EmptyBoundaryKey { block_index } => write!(f, "Block {block_index} has empty boundary key"),
        }
    }
}

impl std::error::Error for FaultIsolationError {}

/// Resultado da auditoria de isolamento de falha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultIsolationReport {
    pub total_blocks: usize,
    pub healthy_blocks: Vec<usize>,
    pub quarantined_blocks: Vec<BlockScrubStatus>,
    pub is_orthogonal_data_preserved: bool,
}

/// Oráculo de isolamento e confinamento cirúrgico de bit-rot.
pub struct FaultIsolationOracle;

impl FaultIsolationOracle {
    /// Executa scrubbing sobre a lista de blocos da SST, isolando blocos corrompidos
    /// sem propagação de erro para os blocos saudáveis.
    #[must_use]
    pub fn scrub_sst_blocks(blocks: &[SstDataBlockMeta]) -> FaultIsolationReport {
        let mut healthy = Vec::new();
        let mut quarantined = Vec::new();
        let mut prev_max_key: Option<&[u8]> = None;

        for block in blocks {
            if block.min_key > block.max_key {
                quarantined.push(BlockScrubStatus::Corrupted {
                    block_index: block.block_index,
                    min_key: block.min_key.clone(),
                    max_key: block.max_key.clone(),
                    expected_crc: block.expected_crc,
                    calculated_crc: 0,
                });
                continue;
            }

            // Invariante estrutural de SST: blocos adjacentes não podem ter sobreposição de chaves
            if let Some(prev_max) = prev_max_key {
                if prev_max >= block.min_key.as_slice() {
                    quarantined.push(BlockScrubStatus::Corrupted {
                        block_index: block.block_index,
                        min_key: block.min_key.clone(),
                        max_key: block.max_key.clone(),
                        expected_crc: block.expected_crc,
                        calculated_crc: 0,
                    });
                    continue;
                }
            }

            let actual_crc = crc32c::crc32c(&block.raw_data);
            if actual_crc == block.expected_crc {
                healthy.push(block.block_index);
                prev_max_key = Some(&block.max_key);
            } else {
                quarantined.push(BlockScrubStatus::Corrupted {
                    block_index: block.block_index,
                    min_key: block.min_key.clone(),
                    max_key: block.max_key.clone(),
                    expected_crc: block.expected_crc,
                    calculated_crc: actual_crc,
                });
            }
        }

        let preserved = !healthy.is_empty() || blocks.is_empty();

        FaultIsolationReport {
            total_blocks: blocks.len(),
            healthy_blocks: healthy,
            quarantined_blocks: quarantined,
            is_orthogonal_data_preserved: preserved,
        }
    }

    /// Avalia uma leitura pontual sob a presença de blocos em quarentena.
    ///
    /// Se a chave estiver fora do intervalo do bloco corrompido, a leitura é segura e não afetada.
    /// Se o bloco cobrindo a chave apresentar falha de integridade, a leitura falha fechada com erro (fail-closed).
    pub fn evaluate_point_read<'a>(
        key: &[u8],
        blocks: &'a [SstDataBlockMeta],
        quarantined: &[BlockScrubStatus],
    ) -> Result<Option<&'a [u8]>, FaultIsolationError> {
        if key.is_empty() {
            return Err(FaultIsolationError::EmptyQueryKey);
        }

        // 1. Verifica se a chave cai no raio de dano de algum bloco corrompido
        for status in quarantined {
            if let BlockScrubStatus::Corrupted { min_key, max_key, .. } = status {
                if key >= min_key.as_slice() && key <= max_key.as_slice() {
                    return Err(FaultIsolationError::KeyInQuarantineZone {
                        key: key.to_vec(),
                        quarantine_min: min_key.clone(),
                        quarantine_max: max_key.clone(),
                    });
                }
            }
        }

        // 2. Busca nos blocos saudáveis
        for block in blocks {
            if key >= block.min_key.as_slice() && key <= block.max_key.as_slice() {
                let actual_crc = crc32c::crc32c(&block.raw_data);
                if actual_crc != block.expected_crc {
                    return Err(FaultIsolationError::UnquarantinedBitrotDetected {
                        key: key.to_vec(),
                        block_index: block.block_index,
                        expected_crc: block.expected_crc,
                        actual_crc,
                    });
                }
                return Ok(Some(block.raw_data.as_slice()));
            }
        }

        Ok(None)
    }

    /// Salva dados saudáveis durante compactação descartando cirurgicamente apenas o bloco corrompido.
    #[must_use]
    pub fn salvage_healthy_records_for_compaction(
        blocks: &[SstDataBlockMeta],
    ) -> Vec<SstDataBlockMeta> {
        let mut salvaged: Vec<SstDataBlockMeta> = Vec::new();
        for b in blocks {
            if b.min_key <= b.max_key && crc32c::crc32c(&b.raw_data) == b.expected_crc {
                if let Some(prev) = salvaged.last() {
                    if prev.max_key >= b.min_key {
                        continue;
                    }
                }
                salvaged.push(b.clone());
            }
        }
        salvaged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sst_data_block_meta_try_new_red_to_green() {
        assert_eq!(
            SstDataBlockMeta::try_new(0, vec![], b"k10".to_vec(), b"data".to_vec()),
            Err(FaultIsolationError::EmptyBoundaryKey { block_index: 0 })
        );
        assert_eq!(
            SstDataBlockMeta::try_new(0, b"k20".to_vec(), b"k10".to_vec(), b"data".to_vec()),
            Err(FaultIsolationError::InvertedBlockBounds {
                block_index: 0,
                min_key: b"k20".to_vec(),
                max_key: b"k10".to_vec(),
            })
        );
        assert_eq!(
            SstDataBlockMeta::try_new(0, b"k00".to_vec(), b"k10".to_vec(), vec![]),
            Err(FaultIsolationError::ZeroBlockData { block_index: 0 })
        );
        let block = SstDataBlockMeta::try_new(0, b"k00".to_vec(), b"k10".to_vec(), b"valid".to_vec()).unwrap();
        assert_eq!(block.expected_crc, crc32c::crc32c(b"valid"));
    }

    #[test]
    fn test_evaluate_point_read_empty_key_red_to_green() {
        let block = SstDataBlockMeta::try_new(0, b"k00".to_vec(), b"k10".to_vec(), b"valid".to_vec()).unwrap();
        assert_eq!(
            FaultIsolationOracle::evaluate_point_read(b"", &[block], &[]),
            Err(FaultIsolationError::EmptyQueryKey)
        );
    }
}

