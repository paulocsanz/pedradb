//! RFC-0291 Fronteira 4: Fecho Monoidal e Associatividade do Colapso de Deltas do Manifest.
//!
//! Modela formalmente as mutações do catálogo de níveis (VersionEdit) como uma
//! estrutura algébrica de Monóide Associativo com elemento neutro e absorção idempotente,
//! garantindo confluência e ausência de arquivos zumbis após reescrita do MANIFEST.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Erros estruturais no catálogo e deltas do MANIFEST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestCollapseError {
    /// Número de arquivo 0 é inválido.
    ZeroFileNumber,
    /// Nível excede o limite máximo permitido (7).
    LevelOutOfBounds { level: usize, max: usize },
    /// Intervalo de chaves invertido (`min_key > max_key`).
    InvertedKeyRange { min_key: Vec<u8>, max_key: Vec<u8> },
    /// Número do arquivo igual ou maior que `next_file_number`.
    FileNumberExceedsNext { file_number: u64, next_file_number: u64 },
    /// Sobreposição ilegal de chaves entre arquivos no mesmo nível (nível > 0).
    OverlappingFilesInLevel { level: usize, file1: u64, file2: u64 },
    /// Nível registrado mas sem arquivos.
    EmptyLevel(usize),
}

impl fmt::Display for ManifestCollapseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroFileNumber => write!(f, "File number 0 is invalid (must be > 0)"),
            Self::LevelOutOfBounds { level, max } => {
                write!(f, "Level {level} exceeds maximum allowed levels ({max})")
            }
            Self::InvertedKeyRange { min_key, max_key } => {
                write!(f, "Inverted key range: min_key {min_key:?} > max_key {max_key:?}")
            }
            Self::FileNumberExceedsNext { file_number, next_file_number } => {
                write!(f, "File number {file_number} >= next_file_number {next_file_number}")
            }
            Self::OverlappingFilesInLevel { level, file1, file2 } => {
                write!(f, "Overlapping key ranges in level {level} between files #{file1} and #{file2}")
            }
            Self::EmptyLevel(lvl) => write!(f, "Catalog level {lvl} is present but contains 0 files"),
        }
    }
}

impl std::error::Error for ManifestCollapseError {}

/// Registro canônico de arquivo no catálogo de níveis.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ManifestFileEntry {
    pub level: usize,
    pub file_number: u64,
    pub min_key: Vec<u8>,
    pub max_key: Vec<u8>,
}

impl ManifestFileEntry {
    /// Cria e valida uma entrada canônica de arquivo no catálogo de níveis.
    pub fn try_new(
        level: usize,
        file_number: u64,
        min_key: Vec<u8>,
        max_key: Vec<u8>,
    ) -> Result<Self, ManifestCollapseError> {
        if file_number == 0 {
            return Err(ManifestCollapseError::ZeroFileNumber);
        }
        if level >= 7 {
            return Err(ManifestCollapseError::LevelOutOfBounds { level, max: 7 });
        }
        if min_key > max_key {
            return Err(ManifestCollapseError::InvertedKeyRange { min_key, max_key });
        }
        Ok(Self {
            level,
            file_number,
            min_key,
            max_key,
        })
    }
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
    /// 2. Para todo arquivo, file_number > 0 e file_number < next_file_number
    /// 3. Para níveis L > 0, os arquivos não possuem sobreposição de chaves
    /// 4. Todo nível é estritamente < 7
    pub fn verify_catalog_consistency_checked(
        catalog: &ManifestCatalogState,
    ) -> Result<(), ManifestCollapseError> {
        for (&level, files) in &catalog.levels {
            if level >= 7 {
                return Err(ManifestCollapseError::LevelOutOfBounds { level, max: 7 });
            }
            if files.is_empty() {
                return Err(ManifestCollapseError::EmptyLevel(level));
            }
            let mut entries: Vec<&ManifestFileEntry> = files.values().collect();
            // Verifica limites, zero ID e next_file_number
            for entry in &entries {
                if entry.file_number == 0 {
                    return Err(ManifestCollapseError::ZeroFileNumber);
                }
                if entry.level >= 7 {
                    return Err(ManifestCollapseError::LevelOutOfBounds { level: entry.level, max: 7 });
                }
                if entry.min_key > entry.max_key {
                    return Err(ManifestCollapseError::InvertedKeyRange {
                        min_key: entry.min_key.clone(),
                        max_key: entry.max_key.clone(),
                    });
                }
                if entry.file_number >= catalog.next_file_number {
                    return Err(ManifestCollapseError::FileNumberExceedsNext {
                        file_number: entry.file_number,
                        next_file_number: catalog.next_file_number,
                    });
                }
            }

            // Para L > 0, nenhum arquivo pode se sobrepor
            if level > 0 && entries.len() > 1 {
                entries.sort_by(|a, b| a.min_key.cmp(&b.min_key));
                for i in 0..entries.len() - 1 {
                    if entries[i].max_key >= entries[i + 1].min_key {
                        return Err(ManifestCollapseError::OverlappingFilesInLevel {
                            level,
                            file1: entries[i].file_number,
                            file2: entries[i + 1].file_number,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Versão booleana para compatibilidade com oráculos existentes.
    pub fn verify_catalog_consistency(catalog: &ManifestCatalogState) -> bool {
        Self::verify_catalog_consistency_checked(catalog).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_delta_collapse_structural_invariants_red_to_green() {
        // 1. Zero file number hazard
        assert_eq!(
            ManifestFileEntry::try_new(0, 0, b"a".to_vec(), b"b".to_vec()),
            Err(ManifestCollapseError::ZeroFileNumber)
        );

        // 2. Level out of bounds hazard (>= 7)
        assert_eq!(
            ManifestFileEntry::try_new(7, 1, b"a".to_vec(), b"b".to_vec()),
            Err(ManifestCollapseError::LevelOutOfBounds { level: 7, max: 7 })
        );

        // 3. Inverted key range hazard
        assert_eq!(
            ManifestFileEntry::try_new(1, 1, b"z".to_vec(), b"a".to_vec()),
            Err(ManifestCollapseError::InvertedKeyRange {
                min_key: b"z".to_vec(),
                max_key: b"a".to_vec(),
            })
        );

        // 4. Catalog with zero file_number detected
        let mut bad_catalog = ManifestCatalogState::empty();
        bad_catalog.next_file_number = 100;
        let mut bad_delta = VersionEditDelta::empty();
        bad_delta.add_file(ManifestFileEntry {
            level: 1,
            file_number: 0,
            min_key: b"10".to_vec(),
            max_key: b"20".to_vec(),
        });
        bad_catalog.apply_delta(&bad_delta);
        assert_eq!(
            ManifestAlgebraOracle::verify_catalog_consistency_checked(&bad_catalog),
            Err(ManifestCollapseError::ZeroFileNumber)
        );
        assert!(!ManifestAlgebraOracle::verify_catalog_consistency(&bad_catalog));

        // 5. Overlapping files in L1 detected
        let mut overlap_catalog = ManifestCatalogState::empty();
        overlap_catalog.next_file_number = 100;
        let mut overlap_delta = VersionEditDelta::empty();
        overlap_delta.add_file(ManifestFileEntry::try_new(1, 1, b"10".to_vec(), b"30".to_vec()).unwrap());
        overlap_delta.add_file(ManifestFileEntry::try_new(1, 2, b"25".to_vec(), b"40".to_vec()).unwrap());
        overlap_catalog.apply_delta(&overlap_delta);
        assert_eq!(
            ManifestAlgebraOracle::verify_catalog_consistency_checked(&overlap_catalog),
            Err(ManifestCollapseError::OverlappingFilesInLevel {
                level: 1,
                file1: 1,
                file2: 2,
            })
        );

        // 6. Valid catalog passes
        let mut valid_catalog = ManifestCatalogState::empty();
        valid_catalog.next_file_number = 100;
        let mut valid_delta = VersionEditDelta::empty();
        valid_delta.add_file(ManifestFileEntry::try_new(1, 1, b"10".to_vec(), b"20".to_vec()).unwrap());
        valid_delta.add_file(ManifestFileEntry::try_new(1, 2, b"30".to_vec(), b"40".to_vec()).unwrap());
        valid_catalog.apply_delta(&valid_delta);
        assert!(ManifestAlgebraOracle::verify_catalog_consistency_checked(&valid_catalog).is_ok());
        assert!(ManifestAlgebraOracle::verify_catalog_consistency(&valid_catalog));
    }
}
