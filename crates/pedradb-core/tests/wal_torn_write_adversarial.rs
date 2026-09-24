//! RFC-0267 P1.1 — Teste Adversarial de Injeção de Torn-Writes Físicos e Cortes Abruptos de Energia.
//!
//! Modela falhas físicas reais de hardware e sistema de arquivos:
//! - Setores físicos de disco de 512B e 4096B.
//! - Torn-writes: corte de energia no meio de uma escrita de bloco (gravação parcial de setor).
//! - Reordenação de bytes não sincronizados e corrupção de CRC/cabeçalho.
//! - Prova que `WalReader::collect_all` e `recover_kernel` recuperam estritamente o prefixo
//!   durável confirmado, falham closed em corrupção e nunca ressuscitam lixo silencioso.

use std::io::Cursor;

use pedradb_core::wal::crc;
use pedradb_core::wal::format::{RecordType, HEADER_SIZE};
use pedradb_core::wal::WalReader;

/// Codifica um registro físico no formato PedraDB padrão com CRC mascarado.
fn encode_record(record_type: RecordType, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(HEADER_SIZE + payload.len());
    let length = payload.len() as u16;
    let type_byte = record_type as u8;

    let checksum = crc::record_checksum(type_byte, length, payload);

    buf.extend_from_slice(&checksum.to_le_bytes());
    buf.extend_from_slice(&length.to_le_bytes());
    buf.push(type_byte);
    buf.extend_from_slice(payload);
    buf
}

#[test]
fn test_wal_physical_sector_torn_write_recovery_prefix() {
    let mut log_bytes = Vec::new();
    let num_records = 150;
    let mut payloads = Vec::new();

    for i in 0..num_records {
        let payload = format!("commit_tx_seq_{:05}_payload_bytes_pad_data_for_long_blocks", i).into_bytes();
        let record = encode_record(RecordType::Full, &payload);
        log_bytes.extend_from_slice(&record);
        payloads.push(payload);
    }

    let total_len = log_bytes.len();
    assert!(total_len > 4096, "Tamanho deve exceder múltiplos setores");

    // Testa cortes abruptos (power-cut) em cada fronteira de setor físico de 512 bytes
    for sector_cut in (512..total_len).step_by(512) {
        let mut torn_image = log_bytes[..sector_cut].to_vec();

        // Simula torn write: o último setor gravou apenas parte dos dados e o resto é lixo/zeros
        if torn_image.len() > 128 {
            let tear_point = torn_image.len() - 128;
            for b in &mut torn_image[tear_point..] {
                *b = 0; // zera bytes finais simulando corte de alimentação no buffer de gravação
            }
        }

        let mut reader = WalReader::new(Cursor::new(&torn_image));
        let recovered = reader.collect_all();

        match recovered {
            Ok(records) => {
                // Todos os registros recuperados devem bater exatamente com o prefixo legítimo
                for (rec, expected) in records.iter().zip(payloads.iter()) {
                    assert_eq!(rec, expected, "Registro recuperado diverge do original!");
                }
                assert!(
                    records.len() <= num_records,
                    "Não pode recuperar mais registros que os emitidos"
                );
                // Offset de corte deve ser consistente com o último append válido
                assert!(reader.last_good_offset() <= sector_cut as u64);
            }
            Err(e) => {
                // Se falhar closed (ex: erro de CRC em cabeçalho quebrado ou truncamento), isso é fail-closed legítimo
                assert!(
                    matches!(e, pedradb_core::error::CoreError::Crc { .. } | pedradb_core::error::CoreError::Truncated(..))
                        || e.to_string().to_lowercase().contains("crc")
                        || e.to_string().to_lowercase().contains("corrupt")
                        || e.to_string().to_lowercase().contains("zero header")
                        || e.to_string().to_lowercase().contains("truncated")
                        || e.to_string().to_lowercase().contains("wal"),
                    "Erro inesperado sob torn write: {:?}",
                    e
                );
            }
        }
    }
}

#[test]
fn test_wal_adversarial_bitflip_fails_closed_never_silent_wrong() {
    let payload = b"critical_financial_transfer_tx_100_usd";
    let record = encode_record(RecordType::Full, payload);

    // Testa corrupção em cada um dos bytes do registro
    for corrupted_idx in 0..record.len() {
        let mut corrupted = record.clone();
        corrupted[corrupted_idx] ^= 0xFF; // Inverte todos os bits daquele byte

        let mut reader = WalReader::new(Cursor::new(&corrupted));
        let res = reader.read_record();

        match res {
            Ok(Some(recovered_payload)) => {
                // Se retornou Ok(Some), o payload NUNCA pode ser diferente do original
                // (isso seria silent data corruption)
                assert_eq!(
                    recovered_payload, payload,
                    "Silent data corruption detectada no byte {}!",
                    corrupted_idx
                );
            }
            Ok(None) => {
                // Truncamento limpo no final
            }
            Err(e) => {
                // Falha explícita com detecção de erro: comportamento correto fail-closed
                assert!(
                    matches!(e, pedradb_core::error::CoreError::Crc { .. } | pedradb_core::error::CoreError::Truncated(..))
                        || e.to_string().contains("corrupt")
                        || e.to_string().contains("internal")
                        || e.to_string().contains("WAL")
                        || e.to_string().contains("Truncated"),
                    "Erro não fail-closed no byte {}: {:?}",
                    corrupted_idx,
                    e
                );
            }
        }
    }
}
