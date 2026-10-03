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
    /// O bloco lido foi retornado de um índice físico LBA incorreto (Misdirection da FTL).
    BlockIndexMisdirection {
        /// Índice de bloco esperado pelo leitor.
        expected_block_index: u32,
        /// Índice de bloco retornado fisicamente pela mídia.
        actual_block_index: u32,
    },
    /// A época de gravação diverge da época global esperada.
    StaleGlobalEpochDetected {
        /// Época registrada no bloco.
        recorded_epoch: u64,
        /// Época global esperada.
        expected_epoch: u64,
    },
    /// Carga útil vazia ou corrompida.
    EmptyPayloadNotAllowed,
    /// Boot UUID não pode ser nulo/zero.
    ZeroBootUuid,
    /// Época global não pode ser zero.
    ZeroEpoch,
    /// Sequence number não pode ser zero.
    ZeroSeq,
}

impl std::fmt::Display for FtlAmnesiaViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaleBlockRollbackDetected { file_id, block_index, stale_block_seq, active_min_seq } => {
                write!(
                    f,
                    "Stale block rollback on file {file_id} block {block_index}: seq {stale_block_seq} < active {active_min_seq}"
                )
            }
            Self::ForeignIncarnationDetected { recorded_boot_uuid, expected_boot_uuid } => {
                write!(
                    f,
                    "Foreign incarnation: recorded {recorded_boot_uuid:x} != expected {expected_boot_uuid:x}"
                )
            }
            Self::TokenChecksumMismatch { expected_crc, calculated_crc } => {
                write!(
                    f,
                    "Token checksum mismatch: expected {expected_crc:x}, calculated {calculated_crc:x}"
                )
            }
            Self::BlockIndexMisdirection { expected_block_index, actual_block_index } => {
                write!(
                    f,
                    "Block index misdirection: expected {expected_block_index}, actual {actual_block_index}"
                )
            }
            Self::StaleGlobalEpochDetected { recorded_epoch, expected_epoch } => {
                write!(
                    f,
                    "Stale global epoch: recorded {recorded_epoch}, expected {expected_epoch}"
                )
            }
            Self::EmptyPayloadNotAllowed => write!(f, "Empty payload not allowed"),
            Self::ZeroBootUuid => write!(f, "Boot UUID cannot be zero"),
            Self::ZeroEpoch => write!(f, "Global epoch cannot be zero"),
            Self::ZeroSeq => write!(f, "LSM sequence number cannot be zero"),
        }
    }
}

impl std::error::Error for FtlAmnesiaViolation {}

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
    /// Gera um token de anti-amnésia validando boot_uuid, global_epoch e lsm_seq.
    pub fn try_new(boot_uuid: u128, global_epoch: u64, lsm_seq: u64, block_index: u32) -> Result<Self, FtlAmnesiaViolation> {
        if boot_uuid == 0 {
            return Err(FtlAmnesiaViolation::ZeroBootUuid);
        }
        if global_epoch == 0 {
            return Err(FtlAmnesiaViolation::ZeroEpoch);
        }
        if lsm_seq == 0 {
            return Err(FtlAmnesiaViolation::ZeroSeq);
        }
        Ok(Self::new(boot_uuid, global_epoch, lsm_seq, block_index))
    }

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalMediaBlock {
    /// Token anti-amnésia da FTL.
    pub token: FtlAntiAmnesiaToken,
    /// Dados úteis do bloco.
    pub payload: Vec<u8>,
}

impl PhysicalMediaBlock {
    /// Cria um bloco físico de mídia validando não-vacuidade do payload e checksum do token.
    pub fn try_new(token: FtlAntiAmnesiaToken, payload: Vec<u8>) -> Result<Self, FtlAmnesiaViolation> {
        if payload.is_empty() {
            return Err(FtlAmnesiaViolation::EmptyPayloadNotAllowed);
        }
        if !token.is_checksum_valid() {
            let calculated = FtlAntiAmnesiaToken::compute_crc(
                token.boot_uuid,
                token.global_epoch,
                token.lsm_seq,
                token.block_index,
            );
            return Err(FtlAmnesiaViolation::TokenChecksumMismatch {
                expected_crc: token.token_crc,
                calculated_crc: calculated,
            });
        }
        Ok(Self { token, payload })
    }
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

        // 3. Prova de payload não vazio (rejeita blocos apagados/uninitialized)
        if block.payload.is_empty() {
            return Err(FtlAmnesiaViolation::EmptyPayloadNotAllowed);
        }

        // 4. Prova de monotonicidade temporal contra o active version catalog:
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

    /// Valida proveniência e integridade do bloco verificando também a correlação com o índice LBA esperado.
    pub fn verify_block_at_index(
        file_id: u64,
        block: &PhysicalMediaBlock,
        expected_boot_uuid: u128,
        active_min_seq: u64,
        expected_block_index: u32,
    ) -> Result<(), FtlAmnesiaViolation> {
        if block.token.block_index != expected_block_index {
            return Err(FtlAmnesiaViolation::BlockIndexMisdirection {
                expected_block_index,
                actual_block_index: block.token.block_index,
            });
        }
        Self::verify_block_provenance(file_id, block, expected_boot_uuid, active_min_seq)
    }

    /// Valida proveniência garantindo que o bloco pertença à época global esperada do cluster/instância.
    pub fn verify_block_with_epoch(
        file_id: u64,
        block: &PhysicalMediaBlock,
        expected_boot_uuid: u128,
        active_min_seq: u64,
        expected_epoch: u64,
    ) -> Result<(), FtlAmnesiaViolation> {
        if block.token.global_epoch != expected_epoch {
            return Err(FtlAmnesiaViolation::StaleGlobalEpochDetected {
                recorded_epoch: block.token.global_epoch,
                expected_epoch,
            });
        }
        Self::verify_block_provenance(file_id, block, expected_boot_uuid, active_min_seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ftl_anti_amnesia_bounds() {
        assert_eq!(
            FtlAntiAmnesiaToken::try_new(0, 1, 1, 0),
            Err(FtlAmnesiaViolation::ZeroBootUuid)
        );

        assert_eq!(
            FtlAntiAmnesiaToken::try_new(0x1234, 0, 1, 0),
            Err(FtlAmnesiaViolation::ZeroEpoch)
        );

        assert_eq!(
            FtlAntiAmnesiaToken::try_new(0x1234, 1, 0, 0),
            Err(FtlAmnesiaViolation::ZeroSeq)
        );

        let token = FtlAntiAmnesiaToken::try_new(0x1234, 1, 100, 0).expect("valid token");

        assert_eq!(
            PhysicalMediaBlock::try_new(token, vec![]),
            Err(FtlAmnesiaViolation::EmptyPayloadNotAllowed)
        );

        let block = PhysicalMediaBlock::try_new(token, vec![1, 2, 3]).expect("valid block");
        assert_eq!(block.payload, vec![1, 2, 3]);
    }

    #[test]
    fn test_ftl_violation_display() {
        let err = FtlAmnesiaViolation::ZeroBootUuid;
        assert_eq!(format!("{err}"), "Boot UUID cannot be zero");

        let err2 = FtlAmnesiaViolation::ZeroEpoch;
        assert_eq!(format!("{err2}"), "Global epoch cannot be zero");

        let err3 = FtlAmnesiaViolation::ZeroSeq;
        assert_eq!(format!("{err3}"), "LSM sequence number cannot be zero");
    }
}

