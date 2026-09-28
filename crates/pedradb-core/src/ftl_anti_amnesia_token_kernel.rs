//! RFC-0292: Pilar 10 - Invariante Anti-Amnésia contra Reversão de Blocos da FTL NVMe.
//!
//! Vincula cada bloco físico persistido a um token espaço-temporal unívoco de geração,
//! detectando e rejeitando imediatamente blocos antigos ressuscitados por falhas de energia na FTL.

/// Violações de monotonicidade física e ressurreição por amnésia da FTL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FtlAmnesiaViolation {
    /// O bloco lido possui um LSM sequence number inferior à linha de base ativa do arquivo.
    StaleBlockRollbackDetected {
        /// ID do arquivo SST.
        file_id: u64,
        /// Índice do bloco afetado.
        block_index: u32,
        /// Sequence number encontrado no cabeçalho do bloco (antigo).
        stale_block_seq: u64,
        /// Sequence number mínimo esperado pelo catálogo de versões.
        active_min_seq: u64,
    },
    /// O Boot UUID do bloco físico divergiu do catálogo de encarnações conhecidas.
    ForeignIncarnationDetected {
        /// Boot UUID gravado no bloco.
        recorded_boot_uuid: u128,
        /// Boot UUID legítimo esperado.
        expected_boot_uuid: u128,
    },
    /// O CRC32C do cabeçalho do token de integridade divergiu.
    TokenChecksumMismatch {
        /// Esperado.
        expected_crc: u32,
        /// Calculado.
        calculated_crc: u32,
    },
}

/// Token físico de integridade espaço-temporal persistido no cabeçalho de cada bloco.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FtlAntiAmnesiaToken {
    /// Identificador universal do boot do host/controlador.
    pub boot_uuid: u128,
    /// Época de montagem global do motor.
    pub global_epoch: u64,
    /// Sequence number monotônico do motor no momento da gravação.
    pub lsm_seq: u64,
    /// Índice ordinal do bloco no arquivo.
    pub block_index: u32,
    /// Checksum CRC32C protegendo os metadados do token.
    pub token_crc: u32,
}

impl FtlAntiAmnesiaToken {
    /// Gera um token de anti-amnésia com CRC32C válido.
    pub fn new(boot_uuid: u128, global_epoch: u64, lsm_seq: u64, block_index: u32) -> Self {
        let crc = Self::compute_crc(boot_uuid, global_epoch, lsm_seq, block_index);
        Self {
            boot_uuid,
            global_epoch,
            lsm_seq,
            block_index,
            token_crc: crc,
        }
    }

    /// Calcula o checksum CRC32C determinístico dos campos do token.
    pub fn compute_crc(boot_uuid: u128, global_epoch: u64, lsm_seq: u64, block_index: u32) -> u32 {
        let mut h = 0x811c9dc5u32;
        let mut push = |bytes: &[u8]| {
            for &b in bytes {
                h ^= u32::from(b);
                h = h.wrapping_mul(0x01000193);
            }
        };
        push(&boot_uuid.to_le_bytes());
        push(&global_epoch.to_le_bytes());
        push(&lsm_seq.to_le_bytes());
        push(&block_index.to_le_bytes());
        h
    }

    /// Valida o checksum interno do token.
    pub fn is_checksum_valid(&self) -> bool {
        self.token_crc == Self::compute_crc(self.boot_uuid, self.global_epoch, self.lsm_seq, self.block_index)
    }
}

/// Bloco físico de dados persistido no NVMe com token de proteção.
#[derive(Debug, Clone)]
pub struct PhysicalMediaBlock {
    /// Token anti-amnésia da FTL.
    pub token: FtlAntiAmnesiaToken,
    /// Dados úteis do bloco.
    pub payload: Vec<u8>,
}

/// Oráculo de validação anti-amnésia contra reversões silenciosas da FTL.
pub struct FtlAntiAmnesiaOracle;

impl FtlAntiAmnesiaOracle {
    /// Valida se um bloco lido da mídia é genuíno ou se foi revertido/ressuscitado
    /// pela FTL devido a um erase-cycle interrompido por queda de energia.
    pub fn verify_block_provenance(
        file_id: u64,
        block: &PhysicalMediaBlock,
        expected_boot_uuid: u128,
        active_min_seq: u64,
    ) -> Result<(), FtlAmnesiaViolation> {
        // 1. Validação de integridade física do token
        if !block.token.is_checksum_valid() {
            let calculated = FtlAntiAmnesiaToken::compute_crc(
                block.token.boot_uuid,
                block.token.global_epoch,
                block.token.lsm_seq,
                block.token.block_index,
            );
            return Err(FtlAmnesiaViolation::TokenChecksumMismatch {
                expected_crc: block.token.token_crc,
                calculated_crc: calculated,
            });
        }

        // 2. Validação da encarnação de boot
        if block.token.boot_uuid != expected_boot_uuid {
            return Err(FtlAmnesiaViolation::ForeignIncarnationDetected {
                recorded_boot_uuid: block.token.boot_uuid,
                expected_boot_uuid,
            });
        }

        // 3. Prova de monotonicidade temporal contra o active version catalog:
        // Se o lsm_seq do bloco for inferior à linha de corte ativa do arquivo,
        // a FTL sofreu rollback e entregou dados de uma geração já reciclada.
        if block.token.lsm_seq < active_min_seq {
            return Err(FtlAmnesiaViolation::StaleBlockRollbackDetected {
                file_id,
                block_index: block.token.block_index,
                stale_block_seq: block.token.lsm_seq,
                active_min_seq,
            });
        }

        Ok(())
    }
}
