//! RFC-0291 Fronteira 2: Homomorfismo de Prefix Seek e Monotonicidade sob Comparadores Customizados.
//!
//! Verifica axiomaticamente que a projeção de prefixo comuta monotonicamente com
//! a relação de pré-ordem estrita do comparador de chaves, garantindo que buscas
//! e iterações por prefixo nunca sofram de terminação prematura ou omissão de chaves válidas.

#![forbid(unsafe_code)]

use std::cmp::Ordering;

/// Trait para comparador de chaves completo.
pub trait CustomComparator: Send + Sync {
    fn compare(&self, a: &[u8], b: &[u8]) -> Ordering;
    fn name(&self) -> &'static str;
}

/// Comparador lexicográfico canônico por bytes.
#[derive(Debug, Clone, Copy, Default)]
pub struct ByteLexicographicalComparator;

impl CustomComparator for ByteLexicographicalComparator {
    fn compare(&self, a: &[u8], b: &[u8]) -> Ordering {
        a.cmp(b)
    }
    fn name(&self) -> &'static str {
        "ByteLexicographicalComparator"
    }
}

/// Extrator de prefixo determinístico.
#[derive(Debug, Clone, Copy)]
pub struct PrefixExtractor {
    pub prefix_len: usize,
}

impl PrefixExtractor {
    pub const fn new(prefix_len: usize) -> Self {
        Self { prefix_len }
    }

    #[must_use]
    pub fn extract<'a>(&self, key: &'a [u8]) -> &'a [u8] {
        if key.len() >= self.prefix_len {
            &key[..self.prefix_len]
        } else {
            key
        }
    }
}

/// Violação da propriedade algébrica de homomorfismo de prefixo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefixHomomorphismViolation {
    /// $\pi(k_1) \prec \pi(k_2)$, mas $k_1 \succ k_2$ (quebra de monotonicidade estrita).
    MonotonicityInversion {
        key_a: Vec<u8>,
        key_b: Vec<u8>,
        prefix_a: Vec<u8>,
        prefix_b: Vec<u8>,
    },
    /// Convexidade quebrada: $k_1 \prec k_3 \prec k_2$, $\pi(k_1) = \pi(k_2)$, mas $\pi(k_3) \ne \pi(k_1)$.
    PrefixConvexityBroken {
        start_key: Vec<u8>,
        mid_key: Vec<u8>,
        end_key: Vec<u8>,
        expected_prefix: Vec<u8>,
        got_prefix: Vec<u8>,
    },
    /// Chave ausente durante busca por prefixo em intervalo contíguo.
    KeyOmissionInContiguousScan {
        omitted_key: Vec<u8>,
        target_prefix: Vec<u8>,
    },
}

/// Oráculo de verificação formal de homomorfismo de prefixo.
pub struct PrefixHomomorphismOracle;

impl PrefixHomomorphismOracle {
    /// Valida que o comparador e o extrator de prefixo satisfazem monotonicidade estrita e convexidade.
    pub fn verify_homomorphism<C: CustomComparator>(
        comparator: &C,
        extractor: &PrefixExtractor,
        keys: &[Vec<u8>],
    ) -> Result<(), PrefixHomomorphismViolation> {
        if keys.len() < 2 {
            return Ok(());
        }

        // 1. Verifica monotonicidade de prefixos em pares ordenados
        for i in 0..keys.len() - 1 {
            let k_a = &keys[i];
            let k_b = &keys[i + 1];

            let p_a = extractor.extract(k_a);
            let p_b = extractor.extract(k_b);

            let ord_prefix = comparator.compare(p_a, p_b);
            let ord_full = comparator.compare(k_a, k_b);

            // Se prefixo A é estritamente maior que prefixo B, mas chave A é menor que B, violação!
            if ord_prefix == Ordering::Greater && ord_full != Ordering::Greater {
                return Err(PrefixHomomorphismViolation::MonotonicityInversion {
                    key_a: k_a.clone(),
                    key_b: k_b.clone(),
                    prefix_a: p_a.to_vec(),
                    prefix_b: p_b.to_vec(),
                });
            }
        }

        // 2. Verifica convexidade de prefixos (nenhum elemento intermediário tem prefixo alienígena)
        for i in 0..keys.len() {
            for j in (i + 2)..keys.len() {
                let p_start = extractor.extract(&keys[i]);
                let p_end = extractor.extract(&keys[j]);

                if comparator.compare(p_start, p_end) == Ordering::Equal {
                    // Todas as chaves intermediárias k_m entre i e j DEVEM ter o mesmo prefixo!
                    for m in (i + 1)..j {
                        let p_mid = extractor.extract(&keys[m]);
                        if comparator.compare(p_mid, p_start) != Ordering::Equal {
                            return Err(PrefixHomomorphismViolation::PrefixConvexityBroken {
                                start_key: keys[i].clone(),
                                mid_key: keys[m].clone(),
                                end_key: keys[j].clone(),
                                expected_prefix: p_start.to_vec(),
                                got_prefix: p_mid.to_vec(),
                            });
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Executa uma varredura por prefixo garantindo que 100% das chaves do prefixo sejam retornadas.
    pub fn scan_prefix<'a, C: CustomComparator>(
        comparator: &C,
        extractor: &PrefixExtractor,
        sorted_keys: &'a [Vec<u8>],
        target_prefix: &[u8],
    ) -> Vec<&'a [u8]> {
        let mut results = Vec::new();
        let mut in_prefix = false;

        for key in sorted_keys {
            let p = extractor.extract(key);
            if comparator.compare(p, target_prefix) == Ordering::Equal {
                in_prefix = true;
                results.push(key.as_slice());
            } else if in_prefix {
                // Como as chaves são ordenadas e convexas, ao sair do prefixo podemos encerrar
                break;
            }
        }

        results
    }
}
