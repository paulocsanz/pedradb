//! Pilar 2: Fragmentação Atômica e Barreira de MTU WireGuard (RFC-0285).
//!
//! Garante que pacotes em trânsito pela malha WireGuard nunca excedam o PMTU seguro
//! (1420 bytes padrão menos headers), segmentando payloads volumosos em fragmentos
//! atômicos com integridade CRC32C e reensamblagem à prova de truncamento.

use bytes::{Bytes, BytesMut};

/// Limite máximo padrão do payload por frame na malha WireGuard.
/// MTU padrão = 1420B.
/// Menos: IP header (20B), UDP header (8B), WireGuard data header (32B), Frame Header (32B) = ~1328B de carga útil máxima.
pub const MESH_SAFE_PAYLOAD_LIMIT: usize = 1320;

/// Identificador único de mensagem federada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId {
    pub sender_node: u64,
    pub sequence: u64,
}

/// Cabeçalho de um fragmento de mensagem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FragmentHeader {
    pub msg_id: MessageId,
    pub fragment_index: u32,
    pub total_fragments: u32,
    pub total_message_len: u32,
    pub fragment_crc32c: u32,
    pub full_message_crc32c: u32,
}

/// Um fragmento físico pronto para despacho no túnel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireFragment {
    pub header: FragmentHeader,
    pub payload: Bytes,
}

/// Erro de fragmentação ou reensamblagem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FragmentationError {
    EmptyPayload,
    FragmentTooLarge { size: usize, limit: usize },
    CorruptFragmentCrc { expected: u32, actual: u32 },
    CorruptFullMessageCrc { expected: u32, actual: u32 },
    UnexpectedTotalFragments { expected: u32, got: u32 },
    IndexOutOfBounds { index: u32, total: u32 },
    DuplicateFragment(u32),
    IncompleteMessage { received: u32, expected: u32 },
    LengthMismatch { expected: u32, actual: u32 },
}

/// Fragmenta um payload de mensagem em fragmentos que respeitam estritamente `max_chunk_size`.
pub fn fragment_message(
    msg_id: MessageId,
    payload: &[u8],
    max_chunk_size: usize,
) -> Result<Vec<WireFragment>, FragmentationError> {
    if payload.is_empty() {
        return Err(FragmentationError::EmptyPayload);
    }
    let chunk_limit = max_chunk_size.min(MESH_SAFE_PAYLOAD_LIMIT);
    let full_crc = crc32c::crc32c(payload);
    let total_len = payload.len();
    let num_fragments = (total_len + chunk_limit - 1) / chunk_limit;

    let mut out = Vec::with_capacity(num_fragments);
    for idx in 0..num_fragments {
        let start = idx * chunk_limit;
        let end = (start + chunk_limit).min(total_len);
        let slice = &payload[start..end];
        let frag_crc = crc32c::crc32c(slice);

        out.push(WireFragment {
            header: FragmentHeader {
                msg_id,
                fragment_index: idx as u32,
                total_fragments: num_fragments as u32,
                total_message_len: total_len as u32,
                fragment_crc32c: frag_crc,
                full_message_crc32c: full_crc,
            },
            payload: Bytes::copy_from_slice(slice),
        });
    }
    Ok(out)
}

/// Estado acumulador de fragmentos para reensamblagem.
#[derive(Debug, Clone)]
pub struct Reassembler {
    pub msg_id: MessageId,
    pub total_fragments: u32,
    pub total_message_len: u32,
    pub full_message_crc32c: u32,
    pub received_fragments: Vec<Option<Bytes>>,
    pub received_count: u32,
}

impl Reassembler {
    pub fn new(first: &FragmentHeader) -> Self {
        let mut slots = Vec::with_capacity(first.total_fragments as usize);
        slots.resize(first.total_fragments as usize, None);
        Self {
            msg_id: first.msg_id,
            total_fragments: first.total_fragments,
            total_message_len: first.total_message_len,
            full_message_crc32c: first.full_message_crc32c,
            received_fragments: slots,
            received_count: 0,
        }
    }

    /// Adiciona um fragmento ao reassembler e valida integridades parciais.
    pub fn add_fragment(&mut self, frag: WireFragment) -> Result<Option<Bytes>, FragmentationError> {
        let h = &frag.header;
        if h.total_fragments != self.total_fragments {
            return Err(FragmentationError::UnexpectedTotalFragments {
                expected: self.total_fragments,
                got: h.total_fragments,
            });
        }
        if h.fragment_index >= self.total_fragments {
            return Err(FragmentationError::IndexOutOfBounds {
                index: h.fragment_index,
                total: self.total_fragments,
            });
        }
        // Validar integridade do fragmento individual
        let actual_crc = crc32c::crc32c(&frag.payload);
        if actual_crc != h.fragment_crc32c {
            return Err(FragmentationError::CorruptFragmentCrc {
                expected: h.fragment_crc32c,
                actual: actual_crc,
            });
        }

        let idx = h.fragment_index as usize;
        if self.received_fragments[idx].is_some() {
            return Err(FragmentationError::DuplicateFragment(h.fragment_index));
        }

        self.received_fragments[idx] = Some(frag.payload);
        self.received_count += 1;

        if self.received_count == self.total_fragments {
            // Reensamblagem atômica final
            let mut assembled = BytesMut::with_capacity(self.total_message_len as usize);
            for part in &self.received_fragments {
                assembled.extend_from_slice(part.as_ref().unwrap());
            }
            if assembled.len() != self.total_message_len as usize {
                return Err(FragmentationError::LengthMismatch {
                    expected: self.total_message_len,
                    actual: assembled.len() as u32,
                });
            }
            let full_crc = crc32c::crc32c(&assembled);
            if full_crc != self.full_message_crc32c {
                return Err(FragmentationError::CorruptFullMessageCrc {
                    expected: self.full_message_crc32c,
                    actual: full_crc,
                });
            }
            Ok(Some(assembled.freeze()))
        } else {
            Ok(None)
        }
    }
}

/// Mutante degenerado (AS-IS): ignora checagem de CRC no reassembler.
pub fn reassemble_as_is_no_crc(re: &mut Reassembler, frag: WireFragment) -> Result<Option<Bytes>, FragmentationError> {
    let idx = frag.header.fragment_index as usize;
    if re.received_fragments[idx].is_some() {
        return Err(FragmentationError::DuplicateFragment(frag.header.fragment_index));
    }
    re.received_fragments[idx] = Some(frag.payload);
    re.received_count += 1;
    if re.received_count == re.total_fragments {
        let mut assembled = BytesMut::new();
        for part in &re.received_fragments {
            assembled.extend_from_slice(part.as_ref().unwrap());
        }
        Ok(Some(assembled.freeze()))
    } else {
        Ok(None)
    }
}
