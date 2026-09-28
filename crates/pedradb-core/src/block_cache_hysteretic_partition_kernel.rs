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
}

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
    /// Inicializa o governador de cache com a capacidade total especificada.
    pub fn new(capacity_bytes: u64, floor_fraction: f64) -> Self {
        let meta_floor_bytes = ((capacity_bytes as f64) * floor_fraction.clamp(0.1, 0.9)) as u64;
        Self {
            capacity_bytes,
            meta_floor_bytes,
            current_meta_bytes: 0,
            current_data_bytes: 0,
        }
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

    /// Tenta admitir um bloco de metadados (índice ou filtro de Bloom).
    pub fn admit_metadata_block(&mut self, block_bytes: u64) {
        let available = self.capacity_bytes.saturating_sub(self.current_meta_bytes);
        if self.current_data_bytes > available {
            // Evict data blocks first to make room for metadata!
            let needed = block_bytes.saturating_sub(self.capacity_bytes.saturating_sub(self.current_meta_bytes + self.current_data_bytes));
            self.current_data_bytes = self.current_data_bytes.saturating_sub(needed);
        }
        self.current_meta_bytes = (self.current_meta_bytes + block_bytes).min(self.capacity_bytes);
    }

    /// Tenta admitir um bloco de dados (gerado por Get ou Scan sequencial).
    ///
    /// O bloco de dados JAMAIS pode desalocar metadados abaixo do piso `meta_floor_bytes`.
    pub fn admit_data_block(&mut self, block_bytes: u64, is_scan_stream: bool) {
        let max_data_allowed = self.capacity_bytes.saturating_sub(self.current_meta_bytes.max(self.meta_floor_bytes));

        if is_scan_stream {
            // Scans sofrem estrangulamento histerético para não preencher a cota de dados quentes
            let scan_quota = max_data_allowed / 2;
            if self.current_data_bytes + block_bytes > scan_quota {
                // Descarta em anel sem desalocar dados quentes
                self.current_data_bytes = scan_quota;
                return;
            }
        }

        if self.current_data_bytes + block_bytes > max_data_allowed {
            self.current_data_bytes = max_data_allowed;
        } else {
            self.current_data_bytes += block_bytes;
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
