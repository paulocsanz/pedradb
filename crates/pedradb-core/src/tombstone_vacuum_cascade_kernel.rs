//! RFC-0294: Pilar 2 - O Paradoxo do Vácuo de Tombstones e Invariante de Drenagem Ativa.
//!
//! Impede a paralisia não-ergódica do garbage collector do LSM quando grandes exclusões derrubam
//! o tamanho em bytes dos níveis, garantindo compactações ativas direcionadas por densidade e TTL de tombstones.

/// Violações do invariante de drenagem ativa de tombstones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TombstoneVacuumViolation {
    /// O nível acumulou alta densidade de tombstones expirados sem disparar compactação (Vácuo / Starvation).
    TombstoneVacuumStarvationDetected {
        /// Nível do LSM afetado.
        level: usize,
        /// Densidade de tombstones em permille [0, 1000].
        tombstone_permille: u64,
        /// Idade do tombstone mais antigo (em ticks/ns).
        oldest_age_ticks: u64,
        /// Limite de TTL configurado.
        ttl_threshold_ticks: u64,
    },
    /// A amplificação de espaço divergiu do teto assintótico pós-drenagem.
    SpaceAmplificationExceeded {
        /// Espaço total alocado no disco.
        allocated_bytes: u64,
        /// Dados úteis vivos reais.
        live_bytes: u64,
        /// Razão de amplificação calculada (em permille).
        space_amp_permille: u64,
    },
}

/// Estado estatístico de um nível do LSM para acionamento de compactação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelTombstoneState {
    /// Nível ordinal (0 = L0, 1 = L1, etc.).
    pub level: usize,
    /// Tamanho total em bytes dos arquivos do nível.
    pub total_bytes: u64,
    /// Quantidade total de registros no nível.
    pub total_records: u64,
    /// Quantidade de tombstones (deletes pontuais ou de intervalo).
    pub tombstone_records: u64,
    /// Idade do tombstone mais antigo presente no nível (em ticks).
    pub oldest_tombstone_age_ticks: u64,
}

impl LevelTombstoneState {
    /// Calcula a razão de densidade de tombstones em permille [0, 1000].
    pub fn tombstone_permille(&self) -> u64 {
        if self.total_records == 0 {
            0
        } else {
            (self.tombstone_records.saturating_mul(1000)) / self.total_records
        }
    }
}

/// Configuração da política de drenagem ativa de tombstones.
#[derive(Debug, Clone)]
pub struct TombstoneDrainConfig {
    /// Limite de bytes padrão para compactação por tamanho.
    pub size_threshold_bytes: u64,
    /// Densidade mínima de tombstones para considerar drenagem ativa (ex: 200 permille = 20%).
    pub min_tombstone_permille: u64,
    /// TTL máximo tolerado para sobrevivência de tombstones sem compactação (ex: 1000 ticks).
    pub tombstone_ttl_ticks: u64,
}

impl Default for TombstoneDrainConfig {
    fn default() -> Self {
        Self {
            size_threshold_bytes: 64 * 1024 * 1024,
            min_tombstone_permille: 200,
            tombstone_ttl_ticks: 1000,
        }
    }
}

/// Motivo que disparou a compactação do nível.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionTriggerReason {
    /// Disparado pelo limiar de tamanho tradicional (Size-Tiered / Leveled threshold).
    SizeThresholdExceeded,
    /// Disparado pela drenagem ativa de tombstones (resolvendo o Paradoxo do Vácuo).
    ActiveTombstoneDrain,
}

/// Controlador de drenagem ativa contra o Paradoxo do Vácuo de Tombstones.
pub struct TombstoneVacuumController {
    config: TombstoneDrainConfig,
}

impl TombstoneVacuumController {
    /// Cria uma nova instância com a configuração especificada.
    pub fn new(config: TombstoneDrainConfig) -> Self {
        Self { config }
    }

    /// Avalia se um nível deve sofrer compactação imediata.
    pub fn evaluate_level(&self, state: &LevelTombstoneState) -> Option<CompactionTriggerReason> {
        // 1. Regra clássica de tamanho
        if state.total_bytes >= self.config.size_threshold_bytes {
            return Some(CompactionTriggerReason::SizeThresholdExceeded);
        }

        // 2. Regra de drenagem ativa de tombstones (evita estagnação permanente)
        let density = state.tombstone_permille();
        if density >= self.config.min_tombstone_permille
            && state.oldest_tombstone_age_ticks >= self.config.tombstone_ttl_ticks
        {
            return Some(CompactionTriggerReason::ActiveTombstoneDrain);
        }

        None
    }

    /// Valida o invariante de que nenhum nível com alta densidade de tombstones expirados
    /// fica sem compactação pendente.
    pub fn verify_draining_invariants(
        &self,
        levels: &[LevelTombstoneState],
    ) -> Result<(), TombstoneVacuumViolation> {
        for state in levels {
            let density = state.tombstone_permille();
            if density >= self.config.min_tombstone_permille
                && state.oldest_tombstone_age_ticks >= self.config.tombstone_ttl_ticks
            {
                // Se preencheu os requisitos, o controlador DEVE ter acionado a compactação
                let decision = self.evaluate_level(state);
                if decision.is_none() {
                    return Err(TombstoneVacuumViolation::TombstoneVacuumStarvationDetected {
                        level: state.level,
                        tombstone_permille: density,
                        oldest_age_ticks: state.oldest_tombstone_age_ticks,
                        ttl_threshold_ticks: self.config.tombstone_ttl_ticks,
                    });
                }
            }
        }
        Ok(())
    }
}
