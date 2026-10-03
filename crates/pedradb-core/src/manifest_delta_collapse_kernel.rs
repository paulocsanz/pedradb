//! RFC-0291 Fronteira 4: Fecho Monoidal e Associatividade do Colapso de Deltas do Manifest.
//!
//! Modela formalmente as mutações do catálogo de níveis (VersionEdit) como uma
//! estrutura algébrica de Monóide Associativo com elemento neutro e absorção idempotente,
//! garantindo confluência e ausência de arquivos zumbis após reescrita do MANIFEST.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

/// Registro canônico de arquivo no catálogo de níveis.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ManifestFileEntry {
    pub level: usize,
    pub file_number: u64,
    pub min_key: Vec<u8>,
    pub max_key: Vec<u8>,
}

/// Delta de edição de versão (VersionEdit).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VersionEditDelta {
    pub added_files: BTreeMap<(usize, u64), ManifestFileEntry>,
    pub deleted_files: BTreeSet<(usize, u64)>,
    pub last_sequence: Option<u64>,
    pub next_file_number: Option<u64>,
}

impl VersionEditDelta {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Adiciona a criação de um arquivo a este delta.
    pub fn add_file(&mut self, entry: ManifestFileEntry) {
        let key = (entry.level, entry.file_number);
        // Se este arquivo foi marcado para deleção neste mesmo delta, limpa a deleção
        self.deleted_files.remove(&key);
        self.added_files.insert(key, entry);
    }

    /// Adiciona a deleção de um arquivo a este delta.
    pub fn delete_file(&mut self, level: usize, file_number: u64) {
        let key = (level, file_number);
        // Se o arquivo foi adicionado no próprio delta corrente, anulam-se mutuamente
        if self.added_files.remove(&key).is_none() {
            self.deleted_files.insert(key);
        }
    }

    /// Operador Monoidal de Composição $(\star)$: $D_1 \star D_2$.
    ///
    /// Aplica $D_2$ sobre a base $D_1$ preservando associatividade estrita.
    #[must_use]
    pub fn compose(&self, other: &Self) -> Self {
        let mut result = self.clone();

        // 1. Aplica deleções de other
        for &del_key in &other.deleted_files {
            // Se o arquivo estava em added_files de result, absorve
            if result.added_files.remove(&del_key).is_none() {
                result.deleted_files.insert(del_key);
            }
        }

        // 2. Aplica adições de other
        for (&add_key, entry) in &other.added_files {
            result.deleted_files.remove(&add_key);
            result.added_files.insert(add_key, entry.clone());
        }

        // 3. Atualiza contadores monotônicos
        if let Some(seq) = other.last_sequence {
            result.last_sequence = Some(result.last_sequence.map_or(seq, |s| s.max(seq)));
        }
        if let Some(next_fn) = other.next_file_number {
            result.next_file_number = Some(result.next_file_number.map_or(next_fn, |n| n.max(next_fn)));
        }

        result
    }
}

/// Estado materializado do catálogo de níveis da LSM.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ManifestCatalogState {
    pub levels: BTreeMap<usize, BTreeMap<u64, ManifestFileEntry>>,
    pub last_sequence: u64,
    pub next_file_number: u64,
}

impl ManifestCatalogState {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Aplica um delta de edição sobre o catálogo.
    pub fn apply_delta(&mut self, delta: &VersionEditDelta) {
        // Aplica deleções e poda níveis que fiquem vazios
        for &(level, file_number) in &delta.deleted_files {
            let empty = if let Some(level_files) = self.levels.get_mut(&level) {
                level_files.remove(&file_number);
                level_files.is_empty()
            } else {
                false
            };
            if empty {
                self.levels.remove(&level);
            }
        }

        // Aplica adições
        for (&(level, file_number), entry) in &delta.added_files {
            self.levels
                .entry(level)
                .or_default()
                .insert(file_number, entry.clone());
        }

        if let Some(seq) = delta.last_sequence {
            self.last_sequence = self.last_sequence.max(seq);
        }
        if let Some(nfn) = delta.next_file_number {
            self.next_file_number = self.next_file_number.max(nfn);
        }
    }

    /// Converte o estado materializado atual em um único delta de Snapshot canônico.
    #[must_use]
    pub fn to_snapshot_delta(&self) -> VersionEditDelta {
        let mut delta = VersionEditDelta::empty();
        for (&level, files) in &self.levels {
            for (&file_number, entry) in files {
                delta.added_files.insert((level, file_number), entry.clone());
            }
        }
        delta.last_sequence = Some(self.last_sequence);
        delta.next_file_number = Some(self.next_file_number);
        delta
    }
}

/// Oráculo de verificação de associatividade e colapso de deltas do Manifest.
pub struct ManifestAlgebraOracle;

impl ManifestAlgebraOracle {
    /// Comprova o Teorema da Associatividade Monoidal:
    /// $(D_1 \star D_2) \star D_3 \equiv D_1 \star (D_2 \star D_3)$
    pub fn verify_associativity(
        d1: &VersionEditDelta,
        d2: &VersionEditDelta,
        d3: &VersionEditDelta,
    ) -> bool {
        let left = d1.compose(d2).compose(d3);
        let right = d1.compose(&d2.compose(d3));
        left == right
    }

    /// Comprova que o colapso de uma sequência de deltas equivale exatamente à aplicação sucessiva.
    pub fn verify_fold_equivalence(
        initial: &ManifestCatalogState,
        deltas: &[VersionEditDelta],
    ) -> bool {
        // 1. Aplicação sucessiva delta a delta
        let mut state_successive = initial.clone();
        for d in deltas {
            state_successive.apply_delta(d);
        }

        // 2. Colapso algébrico monoidal prévio via fold
        let collapsed_delta = deltas
            .iter()
            .fold(VersionEditDelta::empty(), |acc, d| acc.compose(d));

        let mut state_collapsed = initial.clone();
        state_collapsed.apply_delta(&collapsed_delta);

        state_successive == state_collapsed
    }

    /// Comprova que o catálogo de níveis satisfaz invariantes fundamentais de integridade:
    /// 1. Para todo arquivo, min_key <= max_key
    /// 2. Para todo arquivo, file_number < next_file_number
    /// 3. Para níveis L > 0, os arquivos não possuem sobreposição de chaves
    pub fn verify_catalog_consistency(catalog: &ManifestCatalogState) -> bool {
        for (&level, files) in &catalog.levels {
            if files.is_empty() {
                return false;
            }
            let mut entries: Vec<&ManifestFileEntry> = files.values().collect();
            // Verifica limites e next_file_number
            for entry in &entries {
                if entry.min_key > entry.max_key {
                    return false;
                }
                if entry.file_number >= catalog.next_file_number {
                    return false;
                }
            }

            // Para L > 0, nenhum arquivo pode se sobrepor
            if level > 0 && entries.len() > 1 {
                entries.sort_by(|a, b| a.min_key.cmp(&b.min_key));
                for i in 0..entries.len() - 1 {
                    if entries[i].max_key >= entries[i + 1].min_key {
                        return false;
                    }
                }
            }
        }
        true
    }
}
