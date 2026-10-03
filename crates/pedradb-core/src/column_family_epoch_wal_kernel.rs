//! RFC-0294: Pilar 9 - Isolamento Epocal de Column Families no WAL Compartilhado.
//!
//! Vincula cada mutação no WAL à encarnação epocal da Column Family, provando formalmente que
//! mutações de tabelas deletadas jamais ressuscitam após recriação com mesmo nome/ID pós-crash.

use std::collections::HashMap;
use std::fmt;

/// Violações de isolamento epocal de Column Families no WAL compartilhado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnFamilyEpochViolation {
    /// Uma mutação pertencente a uma encarnação anterior (morta) foi erroneamente aplicada (Ressurreição).
    DeadIncarnationResurrected {
        /// ID da Column Family.
        cf_id: u32,
        /// Encarnação obsoleta encontrada no registro do WAL.
        stale_incarnation: u64,
        /// Encarnação ativa corrente no catálogo de versões.
        active_incarnation: u64,
        /// Sequence number da mutação.
        seq: u64,
    },
    /// A transição de encarnação no catálogo regrediu retroativamente (Não-monotônica).
    IncarnationRegressed {
        /// ID da Column Family.
        cf_id: u32,
        /// Encarnação anterior.
        prev_incarnation: u64,
        /// Nova encarnação menor.
        new_incarnation: u64,
    },
    /// Encarnação fornecida é 0 (sentinela inválido).
    ZeroIncarnation {
        /// ID da Column Family.
        cf_id: u32,
    },
    /// Sequence number fornecido é 0 (inválido).
    ZeroSequenceNumber,
    /// Chave fornecida é vazia.
    EmptyKey,
    /// Overflow de encarnação ao avançar.
    IncarnationOverflow {
        /// ID da Column Family.
        cf_id: u32,
    },
}

impl fmt::Display for ColumnFamilyEpochViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeadIncarnationResurrected { cf_id, stale_incarnation, active_incarnation, seq } => {
                write!(f, "CF {} dead incarnation {} resurrected against active {} at seq {}", cf_id, stale_incarnation, active_incarnation, seq)
            }
            Self::IncarnationRegressed { cf_id, prev_incarnation, new_incarnation } => {
                write!(f, "CF {} incarnation regressed from {} to {}", cf_id, prev_incarnation, new_incarnation)
            }
            Self::ZeroIncarnation { cf_id } => {
                write!(f, "CF {} cannot have zero incarnation", cf_id)
            }
            Self::ZeroSequenceNumber => write!(f, "Sequence number cannot be zero"),
            Self::EmptyKey => write!(f, "WAL record key cannot be empty"),
            Self::IncarnationOverflow { cf_id } => {
                write!(f, "CF {} incarnation overflowed u64::MAX", cf_id)
            }
        }
    }
}

impl std::error::Error for ColumnFamilyEpochViolation {}

/// Registro de escrita persistido no WAL compartilhado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedWalRecord {
    /// ID numérico da Column Family de destino.
    pub cf_id: u32,
    /// Encarnação da Column Family no momento da escrita.
    pub cf_incarnation: u64,
    /// Sequence number global da transação.
    pub seq: u64,
    /// Chave gravada.
    pub key: Vec<u8>,
    /// Valor gravado (None se for delete).
    pub value: Option<Vec<u8>>,
}

impl SharedWalRecord {
    /// Construtor seguro com validação estrita de invariantes.
    pub fn try_new(
        cf_id: u32,
        cf_incarnation: u64,
        seq: u64,
        key: Vec<u8>,
        value: Option<Vec<u8>>,
    ) -> Result<Self, ColumnFamilyEpochViolation> {
        if cf_incarnation == 0 {
            return Err(ColumnFamilyEpochViolation::ZeroIncarnation { cf_id });
        }
        if seq == 0 {
            return Err(ColumnFamilyEpochViolation::ZeroSequenceNumber);
        }
        if key.is_empty() {
            return Err(ColumnFamilyEpochViolation::EmptyKey);
        }
        Ok(Self {
            cf_id,
            cf_incarnation,
            seq,
            key,
            value,
        })
    }
}

/// Catálogo de encarnações ativas de Column Families.
#[derive(Debug, Clone, Default)]
pub struct ColumnFamilyCatalog {
    /// Mapeamento de CF_ID para sua encarnação ativa.
    active_incarnations: HashMap<u32, u64>,
}

impl ColumnFamilyCatalog {
    /// Registra uma nova Column Family ou avança a encarnação após um Drop + Create.
    pub fn register_or_recreate(&mut self, cf_id: u32) -> u64 {
        let entry = self.active_incarnations.entry(cf_id).or_insert(0);
        *entry = entry.checked_add(1).expect("Incarnation must not overflow u64::MAX");
        *entry
    }

    /// Define a encarnação para uma Column Family, garantindo monotonicidade estrita.
    pub fn set_incarnation(&mut self, cf_id: u32, incarnation: u64) -> Result<(), ColumnFamilyEpochViolation> {
        if incarnation == 0 {
            let current = self.active_incarnations.get(&cf_id).copied().unwrap_or(0);
            if current > 0 {
                return Err(ColumnFamilyEpochViolation::IncarnationRegressed {
                    cf_id,
                    prev_incarnation: current,
                    new_incarnation: incarnation,
                });
            }
            return Err(ColumnFamilyEpochViolation::ZeroIncarnation { cf_id });
        }
        let current = self.active_incarnations.get(&cf_id).copied().unwrap_or(0);
        if incarnation < current {
            return Err(ColumnFamilyEpochViolation::IncarnationRegressed {
                cf_id,
                prev_incarnation: current,
                new_incarnation: incarnation,
            });
        }
        self.active_incarnations.insert(cf_id, incarnation);
        Ok(())
    }

    /// Retorna a encarnação ativa atual para uma dada Column Family.
    pub fn get_active_incarnation(&self, cf_id: u32) -> Option<u64> {
        self.active_incarnations.get(&cf_id).copied()
    }
}

/// Oráculo de recuperação e reprodução segura do WAL compartilhado.
pub struct SharedWalRecoveryOracle;

impl SharedWalRecoveryOracle {
    /// Executa o algoritmo de replay do WAL pós-crash, aplicando apenas mutações da encarnação ativa
    /// e respeitando a monotonicidade do sequence number (Last-Write-Wins por seq).
    pub fn replay_wal(
        records: &[SharedWalRecord],
        catalog: &ColumnFamilyCatalog,
    ) -> Result<HashMap<(u32, Vec<u8>), Option<Vec<u8>>>, ColumnFamilyEpochViolation> {
        let mut reconstructed_state: HashMap<(u32, Vec<u8>), (u64, Option<Vec<u8>>)> = HashMap::new();

        for record in records {
            if record.cf_incarnation == 0 {
                return Err(ColumnFamilyEpochViolation::ZeroIncarnation { cf_id: record.cf_id });
            }
            if record.seq == 0 {
                return Err(ColumnFamilyEpochViolation::ZeroSequenceNumber);
            }
            if record.key.is_empty() {
                return Err(ColumnFamilyEpochViolation::EmptyKey);
            }

            let active_inc = catalog
                .get_active_incarnation(record.cf_id)
                .unwrap_or(0);

            if record.cf_incarnation < active_inc {
                // Registro de encarnação morta anterior ao Drop: DEVE SER DESCARTADO!
                continue;
            } else if record.cf_incarnation == active_inc && active_inc > 0 {
                // Registro da encarnação ativa corrente: aplica a mutação apenas se seq >= seq anterior
                let key = (record.cf_id, record.key.clone());
                match reconstructed_state.get(&key) {
                    Some((prev_seq, _)) if record.seq < *prev_seq => {
                        // Descarta gravação fora de ordem ou mais antiga
                        continue;
                    }
                    _ => {
                        reconstructed_state.insert(key, (record.seq, record.value.clone()));
                    }
                }
            } else {
                // Registro com encarnação superior à do catálogo ou CF não registrada: violação!
                return Err(ColumnFamilyEpochViolation::DeadIncarnationResurrected {
                    cf_id: record.cf_id,
                    stale_incarnation: record.cf_incarnation,
                    active_incarnation: active_inc,
                    seq: record.seq,
                });
            }
        }

        Ok(reconstructed_state.into_iter().map(|(k, (_, v))| (k, v)).collect())
    }

    /// Replay com proveniência de encarnação e sequência, permitindo distinguir
    /// mutações da nova encarnação que por coincidência possuem valor idêntico ao antigo.
    pub fn replay_wal_with_provenance(
        records: &[SharedWalRecord],
        catalog: &ColumnFamilyCatalog,
    ) -> Result<HashMap<(u32, Vec<u8>), (u64, u64, Option<Vec<u8>>)>, ColumnFamilyEpochViolation> {
        let mut state: HashMap<(u32, Vec<u8>), (u64, u64, Option<Vec<u8>>)> = HashMap::new();

        for record in records {
            if record.cf_incarnation == 0 {
                return Err(ColumnFamilyEpochViolation::ZeroIncarnation { cf_id: record.cf_id });
            }
            if record.seq == 0 {
                return Err(ColumnFamilyEpochViolation::ZeroSequenceNumber);
            }
            if record.key.is_empty() {
                return Err(ColumnFamilyEpochViolation::EmptyKey);
            }

            let active_inc = catalog
                .get_active_incarnation(record.cf_id)
                .unwrap_or(0);

            if record.cf_incarnation < active_inc {
                continue;
            } else if record.cf_incarnation == active_inc && active_inc > 0 {
                let key = (record.cf_id, record.key.clone());
                match state.get(&key) {
                    Some((_, prev_seq, _)) if record.seq < *prev_seq => continue,
                    _ => {
                        state.insert(key, (record.cf_incarnation, record.seq, record.value.clone()));
                    }
                }
            } else {
                return Err(ColumnFamilyEpochViolation::DeadIncarnationResurrected {
                    cf_id: record.cf_id,
                    stale_incarnation: record.cf_incarnation,
                    active_incarnation: active_inc,
                    seq: record.seq,
                });
            }
        }

        Ok(state)
    }

    /// Valida que chaves gravadas em uma encarnação anterior JAMAIS estão presentes no estado recuperado.
    pub fn verify_drop_isolation(
        recovered_state: &HashMap<(u32, Vec<u8>), Option<Vec<u8>>>,
        dead_records: &[SharedWalRecord],
    ) -> Result<(), ColumnFamilyEpochViolation> {
        for dead in dead_records {
            if let Some(val) = recovered_state.get(&(dead.cf_id, dead.key.clone())) {
                if *val == dead.value {
                    return Err(ColumnFamilyEpochViolation::DeadIncarnationResurrected {
                        cf_id: dead.cf_id,
                        stale_incarnation: dead.cf_incarnation,
                        active_incarnation: dead.cf_incarnation + 1,
                        seq: dead.seq,
                    });
                }
            }
        }
        Ok(())
    }

    /// Valida com proveniência que mutações de encarnações mortas jamais persistem,
    /// sem incorrer no falso-positivo quando a nova encarnação grava legitimamente o mesmo valor.
    pub fn verify_drop_isolation_provenance(
        recovered_provenance: &HashMap<(u32, Vec<u8>), (u64, u64, Option<Vec<u8>>)>,
        dead_records: &[SharedWalRecord],
    ) -> Result<(), ColumnFamilyEpochViolation> {
        for dead in dead_records {
            if let Some((active_inc, active_seq, _val)) = recovered_provenance.get(&(dead.cf_id, dead.key.clone())) {
                if *active_inc <= dead.cf_incarnation {
                    return Err(ColumnFamilyEpochViolation::DeadIncarnationResurrected {
                        cf_id: dead.cf_id,
                        stale_incarnation: dead.cf_incarnation,
                        active_incarnation: *active_inc,
                        seq: *active_seq,
                    });
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cf_epoch_wal_hardening_red_to_green() {
        // 1. Rejeita encarnação zero
        assert_eq!(
            SharedWalRecord::try_new(1, 0, 10, b"key".to_vec(), None),
            Err(ColumnFamilyEpochViolation::ZeroIncarnation { cf_id: 1 })
        );

        // 2. Rejeita sequência zero
        assert_eq!(
            SharedWalRecord::try_new(1, 1, 0, b"key".to_vec(), None),
            Err(ColumnFamilyEpochViolation::ZeroSequenceNumber)
        );

        // 3. Rejeita chave vazia
        assert_eq!(
            SharedWalRecord::try_new(1, 1, 10, vec![], None),
            Err(ColumnFamilyEpochViolation::EmptyKey)
        );

        // 4. Providência resolve o falso positivo de valor idêntico gravado na nova encarnação
        let mut catalog = ColumnFamilyCatalog::default();
        catalog.register_or_recreate(1); // inc 1
        catalog.register_or_recreate(1); // inc 2 (após drop)

        let records = vec![
            SharedWalRecord::try_new(1, 1, 10, b"config".to_vec(), Some(b"prod".to_vec())).unwrap(),
            SharedWalRecord::try_new(1, 2, 20, b"config".to_vec(), Some(b"prod".to_vec())).unwrap(),
        ];

        let prov_state = SharedWalRecoveryOracle::replay_wal_with_provenance(&records, &catalog).unwrap();
        let dead = vec![records[0].clone()];
        // verify_drop_isolation_provenance deve aprovar pois a encarnação do valor é 2 (> 1)
        assert!(SharedWalRecoveryOracle::verify_drop_isolation_provenance(&prov_state, &dead).is_ok());
    }
}
