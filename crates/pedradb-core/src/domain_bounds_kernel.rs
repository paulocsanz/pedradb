//! RFC-0328: Domain Bounds Kernel & Structural Impossibility Barriers.
//!
//! Provides closed, encapsulated newtypes that make degenerate, zero, empty,
//! and inverted states unrepresentable in the type system ("Parse, Don't Validate").
//! Strictly adheres to `#![forbid(unsafe_code)]`.

use std::num::NonZeroU64;
use std::ops::Deref;

/// Erros estruturais de violação de limites de domínio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainBoundsError {
    /// Chave de dados vazia é proibida.
    EmptyKeyForbidden,
    /// Intervalo de chaves invertido (`smallest > largest`).
    InvertedKeyInterval {
        /// Chave mínima alegada.
        smallest_len: usize,
        /// Chave máxima alegada.
        largest_len: usize,
    },
    /// Número de sequência zerado é inválido.
    ZeroSequenceForbidden,
    /// Identificador de tenant vazio é proibido.
    EmptyTenantIdForbidden,
    /// Injeção de byte nulo em identificador de tenant é proibida.
    NullByteInTenantId {
        /// Posição do primeiro byte nulo encontrado.
        position: usize,
    },
    /// Denominador zero em cálculo de proporção/taxa.
    ZeroDenominator,
    /// Overflow aritmético em cálculo de taxa/proporção.
    ArithmeticOverflow,
}

impl std::fmt::Display for DomainBoundsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyKeyForbidden => write!(f, "Empty keys (&[]) are structurally forbidden"),
            Self::InvertedKeyInterval { smallest_len, largest_len } => write!(
                f,
                "Inverted key interval: smallest (len {smallest_len}) is strictly greater than largest (len {largest_len})"
            ),
            Self::ZeroSequenceForbidden => write!(f, "Sequence number 0 is structurally forbidden (must be >= 1)"),
            Self::EmptyTenantIdForbidden => write!(f, "Tenant ID cannot be empty"),
            Self::NullByteInTenantId { position } => write!(f, "Tenant ID contains illegal null byte at offset {position}"),
            Self::ZeroDenominator => write!(f, "Ratio denominator cannot be zero"),
            Self::ArithmeticOverflow => write!(f, "Arithmetic overflow during domain bound calculation"),
        }
    }
}

impl std::error::Error for DomainBoundsError {}

/// Fatia de chave não-vazia garantida por construção.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonEmptyKey<'a>(&'a [u8]);

impl<'a> NonEmptyKey<'a> {
    /// Constrói uma `NonEmptyKey` a partir de uma fatia de bytes, rejeitando fatias vazias.
    pub fn try_new(slice: &'a [u8]) -> Result<Self, DomainBoundsError> {
        if slice.is_empty() {
            Err(DomainBoundsError::EmptyKeyForbidden)
        } else {
            Ok(Self(slice))
        }
    }

    /// Retorna a fatia de bytes subjacente garantidamente não-vazia.
    #[must_use]
    pub fn as_bytes(&self) -> &'a [u8] {
        self.0
    }

    /// Converte para uma chave possuída (`OwnedNonEmptyKey`).
    #[must_use]
    pub fn to_owned_key(&self) -> OwnedNonEmptyKey {
        OwnedNonEmptyKey(self.0.to_vec())
    }
}

impl<'a> Deref for NonEmptyKey<'a> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a> AsRef<[u8]> for NonEmptyKey<'a> {
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

/// Chave possuída não-vazia garantida por construção.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OwnedNonEmptyKey(Vec<u8>);

impl OwnedNonEmptyKey {
    /// Constrói uma `OwnedNonEmptyKey`, rejeitando vetores vazios.
    pub fn try_new(bytes: Vec<u8>) -> Result<Self, DomainBoundsError> {
        if bytes.is_empty() {
            Err(DomainBoundsError::EmptyKeyForbidden)
        } else {
            Ok(Self(bytes))
        }
    }

    /// Retorna a fatia de bytes subjacente garantidamente não-vazia.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Empresta como uma `NonEmptyKey`.
    #[must_use]
    pub fn as_non_empty_key(&self) -> NonEmptyKey<'_> {
        NonEmptyKey(&self.0)
    }

    /// Consome e devolve o vetor subjacente.
    #[must_use]
    pub fn into_inner(self) -> Vec<u8> {
        self.0
    }
}

impl Deref for OwnedNonEmptyKey {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<[u8]> for OwnedNonEmptyKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Intervalo fechado de chaves `[smallest, largest]` com invariante estrita `smallest <= largest`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyInterval {
    smallest: OwnedNonEmptyKey,
    largest: OwnedNonEmptyKey,
}

impl KeyInterval {
    /// Constrói um intervalo de chaves validado, rejeitando intervalos invertidos ou chaves vazias.
    pub fn try_new(smallest: Vec<u8>, largest: Vec<u8>) -> Result<Self, DomainBoundsError> {
        let s = OwnedNonEmptyKey::try_new(smallest)?;
        let l = OwnedNonEmptyKey::try_new(largest)?;
        if s > l {
            return Err(DomainBoundsError::InvertedKeyInterval {
                smallest_len: s.len(),
                largest_len: l.len(),
            });
        }
        Ok(Self {
            smallest: s,
            largest: l,
        })
    }

    /// Constrói um intervalo a partir de referências garantidamente não-vazias.
    pub fn try_from_borrowed(smallest: &[u8], largest: &[u8]) -> Result<Self, DomainBoundsError> {
        Self::try_new(smallest.to_vec(), largest.to_vec())
    }

    /// Chave mínima do intervalo.
    #[must_use]
    pub fn smallest(&self) -> &OwnedNonEmptyKey {
        &self.smallest
    }

    /// Chave máxima do intervalo.
    #[must_use]
    pub fn largest(&self) -> &OwnedNonEmptyKey {
        &self.largest
    }

    /// Verifica se uma chave está contida no intervalo fechado `[smallest, largest]`.
    #[must_use]
    pub fn contains_key(&self, key: &[u8]) -> bool {
        key >= self.smallest.as_bytes() && key <= self.largest.as_bytes()
    }

    /// Verifica se dois intervalos se sobrepõem.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        self.smallest <= other.largest && other.smallest <= self.largest
    }

    /// Verifica se dois intervalos são disjuntos.
    #[must_use]
    pub fn is_disjoint(&self, other: &Self) -> bool {
        !self.overlaps(other)
    }
}

/// Número de sequência estritamente positivo (>= 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DomainSequence(NonZeroU64);

impl DomainSequence {
    /// Menor número de sequência possível (1).
    pub const MIN: Self = Self(match NonZeroU64::new(1) {
        Some(v) => v,
        None => unreachable!(),
    });

    /// Constrói a partir de um `NonZeroU64`.
    #[must_use]
    pub const fn new(val: NonZeroU64) -> Self {
        Self(val)
    }

    /// Tenta construir a partir de um `u64`, rejeitando 0.
    pub fn try_new(val: u64) -> Result<Self, DomainBoundsError> {
        NonZeroU64::new(val)
            .map(Self)
            .ok_or(DomainBoundsError::ZeroSequenceForbidden)
    }

    /// Retorna o valor numérico em `u64`.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    /// Retorna o próximo número de sequência com verificação de overflow.
    pub fn next(self) -> Result<Self, DomainBoundsError> {
        let n = self.get().checked_add(1).ok_or(DomainBoundsError::ArithmeticOverflow)?;
        Self::try_new(n)
    }

    /// Soma um delta com verificação estrita de ausência de overflow.
    pub fn checked_add(self, delta: u64) -> Result<Self, DomainBoundsError> {
        let n = self.get().checked_add(delta).ok_or(DomainBoundsError::ArithmeticOverflow)?;
        Self::try_new(n)
    }
}

/// Identificador de inquilino (Tenant) validado: não-vazio e sem bytes nulos (`\0`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ValidTenantId(String);

impl ValidTenantId {
    /// Constrói um `ValidTenantId`, rejeitando strings vazias e bytes nulos.
    pub fn try_new(id: impl Into<String>) -> Result<Self, DomainBoundsError> {
        let s = id.into();
        if s.is_empty() {
            return Err(DomainBoundsError::EmptyTenantIdForbidden);
        }
        if let Some(pos) = s.bytes().position(|b| b == 0) {
            return Err(DomainBoundsError::NullByteInTenantId { position: pos });
        }
        Ok(Self(s))
    }

    /// Retorna o identificador textual do inquilino.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Gera o prefixo canônico delimitado para isolamento no espaço de chaves físico.
    #[must_use]
    pub fn canonical_prefix(&self) -> Vec<u8> {
        let mut prefix = Vec::with_capacity(self.0.len() + 2);
        prefix.extend_from_slice(self.0.as_bytes());
        prefix.push(b':');
        prefix.push(b':');
        prefix
    }

    /// Formata uma chave lógica adicionando o prefixo do inquilino de forma injetiva.
    #[must_use]
    pub fn prefix_key(&self, key: &NonEmptyKey<'_>) -> Vec<u8> {
        let mut out = self.canonical_prefix();
        out.extend_from_slice(key.as_bytes());
        out
    }

    /// Verifica se uma chave física pertence a este inquilino.
    #[must_use]
    pub fn owns_physical_key(&self, key: &[u8]) -> bool {
        let prefix = self.canonical_prefix();
        key.starts_with(&prefix)
    }
}

/// Proporção e taxa com proteção estrita contra overflow de multiplicação (`val * 100`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CheckedPermille(u32);

impl CheckedPermille {
    /// Zero por mil (0%).
    pub const ZERO: Self = Self(0);
    /// Mil por mil (100%).
    pub const ONE_HUNDRED_PERCENT: Self = Self(1000);

    /// Constrói a partir do valor direto em permil (0..=1000).
    pub fn from_permille(permille: u32) -> Result<Self, DomainBoundsError> {
        if permille > 1000 {
            Err(DomainBoundsError::ArithmeticOverflow)
        } else {
            Ok(Self(permille))
        }
    }

    /// Calcula a taxa `(numerator / denominator)` em permil (0..=1000) sem overflow intermediário.
    pub fn from_ratio(numerator: u64, denominator: u64) -> Result<Self, DomainBoundsError> {
        if denominator == 0 {
            return Err(DomainBoundsError::ZeroDenominator);
        }
        if numerator >= denominator {
            return Ok(Self::ONE_HUNDRED_PERCENT);
        }
        let num = numerator as u128;
        let den = denominator as u128;
        let permille = ((num * 1000 + (den / 2)) / den) as u32;
        Ok(Self(permille.min(1000)))
    }

    /// Aplica a proporção a um valor total sem risco de overflow intermediário.
    pub fn apply_to(&self, total: u64) -> Result<u64, DomainBoundsError> {
        let res = (((total as u128) * (self.0 as u128) + 500) / 1000) as u64;
        Ok(res)
    }

    /// Retorna o valor numérico em permille (0..=1000).
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_empty_key_construction_red_to_green() {
        assert_eq!(
            NonEmptyKey::try_new(b""),
            Err(DomainBoundsError::EmptyKeyForbidden)
        );
        let k = NonEmptyKey::try_new(b"valid_key").expect("valid");
        assert_eq!(k.as_bytes(), b"valid_key");
        assert_eq!(&*k, b"valid_key");
    }

    #[test]
    fn test_key_interval_inversion_rejection() {
        assert!(matches!(
            KeyInterval::try_new(vec![20], vec![10]),
            Err(DomainBoundsError::InvertedKeyInterval { .. })
        ));
        assert!(matches!(
            KeyInterval::try_new(vec![], vec![10]),
            Err(DomainBoundsError::EmptyKeyForbidden)
        ));

        let interval = KeyInterval::try_new(b"a".to_vec(), b"z".to_vec()).expect("valid");
        assert!(interval.contains_key(b"m"));
        assert!(interval.contains_key(b"a"));
        assert!(interval.contains_key(b"z"));
        assert!(!interval.contains_key(b"0"));
        assert!(!interval.contains_key(b"{"));
    }

    #[test]
    fn test_domain_sequence_non_zero() {
        assert_eq!(
            DomainSequence::try_new(0),
            Err(DomainBoundsError::ZeroSequenceForbidden)
        );
        let seq = DomainSequence::try_new(10).expect("valid");
        assert_eq!(seq.get(), 10);
        let next = seq.next().expect("valid");
        assert_eq!(next.get(), 11);
    }

    #[test]
    fn test_valid_tenant_id_null_and_empty_rejection() {
        assert_eq!(
            ValidTenantId::try_new(""),
            Err(DomainBoundsError::EmptyTenantIdForbidden)
        );
        assert!(matches!(
            ValidTenantId::try_new("tenant\0evil"),
            Err(DomainBoundsError::NullByteInTenantId { position: 6 })
        ));

        let t = ValidTenantId::try_new("tenant_alpha").expect("valid");
        let k = NonEmptyKey::try_new(b"order_123").expect("valid");
        let physical = t.prefix_key(&k);
        assert!(t.owns_physical_key(&physical));
        assert!(!t.owns_physical_key(b"tenant_beta::order_123"));
    }

    #[test]
    fn test_checked_permille_no_overflow_on_large_values() {
        assert_eq!(
            CheckedPermille::from_ratio(10, 0),
            Err(DomainBoundsError::ZeroDenominator)
        );
        // Evita overflow de dirty_bytes * 100 mesmo com u64::MAX / 2
        let large_num = u64::MAX / 4;
        let large_denom = u64::MAX / 2;
        let p = CheckedPermille::from_ratio(large_num, large_denom).expect("valid");
        assert_eq!(p.get(), 500);

        let scaled = p.apply_to(100_000).expect("valid");
        assert_eq!(scaled, 50_000);
    }
}
