//! RFC-0291 Fronteira 9: Integridade de Dois Estágios sob Compressão com Dicionário Compartilhado.
//!
//! Elimina o risco de corrupção semântica silenciosa (onde dados são descomprimidos
//! com o dicionário errado gerando lixo não detectado), vinculando criptograficamente
//! o digest do dicionário ao bloco físico e validando integridade em dois estágios.

#![forbid(unsafe_code)]

/// Descritor do dicionário de compressão pré-treinado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PretrainedDictionary {
    pub dictionary_id: u32,
    pub dictionary_digest: u64,
    pub raw_dict_bytes: Vec<u8>,
}

impl PretrainedDictionary {
    pub fn new(dictionary_id: u32, raw_dict_bytes: Vec<u8>) -> Self {
        let digest = crc32c::crc32c(&raw_dict_bytes) as u64;
        Self {
            dictionary_id,
            dictionary_digest: digest,
            raw_dict_bytes,
        }
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
}

/// Mecanismo de empacotamento e descompressão com integridade de dois estágios.
pub struct TwoStageBlockCodec;

impl TwoStageBlockCodec {
    /// Empacota um bloco comprimido gerando os checksums de dois estágios e a amarração de dicionário.
    pub fn encode_block(
        uncompressed_payload: &[u8],
        compressed_payload: &[u8],
        dictionary: &PretrainedDictionary,
    ) -> (CompressedBlockHeader, Vec<u8>) {
        let header = CompressedBlockHeader {
            uncompressed_len: uncompressed_payload.len() as u32,
            compressed_len: compressed_payload.len() as u32,
            dictionary_id: dictionary.dictionary_id,
            dictionary_digest: dictionary.dictionary_digest,
            stage1_physical_crc: crc32c::crc32c(compressed_payload),
            stage2_logical_crc: crc32c::crc32c(uncompressed_payload),
        };

        (header, compressed_payload.to_vec())
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

        // 4. Estágio 2: Integridade lógica do payload reconstituído
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
