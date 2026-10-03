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
    /// Contabilidade impossível de espaço (ex: allocated_bytes < live_bytes).
    CorruptedSpaceAccounting {
        allocated_bytes: u64,
        live_bytes: u64,
    },
    /// Contabilidade impossível de registros (ex: tombstone_records > total_records).
    CorruptedRecordAccounting {
        total_records: u64,
        tombstone_records: u64,
    },
    /// Parâmetro de configuração inválido.
    InvalidConfig {
        reason: &'static str,
    },
}

impl std::fmt::Display for TombstoneVacuumViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TombstoneVacuumStarvationDetected {
                level,
                tombstone_permille,
                oldest_age_ticks,
                ttl_threshold_ticks,
            } => {
                write!(
                    f,
                    "tombstone vacuum starvation at level {level}: density {tombstone_permille}‰, oldest age {oldest_age_ticks} > TTL {ttl_threshold_ticks}"
                )
            }
            Self::SpaceAmplificationExceeded {
                allocated_bytes,
                live_bytes,
                space_amp_permille,
            } => {
                write!(
                    f,
                    "space amplification exceeded: allocated {allocated_bytes}B, live {live_bytes}B, amp {space_amp_permille}‰"
                )
            }
            Self::CorruptedSpaceAccounting {
                allocated_bytes,
                live_bytes,
            } => {
                write!(
                    f,
                    "corrupted space accounting: allocated {allocated_bytes}B < live {live_bytes}B"
                )
            }
            Self::CorruptedRecordAccounting {
                total_records,
                tombstone_records,
            } => {
                write!(
                    f,
                    "corrupted record accounting: total {total_records} < tombstones {tombstone_records}"
                )
            }
            Self::InvalidConfig { reason } => {
                write!(f, "invalid tombstone drain configuration: {reason}")
            }
        }
    }
}

impl std::error::Error for TombstoneVacuumViolation {}

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
    /// Valida consistência interna do estado do nível.
    pub fn validate(&self) -> Result<(), TombstoneVacuumViolation> {
        if self.tombstone_records > self.total_records {
            return Err(TombstoneVacuumViolation::CorruptedRecordAccounting {
                total_records: self.total_records,
                tombstone_records: self.tombstone_records,
            });
        }
        Ok(())
    }

    /// Calcula a razão de densidade de tombstones em permille [0, 1000].
    pub fn tombstone_permille(&self) -> u64 {
        if self.total_records == 0 {
            0
        } else {
            let safe_tombstones = self.tombstone_records.min(self.total_records);
            (safe_tombstones.saturating_mul(1000)) / self.total_records
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
    /// Limite máximo tolerado de amplificação de espaço em permille (ex: 2000 permille = 2.0x).
    pub max_space_amplification_permille: u64,
}

impl TombstoneDrainConfig {
    /// Valida rigorosamente a configuração da política de drenagem ativa.
    pub fn validate(&self) -> Result<(), TombstoneVacuumViolation> {
        if self.size_threshold_bytes == 0 {
            return Err(TombstoneVacuumViolation::InvalidConfig {
                reason: "size_threshold_bytes cannot be zero",
            });
        }
        if self.min_tombstone_permille == 0 || self.min_tombstone_permille > 1000 {
            return Err(TombstoneVacuumViolation::InvalidConfig {
                reason: "min_tombstone_permille must be between 1 and 1000",
            });
        }
        if self.max_space_amplification_permille < 1000 {
            return Err(TombstoneVacuumViolation::InvalidConfig {
                reason: "max_space_amplification_permille must be >= 1000",
            });
        }
        if self.tombstone_ttl_ticks == 0 {
            return Err(TombstoneVacuumViolation::InvalidConfig {
                reason: "tombstone_ttl_ticks cannot be zero",
            });
        }
        Ok(())
    }
}

impl Default for TombstoneDrainConfig {
    fn default() -> Self {
        Self {
            size_threshold_bytes: 64 * 1024 * 1024,
            min_tombstone_permille: 200,
            tombstone_ttl_ticks: 1000,
            max_space_amplification_permille: 2000,
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
    /// Disparado por estouro de amplificação de espaço além do teto assintótico.
    SpaceAmplificationExceeded,
}

/// Controlador de drenagem ativa contra o Paradoxo do Vácuo de Tombstones.
pub struct TombstoneVacuumController {
    config: TombstoneDrainConfig,
}

impl TombstoneVacuumController {
    /// Cria uma nova instância validando a configuração fornecida.
    pub fn try_new(config: TombstoneDrainConfig) -> Result<Self, TombstoneVacuumViolation> {
        config.validate()?;
        Ok(Self { config })
    }

    /// Cria uma nova instância com a configuração especificada.
    #[track_caller]
    pub fn new(config: TombstoneDrainConfig) -> Self {
        Self::try_new(config).expect("valid tombstone drain configuration required")
    }

    /// Avalia se a amplificação de espaço diverge do teto assintótico configurado.
    pub fn evaluate_space_amplification(
        &self,
        allocated_bytes: u64,
        live_bytes: u64,
    ) -> Result<(), TombstoneVacuumViolation> {
        if allocated_bytes < live_bytes {
            return Err(TombstoneVacuumViolation::CorruptedSpaceAccounting {
                allocated_bytes,
                live_bytes,
            });
        }
        if allocated_bytes == 0 {
            return Ok(());
        }
        if live_bytes == 0 {
            return Err(TombstoneVacuumViolation::SpaceAmplificationExceeded {
                allocated_bytes,
                live_bytes: 0,
                space_amp_permille: u64::MAX,
            });
        }
        let space_amp_permille = allocated_bytes.saturating_mul(1000) / live_bytes;
        if space_amp_permille > self.config.max_space_amplification_permille {
            return Err(TombstoneVacuumViolation::SpaceAmplificationExceeded {
                allocated_bytes,
                live_bytes,
                space_amp_permille,
            });
        }
        Ok(())
    }

    /// Avalia se um nível deve sofrer compactação imediata.
    pub fn evaluate_level(&self, state: &LevelTombstoneState) -> Option<CompactionTriggerReason> {
        // 1. Regra clássica de tamanho
        if state.total_bytes >= self.config.size_threshold_bytes {
            return Some(CompactionTriggerReason::SizeThresholdExceeded);
        }

        // 2. Regra de drenagem ativa de tombstones (evita estagnação permanente)
        let density = state.tombstone_permille();
        if density == 1000
            || (density >= self.config.min_tombstone_permille
                && state.oldest_tombstone_age_ticks >= self.config.tombstone_ttl_ticks)
        {
            return Some(CompactionTriggerReason::ActiveTombstoneDrain);
        }

        None
    }

    /// Avalia um nível considerando também a amplificação de espaço em relação aos dados vivos.
    pub fn evaluate_level_with_space_amp(
        &self,
        state: &LevelTombstoneState,
        live_bytes: u64,
    ) -> Option<CompactionTriggerReason> {
        if let Some(reason) = self.evaluate_level(state) {
            return Some(reason);
        }

        if live_bytes == 0 && state.total_bytes > 0 {
            return Some(CompactionTriggerReason::SpaceAmplificationExceeded);
        }

        if live_bytes > 0 {
            let space_amp = state.total_bytes.saturating_mul(1000) / live_bytes;
            if space_amp > self.config.max_space_amplification_permille {
                return Some(CompactionTriggerReason::SpaceAmplificationExceeded);
            }
        }

        None
    }

    /// Avalia um nível com validação estrita de integridade de espaço.
    pub fn try_evaluate_level_with_space_amp(
        &self,
        state: &LevelTombstoneState,
        live_bytes: u64,
    ) -> Result<Option<CompactionTriggerReason>, TombstoneVacuumViolation> {
        state.validate()?;
        if state.total_bytes < live_bytes {
            return Err(TombstoneVacuumViolation::CorruptedSpaceAccounting {
                allocated_bytes: state.total_bytes,
                live_bytes,
            });
        }
        Ok(self.evaluate_level_with_space_amp(state, live_bytes))
    }

    /// Valida o invariante de que nenhum nível com alta densidade de tombstones expirados
    /// fica sem compactação pendente.
    pub fn verify_draining_invariants(
        &self,
        levels: &[LevelTombstoneState],
    ) -> Result<(), TombstoneVacuumViolation> {
        for state in levels {
            state.validate()?;
            let density = state.tombstone_permille();
            if density == 1000
                || (density >= self.config.min_tombstone_permille
                    && state.oldest_tombstone_age_ticks >= self.config.tombstone_ttl_ticks)
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
