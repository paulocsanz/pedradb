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
    /// Creates a prefix extractor with compile-time or runtime length.
    pub const fn new(prefix_len: usize) -> Self {
        Self { prefix_len }
    }

    /// Safely constructs a prefix extractor, rejecting zero length.
    pub fn try_new(prefix_len: usize) -> Result<Self, PrefixHomomorphismViolation> {
        if prefix_len == 0 {
            return Err(PrefixHomomorphismViolation::ZeroPrefixLength);
        }
        Ok(Self { prefix_len })
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
    /// Prefix length cannot be zero.
    ZeroPrefixLength,
    /// Target prefix for scan/seek cannot be empty.
    EmptyTargetPrefix,
}

impl std::fmt::Display for PrefixHomomorphismViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MonotonicityInversion { key_a, key_b, prefix_a, prefix_b } => {
                write!(f, "Monotonicity inversion: keys [{key_a:?}, {key_b:?}] but prefixes [{prefix_a:?}, {prefix_b:?}]")
            }
            Self::PrefixConvexityBroken { start_key, mid_key, end_key, expected_prefix, got_prefix } => {
                write!(f, "Prefix convexity broken: span [{start_key:?}, {end_key:?}] expected prefix {expected_prefix:?}, mid key {mid_key:?} has {got_prefix:?}")
            }
            Self::KeyOmissionInContiguousScan { omitted_key, target_prefix } => {
                write!(f, "Key {omitted_key:?} omitted during contiguous scan for prefix {target_prefix:?}")
            }
            Self::ZeroPrefixLength => write!(f, "Prefix length cannot be zero"),
            Self::EmptyTargetPrefix => write!(f, "Target scan prefix cannot be empty"),
        }
    }
}

impl std::error::Error for PrefixHomomorphismViolation {}

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

            // Monotonicidade estrita: se chaves estão invertidas ou prefixo inverte a relação da chave
            if ord_full == Ordering::Greater
                || (ord_full == Ordering::Less && ord_prefix == Ordering::Greater)
                || (ord_full == Ordering::Equal && ord_prefix != Ordering::Equal)
            {
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
            let ord = comparator.compare(p, target_prefix);
            if ord == Ordering::Equal {
                in_prefix = true;
                results.push(key.as_slice());
            } else if in_prefix || ord == Ordering::Greater {
                // Como as chaves são ordenadas e convexas, ao sair do prefixo ou ultrapassá-lo encerramos
                break;
            }
        }

        results
    }

    /// Safely performs a prefix scan, validating that the target prefix is non-empty.
    pub fn try_scan_prefix<'a, C: CustomComparator>(
        comparator: &C,
        extractor: &PrefixExtractor,
        sorted_keys: &'a [Vec<u8>],
        target_prefix: &[u8],
    ) -> Result<Vec<&'a [u8]>, PrefixHomomorphismViolation> {
        if target_prefix.is_empty() {
            return Err(PrefixHomomorphismViolation::EmptyTargetPrefix);
        }
        Ok(Self::scan_prefix(comparator, extractor, sorted_keys, target_prefix))
    }

    /// Locates the index of the first key matching the target prefix via binary search in $O(\log N)$.
    pub fn seek_prefix_first<C: CustomComparator>(
        comparator: &C,
        extractor: &PrefixExtractor,
        sorted_keys: &[Vec<u8>],
        target_prefix: &[u8],
    ) -> Option<usize> {
        if sorted_keys.is_empty() || target_prefix.is_empty() {
            return None;
        }
        let mut low = 0;
        let mut high = sorted_keys.len();
        while low < high {
            let mid = low + (high - low) / 2;
            let p = extractor.extract(&sorted_keys[mid]);
            match comparator.compare(p, target_prefix) {
                Ordering::Less => low = mid + 1,
                Ordering::Equal | Ordering::Greater => high = mid,
            }
        }
        if low < sorted_keys.len() && comparator.compare(extractor.extract(&sorted_keys[low]), target_prefix) == Ordering::Equal {
            Some(low)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_prefix_len_try_new_rejected() {
        let err = PrefixExtractor::try_new(0).unwrap_err();
        assert_eq!(err, PrefixHomomorphismViolation::ZeroPrefixLength);
    }

    #[test]
    fn test_try_scan_prefix_empty_target_rejected() {
        let cmp = ByteLexicographicalComparator;
        let ext = PrefixExtractor::new(4);
        let keys = [b"user_1".to_vec()];
        let err = PrefixHomomorphismOracle::try_scan_prefix(&cmp, &ext, &keys, b"").unwrap_err();
        assert_eq!(err, PrefixHomomorphismViolation::EmptyTargetPrefix);
    }

    #[test]
    fn test_seek_prefix_first_binary_search() {
        let cmp = ByteLexicographicalComparator;
        let ext = PrefixExtractor::new(4);
        let keys = vec![
            b"aaaa_01".to_vec(),
            b"bbbb_01".to_vec(),
            b"user_01".to_vec(),
            b"user_02".to_vec(),
            b"zone_01".to_vec(),
        ];

        // Seek existing prefix
        let idx = PrefixHomomorphismOracle::seek_prefix_first(&cmp, &ext, &keys, b"user");
        assert_eq!(idx, Some(2));

        // Seek non-existing prefix
        let idx_none = PrefixHomomorphismOracle::seek_prefix_first(&cmp, &ext, &keys, b"cccc");
        assert_eq!(idx_none, None);

        // Seek with empty target
        assert_eq!(PrefixHomomorphismOracle::seek_prefix_first(&cmp, &ext, &keys, b""), None);
    }
}

