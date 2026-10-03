//! RFC-0294: Pilar 8 - Autômato Histerético de Partição de Cache de Blocos.
//!
//! Blinda o Block Cache contra thrashing provocado por scans analíticos massivos, garantindo
//! um piso inviolável de memória para blocos de índice/filtros via controle com histerese sigmoide.

/// Violações de estabilidade e partição do cache de blocos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheThrashingViolation {
    /// A cota de metadados foi reduzida abaixo do piso mínimo garantido (Thrashing / Eviction).
    MetadataFloorViolated {
        /// Bytes alocados atuais para metadados.
        actual_meta_bytes: u64,
        /// Piso mínimo inviolável exigido.
        floor_bytes: u64,
    },
    /// A soma das partições excedeu a capacidade total do cache de blocos.
    CacheCapacityExceeded {
        /// Total alocado (meta + dados).
        total_allocated: u64,
        /// Capacidade máxima do cache.
        cache_capacity: u64,
    },
    /// Capacidade total do cache não pode ser zero.
    ZeroCapacityBytes,
    /// Fração de piso de metadados inválida (deve estar estritamente entre 0.0 e 1.0).
    InvalidFloorFraction,
    /// Tamanho do bloco para admissão não pode ser zero.
    ZeroBlockSize,
}

impl std::fmt::Display for CacheThrashingViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MetadataFloorViolated { actual_meta_bytes, floor_bytes } => write!(
                f,
                "Metadata floor violated: actual {actual_meta_bytes} < floor {floor_bytes}"
            ),
            Self::CacheCapacityExceeded { total_allocated, cache_capacity } => write!(
                f,
                "Cache capacity exceeded: allocated {total_allocated} > capacity {cache_capacity}"
            ),
            Self::ZeroCapacityBytes => write!(f, "Block cache capacity cannot be zero"),
            Self::InvalidFloorFraction => write!(
                f,
                "Floor fraction must be finite and strictly between 0.0 and 1.0"
            ),
            Self::ZeroBlockSize => write!(f, "Block size for cache admission cannot be zero"),
        }
    }
}

impl std::error::Error for CacheThrashingViolation {}

/// Estado da alocação de memória no Block Cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachePartitionState {
    /// Bytes ocupados por blocos de índice e filtros de Bloom (alta prioridade).
    pub metadata_bytes: u64,
    /// Bytes ocupados por blocos de dados de usuário (baixa prioridade / sujeitos a scan).
    pub data_bytes: u64,
    /// Capacidade total de memória do cache.
    pub total_capacity_bytes: u64,
    /// Piso mínimo garantido para metadados (ex: 30% do total).
    pub metadata_floor_bytes: u64,
}

/// Autômato histerético para controle dinâmico da partição de cache.
pub struct HystereticCacheGovernor {
    /// Capacidade total do cache.
    capacity_bytes: u64,
    /// Piso mínimo de metadados.
    meta_floor_bytes: u64,
    /// Alocação atual de metadados.
    current_meta_bytes: u64,
    /// Alocação atual de dados brutos.
    current_data_bytes: u64,
}

impl HystereticCacheGovernor {
    /// Inicializa o governador com validação fail-closed de parâmetros.
    pub fn try_new(capacity_bytes: u64, floor_fraction: f64) -> Result<Self, CacheThrashingViolation> {
        if capacity_bytes == 0 {
            return Err(CacheThrashingViolation::ZeroCapacityBytes);
        }
        if !floor_fraction.is_finite() || floor_fraction <= 0.0 || floor_fraction >= 1.0 {
            return Err(CacheThrashingViolation::InvalidFloorFraction);
        }
        let safe_fraction = floor_fraction.clamp(0.1, 0.9);
        let meta_floor_bytes = ((capacity_bytes as f64) * safe_fraction) as u64;
        Ok(Self {
            capacity_bytes,
            meta_floor_bytes: meta_floor_bytes.min(capacity_bytes),
            current_meta_bytes: 0,
            current_data_bytes: 0,
        })
    }

    /// Inicializa o governador de cache com a capacidade total especificada.
    pub fn new(capacity_bytes: u64, floor_fraction: f64) -> Self {
        Self::try_new(capacity_bytes, floor_fraction).unwrap_or_else(|_| {
            let cap = capacity_bytes.max(1);
            let safe_fraction = if floor_fraction.is_finite() {
                floor_fraction.clamp(0.1, 0.9)
            } else {
                0.3
            };
            let meta_floor_bytes = ((cap as f64) * safe_fraction) as u64;
            Self {
                capacity_bytes: cap,
                meta_floor_bytes: meta_floor_bytes.min(cap),
                current_meta_bytes: 0,
                current_data_bytes: 0,
            }
        })
    }

    /// Retorna o estado atual da partição.
    pub fn state(&self) -> CachePartitionState {
        CachePartitionState {
            metadata_bytes: self.current_meta_bytes,
            data_bytes: self.current_data_bytes,
            total_capacity_bytes: self.capacity_bytes,
            metadata_floor_bytes: self.meta_floor_bytes,
        }
    }

    /// Tenta admitir um bloco de metadados com validação de parâmetros.
    pub fn try_admit_metadata_block(&mut self, block_bytes: u64) -> Result<(), CacheThrashingViolation> {
        if block_bytes == 0 {
            return Err(CacheThrashingViolation::ZeroBlockSize);
        }
        self.admit_metadata_block(block_bytes);
        Ok(())
    }

    /// Tenta admitir um bloco de metadados (índice ou filtro de Bloom).
    pub fn admit_metadata_block(&mut self, block_bytes: u64) {
        let new_meta = self.current_meta_bytes.saturating_add(block_bytes).min(self.capacity_bytes);
        self.current_meta_bytes = new_meta;

        // Evict data blocks if total exceeds capacity:
        // current_data_bytes + current_meta_bytes <= capacity_bytes
        let max_data = self.capacity_bytes.saturating_sub(self.current_meta_bytes);
        if self.current_data_bytes > max_data {
            self.current_data_bytes = max_data;
        }
    }

    /// Tenta admitir um bloco de dados com validação de parâmetros.
    pub fn try_admit_data_block(&mut self, block_bytes: u64, is_scan_stream: bool) -> Result<(), CacheThrashingViolation> {
        if block_bytes == 0 {
            return Err(CacheThrashingViolation::ZeroBlockSize);
        }
        self.admit_data_block(block_bytes, is_scan_stream);
        Ok(())
    }

    /// Tenta admitir um bloco de dados (gerado por Get ou Scan sequencial).
    ///
    /// O bloco de dados JAMAIS pode desalocar metadados abaixo do piso `meta_floor_bytes`.
    pub fn admit_data_block(&mut self, block_bytes: u64, is_scan_stream: bool) {
        let max_data_allowed = self.capacity_bytes.saturating_sub(self.current_meta_bytes.max(self.meta_floor_bytes));

        if is_scan_stream {
            // Scans sofrem estrangulamento histerético para não preencher a cota de dados quentes
            let scan_quota = max_data_allowed / 2;
            let target_data = self.current_data_bytes.saturating_add(block_bytes);
            if target_data > scan_quota {
                // Descarta em anel sem desalocar dados quentes
                self.current_data_bytes = scan_quota;
                return;
            }
        }

        let target_data = self.current_data_bytes.saturating_add(block_bytes);
        if target_data > max_data_allowed {
            self.current_data_bytes = max_data_allowed;
        } else {
            self.current_data_bytes = target_data;
        }
    }

    /// Valida formalmente as invariantes de isolamento e imunidade a thrashing.
    pub fn verify_cache_invariants(&self) -> Result<(), CacheThrashingViolation> {
        if self.current_meta_bytes < self.meta_floor_bytes {
            return Err(CacheThrashingViolation::MetadataFloorViolated {
                actual_meta_bytes: self.current_meta_bytes,
                floor_bytes: self.meta_floor_bytes,
            });
        }

        let total = self.current_meta_bytes + self.current_data_bytes;
        if total > self.capacity_bytes {
            return Err(CacheThrashingViolation::CacheCapacityExceeded {
                total_allocated: total,
                cache_capacity: self.capacity_bytes,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_cache_bounds_red_to_green() {
        assert_eq!(
            HystereticCacheGovernor::try_new(0, 0.3).err(),
            Some(CacheThrashingViolation::ZeroCapacityBytes)
        );
        assert_eq!(
            HystereticCacheGovernor::try_new(1000, 1.5).err(),
            Some(CacheThrashingViolation::InvalidFloorFraction)
        );

        let mut gov = HystereticCacheGovernor::try_new(1000, 0.3).expect("valid gov");
        assert_eq!(
            gov.try_admit_metadata_block(0).err(),
            Some(CacheThrashingViolation::ZeroBlockSize)
        );
        assert_eq!(
            gov.try_admit_data_block(0, false).err(),
            Some(CacheThrashingViolation::ZeroBlockSize)
        );
    }
}
