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

        for block in blocks {
            let actual_crc = crc32c::crc32c(&block.raw_data);
            if actual_crc == block.expected_crc {
                healthy.push(block.block_index);
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
    pub fn evaluate_point_read<'a>(
        key: &[u8],
        blocks: &'a [SstDataBlockMeta],
        quarantined: &[BlockScrubStatus],
    ) -> Result<Option<&'a [u8]>, &'static str> {
        // 1. Verifica se a chave cai no raio de dano de algum bloco corrompido
        for status in quarantined {
            if let BlockScrubStatus::Corrupted { min_key, max_key, .. } = status {
                if key >= min_key.as_slice() && key <= max_key.as_slice() {
                    return Err("Key resides within physical bit-rot quarantine zone");
                }
            }
        }

        // 2. Busca nos blocos saudáveis
        for block in blocks {
            if key >= block.min_key.as_slice() && key <= block.max_key.as_slice() {
                let actual_crc = crc32c::crc32c(&block.raw_data);
                if actual_crc == block.expected_crc {
                    return Ok(Some(block.raw_data.as_slice()));
                }
            }
        }

        Ok(None)
    }

    /// Salva dados saudáveis durante compactação descartando cirurgicamente apenas o bloco corrompido.
    #[must_use]
    pub fn salvage_healthy_records_for_compaction(
        blocks: &[SstDataBlockMeta],
    ) -> Vec<SstDataBlockMeta> {
        blocks
            .iter()
            .filter(|b| crc32c::crc32c(&b.raw_data) == b.expected_crc)
            .cloned()
            .collect()
    }
}
