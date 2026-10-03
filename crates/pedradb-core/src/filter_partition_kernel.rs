//! RFC-0236 partitioned Bloom: which filter partition a point miss loads.
//! Integer remainder; no I/O, no env pin, no `PEDRA_FILTER_PARTS`.
//!
//! **Term:** this file is what `rustc` links. Aeneas extracts that body
//! (`scripts/aeneas_filter_partition.sh`).
//!
//!   ./scripts/aeneas_filter_partition.sh --required
//!
//! AS-IS always returns partition 0 (one unpartitioned filter — the
//! pre-0236 shape that loaded the whole Bloom on every miss).

#![forbid(unsafe_code)]

/// How many filter partitions a run of `n_keys` gets. Small runs stay
/// one filter; otherwise four ~equal parts (Fjall 3 / Rocks partitioned
/// filter). Not an env pin.
#[must_use]
pub fn filter_nparts(n_keys: u64) -> u32 {
    if n_keys < 4 {
        1
    } else {
        4
    }
}

/// AS-IS: every run is one filter.
#[must_use]
pub fn filter_nparts_as_is(_n_keys: u64) -> u32 {
    1
}

/// Partition index in `0..nparts` from a key hash (`h1`). `nparts <= 1`
/// collapses to 0.
#[must_use]
pub fn filter_partition(h1: u64, nparts: u32) -> u32 {
    if nparts <= 1 {
        return 0;
    }
    (h1 % u64::from(nparts)) as u32
}

/// AS-IS: always partition 0 (monolithic Bloom).
#[must_use]
pub fn filter_partition_as_is(_h1: u64, _nparts: u32) -> u32 {
    0
}

/// Erros de validação e roteamento em filtros particionados.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterPartitionError {
    /// Número de partições não pode ser zero.
    ZeroPartitions,
    /// Chave vazia fornecida para roteamento.
    EmptyKey,
}

impl std::fmt::Display for FilterPartitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroPartitions => write!(f, "filter partitions count cannot be zero"),
            Self::EmptyKey => write!(f, "key cannot be empty for filter partition routing"),
        }
    }
}

impl std::error::Error for FilterPartitionError {}

/// Roteador tipado e seguro para índices de partição de filtro Bloom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartitionedFilterRouter {
    nparts: u32,
}

impl PartitionedFilterRouter {
    /// Cria um novo roteador com validação fail-closed (rejeita 0 partições).
    pub fn try_new(nparts: u32) -> Result<Self, FilterPartitionError> {
        if nparts == 0 {
            return Err(FilterPartitionError::ZeroPartitions);
        }
        Ok(Self { nparts })
    }

    /// Retorna o número de partições ativas.
    #[must_use]
    pub fn nparts(&self) -> u32 {
        self.nparts
    }

    /// Roteia um hash de chave para a partição correspondente.
    #[must_use]
    pub fn route_hash(&self, h1: u64) -> u32 {
        filter_partition(h1, self.nparts)
    }

    /// Roteia uma chave em bytes validando não-vacuidade.
    pub fn route_key(&self, key: &[u8]) -> Result<u32, FilterPartitionError> {
        if key.is_empty() {
            return Err(FilterPartitionError::EmptyKey);
        }
        let h1 = u64::from(crc32c::crc32c(key));
        Ok(self.route_hash(h1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_partition_on_live_scan_window_is_not_ok() {
        assert_eq!(filter_partition(5, 4), 1);
        assert_eq!(
            filter_partition_as_is(5, 4),
            0,
            "AS-IS dente: every key still partition 0"
        );
    }

    #[test]
    fn filter_nparts_small_run_is_one() {
        assert_eq!(filter_nparts(0), 1);
        assert_eq!(filter_nparts(3), 1);
        assert_eq!(filter_nparts(4), 4);
        assert_eq!(filter_nparts_as_is(4), 1);
    }

    #[test]
    fn filter_partition_collapses_when_one_part() {
        assert_eq!(filter_partition(99, 1), 0);
        assert_eq!(filter_partition(99, 0), 0);
    }

    #[test]
    fn test_partitioned_filter_router_bounds() {
        assert_eq!(
            PartitionedFilterRouter::try_new(0),
            Err(FilterPartitionError::ZeroPartitions)
        );

        let router = PartitionedFilterRouter::try_new(4).expect("valid router");
        assert_eq!(router.nparts(), 4);
        assert_eq!(
            router.route_key(&[]),
            Err(FilterPartitionError::EmptyKey)
        );
        let part = router.route_key(b"test_key").expect("routed partition");
        assert!(part < 4);
    }

    #[test]
    fn test_filter_partition_error_display() {
        let err = FilterPartitionError::ZeroPartitions;
        assert_eq!(format!("{err}"), "filter partitions count cannot be zero");

        let err2 = FilterPartitionError::EmptyKey;
        assert_eq!(
            format!("{err2}"),
            "key cannot be empty for filter partition routing"
        );
    }
}

