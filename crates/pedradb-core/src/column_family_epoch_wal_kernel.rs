//! RFC-0294: Pilar 9 - Isolamento Epocal de Column Families no WAL Compartilhado.
//!
//! Vincula cada mutação no WAL à encarnação epocal da Column Family, provando formalmente que
//! mutações de tabelas deletadas jamais ressuscitam após recriação com mesmo nome/ID pós-crash.

use std::collections::HashMap;

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
}

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
        *entry += 1;
        *entry
    }

    /// Retorna a encarnação ativa atual para uma dada Column Family.
    pub fn get_active_incarnation(&self, cf_id: u32) -> Option<u64> {
        self.active_incarnations.get(&cf_id).copied()
    }
}

/// Oráculo de recuperação e reprodução segura do WAL compartilhado.
pub struct SharedWalRecoveryOracle;

impl SharedWalRecoveryOracle {
    /// Executa o algoritmo de replay do WAL pós-crash, aplicando apenas mutações da encarnação ativa.
    pub fn replay_wal(
        records: &[SharedWalRecord],
        catalog: &ColumnFamilyCatalog,
    ) -> Result<HashMap<(u32, Vec<u8>), Option<Vec<u8>>>, ColumnFamilyEpochViolation> {
        let mut reconstructed_state = HashMap::new();

        for record in records {
            let active_inc = catalog
                .get_active_incarnation(record.cf_id)
                .unwrap_or(0);

            if record.cf_incarnation < active_inc {
                // Registro de encarnação morta anterior ao Drop: DEVE SER DESCARTADO!
                continue;
            } else if record.cf_incarnation == active_inc {
                // Registro da encarnação ativa corrente: aplica a mutação
                reconstructed_state.insert((record.cf_id, record.key.clone()), record.value.clone());
            } else {
                // Registro com encarnação superior à do catálogo: catálogo corrompido!
                return Err(ColumnFamilyEpochViolation::DeadIncarnationResurrected {
                    cf_id: record.cf_id,
                    stale_incarnation: record.cf_incarnation,
                    active_incarnation: active_inc,
                    seq: record.seq,
                });
            }
        }

        Ok(reconstructed_state)
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
}
