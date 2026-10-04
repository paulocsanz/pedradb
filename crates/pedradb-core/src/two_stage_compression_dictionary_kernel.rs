//! RFC-0291 Fronteira 9: Integridade de Dois Estágios sob Compressão com Dicionário Compartilhado.
//!
//! Elimina o risco de corrupção semântica silenciosa (onde dados são descomprimidos
//! com o dicionário errado gerando lixo não detectado), vinculando criptograficamente
//! o digest do dicionário ao bloco físico e validando integridade em dois estágios.

#![forbid(unsafe_code)]

/// Tamanho máximo de bloco comprimido ou descomprimido (64 MiB) para prevenção de DoS e OOM.
pub const MAX_BLOCK_PAYLOAD_SIZE: u32 = 64 * 1024 * 1024;

/// Descritor do dicionário de compressão pré-treinado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PretrainedDictionary {
    pub dictionary_id: u32,
    pub dictionary_digest: u64,
    pub raw_dict_bytes: Vec<u8>,
}

impl PretrainedDictionary {
    /// Constrói um dicionário validando ID não nulo e bytes não vazios.
    pub fn try_new(dictionary_id: u32, raw_dict_bytes: Vec<u8>) -> Result<Self, TwoStageIntegrityViolation> {
        if dictionary_id == 0 {
            return Err(TwoStageIntegrityViolation::ZeroDictionaryId);
        }
        if raw_dict_bytes.is_empty() {
            return Err(TwoStageIntegrityViolation::EmptyDictionary);
        }
        let digest = crc32c::crc32c(&raw_dict_bytes) as u64;
        Ok(Self {
            dictionary_id,
            dictionary_digest: digest,
            raw_dict_bytes,
        })
    }

    #[track_caller]
    pub fn new(dictionary_id: u32, raw_dict_bytes: Vec<u8>) -> Self {
        Self::try_new(dictionary_id, raw_dict_bytes).expect("valid dictionary parameters required")
    }
}

/// Cabeçalho de integridade física e lógica do bloco comprimido.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressedBlockHeader {
    pub uncompressed_len: u32,
    pub compressed_len: u32,
    pub dictionary_id: u32,
    pub dictionary_digest: u64,
    pub stage1_physical_crc: u32,
    pub stage2_logical_crc: u32,
}

impl CompressedBlockHeader {
    /// Tamanho fixo da representação em wire bytes do cabeçalho (36B).
    pub const HEADER_WIRE_SIZE: usize = 36;
    const HEADER_MAGIC: [u8; 4] = *b"2SDC";

    /// Serializa o cabeçalho em formato wire canônico de 36 bytes com magic e checksum.
    #[must_use]
    pub fn encode(&self) -> [u8; Self::HEADER_WIRE_SIZE] {
        let mut buf = [0u8; Self::HEADER_WIRE_SIZE];
        buf[0..4].copy_from_slice(&Self::HEADER_MAGIC);
        buf[4..8].copy_from_slice(&self.uncompressed_len.to_le_bytes());
        buf[8..12].copy_from_slice(&self.compressed_len.to_le_bytes());
        buf[12..16].copy_from_slice(&self.dictionary_id.to_le_bytes());
        buf[16..24].copy_from_slice(&self.dictionary_digest.to_le_bytes());
        buf[24..28].copy_from_slice(&self.stage1_physical_crc.to_le_bytes());
        buf[28..32].copy_from_slice(&self.stage2_logical_crc.to_le_bytes());
        let header_crc = crc32c::crc32c(&buf[0..32]);
        buf[32..36].copy_from_slice(&header_crc.to_le_bytes());
        buf
    }

    /// Desserializa e valida rigorosamente o cabeçalho a partir de bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, TwoStageIntegrityViolation> {
        if bytes.len() < Self::HEADER_WIRE_SIZE {
            return Err(TwoStageIntegrityViolation::InvalidWireFormat {
                reason: "Header slice truncated",
            });
        }
        if &bytes[0..4] != &Self::HEADER_MAGIC {
            return Err(TwoStageIntegrityViolation::InvalidWireFormat {
                reason: "Bad magic header marker",
            });
        }
        let expected_crc = u32::from_le_bytes(
            bytes[32..36]
                .try_into()
                .map_err(|_| TwoStageIntegrityViolation::InvalidWireFormat {
                    reason: "Invalid checksum bytes",
                })?,
        );
        let calc_crc = crc32c::crc32c(&bytes[0..32]);
        if expected_crc != calc_crc {
            return Err(TwoStageIntegrityViolation::InvalidWireFormat {
                reason: "Header checksum mismatch",
            });
        }

        let uncompressed_len = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let compressed_len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let dictionary_id = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let dictionary_digest = u64::from_le_bytes([
            bytes[16], bytes[17], bytes[18], bytes[19],
            bytes[20], bytes[21], bytes[22], bytes[23],
        ]);
        let stage1_physical_crc = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
        let stage2_logical_crc = u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);

        if dictionary_id == 0 {
            return Err(TwoStageIntegrityViolation::ZeroDictionaryId);
        }

        if compressed_len == 0 && uncompressed_len == 0 {
            return Err(TwoStageIntegrityViolation::EmptyBlockPayload);
        }

        if (compressed_len > 0 && uncompressed_len == 0) || (compressed_len == 0 && uncompressed_len > 0) {
            return Err(TwoStageIntegrityViolation::InvalidWireFormat {
                reason: "Inconsistent compressed and uncompressed block lengths",
            });
        }

        if compressed_len > MAX_BLOCK_PAYLOAD_SIZE {
            return Err(TwoStageIntegrityViolation::BlockPayloadTooLarge {
                max: MAX_BLOCK_PAYLOAD_SIZE,
                actual: compressed_len,
            });
        }

        if uncompressed_len > MAX_BLOCK_PAYLOAD_SIZE {
            return Err(TwoStageIntegrityViolation::BlockPayloadTooLarge {
                max: MAX_BLOCK_PAYLOAD_SIZE,
                actual: uncompressed_len,
            });
        }

        Ok(Self {
            uncompressed_len,
            compressed_len,
            dictionary_id,
            dictionary_digest,
            stage1_physical_crc,
            stage2_logical_crc,
        })
    }
}

/// Violações de integridade em descompressão com dicionário.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TwoStageIntegrityViolation {
    Stage1PhysicalCrcMismatch {
        expected: u32,
        calculated: u32,
    },
    DictionaryBindingMismatch {
        block_dict_id: u32,
        block_dict_digest: u64,
        active_dict_id: u32,
        active_dict_digest: u64,
    },
    Stage2LogicalCrcMismatch {
        expected: u32,
        calculated: u32,
    },
    CompressedLengthMismatch {
        expected: u32,
        actual: u32,
    },
    UncompressedLengthMismatch {
        expected: u32,
        actual: u32,
    },
    ZeroDictionaryId,
    EmptyDictionary,
    EmptyBlockPayload,
    BlockPayloadTooLarge {
        max: u32,
        actual: u32,
    },
    InvalidWireFormat {
        reason: &'static str,
    },
}

impl std::fmt::Display for TwoStageIntegrityViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stage1PhysicalCrcMismatch { expected, calculated } => {
                write!(
                    f,
                    "stage 1 physical crc mismatch: expected {expected:#x}, got {calculated:#x}"
                )
            }
            Self::DictionaryBindingMismatch {
                block_dict_id,
                block_dict_digest,
                active_dict_id,
                active_dict_digest,
            } => {
                write!(
                    f,
                    "dictionary binding mismatch: block ({block_dict_id}, {block_dict_digest:#x}) vs active ({active_dict_id}, {active_dict_digest:#x})"
                )
            }
            Self::Stage2LogicalCrcMismatch { expected, calculated } => {
                write!(
                    f,
                    "stage 2 logical crc mismatch: expected {expected:#x}, got {calculated:#x}"
                )
            }
            Self::CompressedLengthMismatch { expected, actual } => {
                write!(
                    f,
                    "compressed length mismatch: expected {expected}, actual {actual}"
                )
            }
            Self::UncompressedLengthMismatch { expected, actual } => {
                write!(
                    f,
                    "uncompressed length mismatch: expected {expected}, actual {actual}"
                )
            }
            Self::ZeroDictionaryId => write!(f, "dictionary id cannot be zero"),
            Self::EmptyDictionary => write!(f, "dictionary payload cannot be empty"),
            Self::EmptyBlockPayload => write!(f, "block payload cannot be empty"),
            Self::BlockPayloadTooLarge { max, actual } => {
                write!(
                    f,
                    "block payload size {actual} exceeds maximum allowed {max}"
                )
            }
            Self::InvalidWireFormat { reason } => write!(f, "invalid wire format: {reason}"),
        }
    }
}

impl std::error::Error for TwoStageIntegrityViolation {}

/// Mecanismo de empacotamento e descompressão com integridade de dois estágios.
pub struct TwoStageBlockCodec;

impl TwoStageBlockCodec {
    /// Empacota um bloco comprimido gerando os checksums de dois estágios e a amarração de dicionário com validações.
    pub fn try_encode_block(
        uncompressed_payload: &[u8],
        compressed_payload: &[u8],
        dictionary: &PretrainedDictionary,
    ) -> Result<(CompressedBlockHeader, Vec<u8>), TwoStageIntegrityViolation> {
        if uncompressed_payload.is_empty() || compressed_payload.is_empty() {
            return Err(TwoStageIntegrityViolation::EmptyBlockPayload);
        }
        if dictionary.dictionary_id == 0 {
            return Err(TwoStageIntegrityViolation::ZeroDictionaryId);
        }
        let uncompressed_len = u32::try_from(uncompressed_payload.len()).map_err(|_| {
            TwoStageIntegrityViolation::BlockPayloadTooLarge {
                max: MAX_BLOCK_PAYLOAD_SIZE,
                actual: u32::MAX,
            }
        })?;
        let compressed_len = u32::try_from(compressed_payload.len()).map_err(|_| {
            TwoStageIntegrityViolation::BlockPayloadTooLarge {
                max: MAX_BLOCK_PAYLOAD_SIZE,
                actual: u32::MAX,
            }
        })?;
        if uncompressed_len > MAX_BLOCK_PAYLOAD_SIZE {
            return Err(TwoStageIntegrityViolation::BlockPayloadTooLarge {
                max: MAX_BLOCK_PAYLOAD_SIZE,
                actual: uncompressed_len,
            });
        }
        if compressed_len > MAX_BLOCK_PAYLOAD_SIZE {
            return Err(TwoStageIntegrityViolation::BlockPayloadTooLarge {
                max: MAX_BLOCK_PAYLOAD_SIZE,
                actual: compressed_len,
            });
        }

        let header = CompressedBlockHeader {
            uncompressed_len,
            compressed_len,
            dictionary_id: dictionary.dictionary_id,
            dictionary_digest: dictionary.dictionary_digest,
            stage1_physical_crc: crc32c::crc32c(compressed_payload),
            stage2_logical_crc: crc32c::crc32c(uncompressed_payload),
        };

        Ok((header, compressed_payload.to_vec()))
    }

    /// Empacota um bloco comprimido gerando os checksums de dois estágios e a amarração de dicionário.
    #[track_caller]
    pub fn encode_block(
        uncompressed_payload: &[u8],
        compressed_payload: &[u8],
        dictionary: &PretrainedDictionary,
    ) -> (CompressedBlockHeader, Vec<u8>) {
        Self::try_encode_block(uncompressed_payload, compressed_payload, dictionary)
            .expect("valid block payloads and dictionary required")
    }

    /// Desempacota e valida estritamente a integridade de dois estágios.
    pub fn decode_and_verify<F>(
        header: &CompressedBlockHeader,
        compressed_bytes: &[u8],
        active_dictionary: &PretrainedDictionary,
        decompress_fn: F,
    ) -> Result<Vec<u8>, TwoStageIntegrityViolation>
    where
        F: FnOnce(&[u8], &[u8]) -> Vec<u8>,
    {
        if header.dictionary_id == 0 {
            return Err(TwoStageIntegrityViolation::ZeroDictionaryId);
        }
        if header.compressed_len == 0 || header.uncompressed_len == 0 {
            return Err(TwoStageIntegrityViolation::EmptyBlockPayload);
        }
        if header.compressed_len > MAX_BLOCK_PAYLOAD_SIZE {
            return Err(TwoStageIntegrityViolation::BlockPayloadTooLarge {
                max: MAX_BLOCK_PAYLOAD_SIZE,
                actual: header.compressed_len,
            });
        }
        if header.uncompressed_len > MAX_BLOCK_PAYLOAD_SIZE {
            return Err(TwoStageIntegrityViolation::BlockPayloadTooLarge {
                max: MAX_BLOCK_PAYLOAD_SIZE,
                actual: header.uncompressed_len,
            });
        }

        // 0. Validação de tamanho físico comprimido
        let actual_comp_len = u32::try_from(compressed_bytes.len()).unwrap_or(u32::MAX);
        if compressed_bytes.len() != header.compressed_len as usize {
            return Err(TwoStageIntegrityViolation::CompressedLengthMismatch {
                expected: header.compressed_len,
                actual: actual_comp_len,
            });
        }

        // 1. Estágio 1: Integridade física do payload comprimido antes de tocar qualquer parser
        let calc_stage1 = crc32c::crc32c(compressed_bytes);
        if calc_stage1 != header.stage1_physical_crc {
            return Err(TwoStageIntegrityViolation::Stage1PhysicalCrcMismatch {
                expected: header.stage1_physical_crc,
                calculated: calc_stage1,
            });
        }

        // 2. Amarração Criptográfica do Dicionário: impede descompressão cega com dicionário incompatível
        if header.dictionary_id != active_dictionary.dictionary_id
            || header.dictionary_digest != active_dictionary.dictionary_digest
        {
            return Err(TwoStageIntegrityViolation::DictionaryBindingMismatch {
                block_dict_id: header.dictionary_id,
                block_dict_digest: header.dictionary_digest,
                active_dict_id: active_dictionary.dictionary_id,
                active_dict_digest: active_dictionary.dictionary_digest,
            });
        }

        // 3. Execução da descompressão
        let uncompressed = decompress_fn(compressed_bytes, &active_dictionary.raw_dict_bytes);

        // 4. Validação de tamanho do payload descomprimido
        let actual_uncomp_len = u32::try_from(uncompressed.len()).unwrap_or(u32::MAX);
        if uncompressed.len() != header.uncompressed_len as usize {
            return Err(TwoStageIntegrityViolation::UncompressedLengthMismatch {
                expected: header.uncompressed_len,
                actual: actual_uncomp_len,
            });
        }

        // 5. Estágio 2: Integridade lógica do payload reconstituído
        let calc_stage2 = crc32c::crc32c(&uncompressed);
        if calc_stage2 != header.stage2_logical_crc {
            return Err(TwoStageIntegrityViolation::Stage2LogicalCrcMismatch {
                expected: header.stage2_logical_crc,
                calculated: calc_stage2,
            });
        }

        Ok(uncompressed)
    }
}
