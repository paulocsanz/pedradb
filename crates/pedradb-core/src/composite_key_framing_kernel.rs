//! RFC-0294: Pilar 4 - Álgebra de Codificação Injetiva de Chaves Compostas (Prefix-Free Framing).
//!
//! Garante a bijeção e a preservação rigorosa da ordem lexicográfica componente a componente em
//! chaves compostas (tuplas), erradicando ataques de injeção de separador e vazamento multi-tenant.

use std::cmp::Ordering;

/// Violações do contrato de codificação injetiva de tuplas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FramingViolation {
    /// O decodificador gerou uma tupla divergente da original (Falha de Invertibilidade).
    DecodedTupleMismatch {
        /// Tupla original.
        expected: Vec<Vec<u8>>,
        /// Tupla decodificada.
        decoded: Vec<Vec<u8>>,
    },
    /// A ordem lexicográfica dos bytes codificados divergiu da ordem da tupla original.
    LexicographicalOrderInverted {
        /// Tupla A menor.
        tuple_a: Vec<Vec<u8>>,
        /// Tupla B maior.
        tuple_b: Vec<Vec<u8>>,
        /// Bytes codificados de A.
        encoded_a: Vec<u8>,
        /// Bytes codificados de B.
        encoded_b: Vec<u8>,
    },
    /// Sequência malformada de bytes de escape ou terminação inválida.
    MalformedFramingSequence,
}

/// Codificador injetivo de tuplas binárias com preservação de ordem estrita (Prefix-Free Encoding).
///
/// Algoritmo:
/// Cada componente binário é codificado com escape de byte nulo:
/// - O byte `0x00` no payload é codificado como `[0x00, 0xFF]`.
/// - O final de cada componente é delimitador por `[0x00, 0x01]`.
/// - Como `0x00, 0x01 < 0x00, 0xFF`, um componente mais curto precede qualquer componente mais longo
///   com o mesmo prefixo, preservando estritamente a ordem lexicográfica canônica.
pub struct OrderPreservingTupleCodec;

impl OrderPreservingTupleCodec {
    /// Codifica uma tupla de componentes binários em uma única fatia contígua de bytes injetiva.
    pub fn encode_tuple(components: &[&[u8]]) -> Vec<u8> {
        let mut encoded = Vec::new();

        for component in components {
            for &byte in *component {
                if byte == 0x00 {
                    // Escape de byte nulo: 0x00 0xFF
                    encoded.push(0x00);
                    encoded.push(0xFF);
                } else {
                    encoded.push(byte);
                }
            }
            // Delimitador de fim de componente: 0x00 0x01
            encoded.push(0x00);
            encoded.push(0x01);
        }

        encoded
    }

    /// Decodifica os bytes para os componentes binários originais.
    pub fn decode_tuple(encoded: &[u8]) -> Result<Vec<Vec<u8>>, FramingViolation> {
        let mut components = Vec::new();
        let mut current = Vec::new();
        let mut i = 0;

        while i < encoded.len() {
            let byte = encoded[i];
            if byte == 0x00 {
                if i + 1 >= encoded.len() {
                    return Err(FramingViolation::MalformedFramingSequence);
                }
                let next = encoded[i + 1];
                match next {
                    0xFF => {
                        // Byte nulo escapado
                        current.push(0x00);
                        i += 2;
                    }
                    0x01 => {
                        // Fim de componente
                        components.push(current);
                        current = Vec::new();
                        i += 2;
                    }
                    _ => return Err(FramingViolation::MalformedFramingSequence),
                }
            } else {
                current.push(byte);
                i += 1;
            }
        }

        if !current.is_empty() {
            return Err(FramingViolation::MalformedFramingSequence);
        }

        Ok(components)
    }

    /// Compara duas tuplas de componentes lexicograficamente componente a componente.
    pub fn compare_tuples(a: &[&[u8]], b: &[&[u8]]) -> Ordering {
        let min_len = a.len().min(b.len());
        for i in 0..min_len {
            let ord = a[i].cmp(b[i]);
            if ord != Ordering::Equal {
                return ord;
            }
        }
        a.len().cmp(&b.len())
    }

    /// Valida formalmente a bijeção e a preservação de ordem entre pares de tuplas arbitrárias.
    pub fn verify_tuple_invariants(
        tuple_a: &[&[u8]],
        tuple_b: &[&[u8]],
    ) -> Result<(), FramingViolation> {
        // 1. Prova de bijeção e invertibilidade
        let enc_a = Self::encode_tuple(tuple_a);
        let dec_a = Self::decode_tuple(&enc_a)?;
        let expected_a: Vec<Vec<u8>> = tuple_a.iter().map(|s| s.to_vec()).collect();
        if dec_a != expected_a {
            return Err(FramingViolation::DecodedTupleMismatch {
                expected: expected_a,
                decoded: dec_a,
            });
        }

        let enc_b = Self::encode_tuple(tuple_b);
        let dec_b = Self::decode_tuple(&enc_b)?;
        let expected_b: Vec<Vec<u8>> = tuple_b.iter().map(|s| s.to_vec()).collect();
        if dec_b != expected_b {
            return Err(FramingViolation::DecodedTupleMismatch {
                expected: expected_b,
                decoded: dec_b,
            });
        }

        // 2. Prova de homomorfismo de ordem:
        // tuple_a <=> tuple_b  DEVE SER IDÊNTICO A  enc_a <=> enc_b
        let tuple_ord = Self::compare_tuples(tuple_a, tuple_b);
        let bytes_ord = enc_a.cmp(&enc_b);

        if tuple_ord != bytes_ord {
            return Err(FramingViolation::LexicographicalOrderInverted {
                tuple_a: expected_a,
                tuple_b: expected_b,
                encoded_a: enc_a,
                encoded_b: enc_b,
            });
        }

        Ok(())
    }
}
