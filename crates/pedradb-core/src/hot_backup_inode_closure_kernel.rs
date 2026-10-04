//! RFC-0294: Pilar 7 - Fecho de Coerência de Inodes e Consistência Topológica em Hot-Backup.
//!
//! Garante que checkpoints físicos assíncronos via hard-links gerem conjuntos de arquivos
//! topologicamente fechados e causalmente consistentes com o MANIFEST vigente, sem órfãos nem lacunas.

#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::fmt;

/// Violações de coerência e fecho topológico de arquivos no hot-backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupIncompletenessViolation {
    /// O MANIFEST do backup referencia um arquivo SST que não foi copiado/linkado (Buraco no Backup).
    MissingReferencedSstFile {
        /// ID do arquivo SST ausente.
        file_id: u64,
        /// Nome do arquivo esperado.
        file_name: String,
    },
    /// O backup capturou um arquivo fantasma não catalogado no MANIFEST daquela versão.
    GhostFileInBackup {
        /// Nome do arquivo fantasma.
        file_name: String,
    },
    /// O arquivo MANIFEST copiado no backup está truncado ou ausente.
    MissingOrCorruptedManifest,
    /// Múltiplos arquivos MANIFEST conflitantes ou divergentes no backup.
    ConflictingManifestFiles {
        /// Nome do MANIFEST esperado.
        expected: String,
        /// Nome do MANIFEST conflitante encontrado.
        conflicting: String,
    },
    /// ID do arquivo SST referenciado é inválido (ex: 0).
    InvalidReferencedSstId {
        /// ID inválido.
        file_id: u64,
    },
    /// Sequência do MANIFEST é zero ou inválida.
    ZeroManifestSequence,
    /// Ponteiro CURRENT está vazio ou ausente.
    MissingCurrentPointer,
    /// Ponteiro CURRENT aponta para um MANIFEST divergente da versão do backup.
    DivergentCurrentPointer {
        /// Nome esperado do MANIFEST.
        expected: String,
        /// Alvo apontado pelo CURRENT.
        target: String,
    },
}

impl fmt::Display for BackupIncompletenessViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingReferencedSstFile { file_id, file_name } => {
                write!(f, "Missing referenced SST file #{file_id}: {file_name}")
            }
            Self::GhostFileInBackup { file_name } => {
                write!(f, "Ghost uncataloged file in backup: {file_name}")
            }
            Self::MissingOrCorruptedManifest => {
                write!(f, "Missing or corrupted MANIFEST in backup")
            }
            Self::ConflictingManifestFiles { expected, conflicting } => {
                write!(
                    f,
                    "Conflicting MANIFEST files in backup: expected {expected}, found conflicting {conflicting}"
                )
            }
            Self::InvalidReferencedSstId { file_id } => {
                write!(f, "Invalid referenced SST file ID {file_id} (must be non-zero)")
            }
            Self::ZeroManifestSequence => {
                write!(f, "Invalid MANIFEST sequence number 0 (must be > 0)")
            }
            Self::MissingCurrentPointer => {
                write!(f, "Missing or empty CURRENT pointer in backup")
            }
            Self::DivergentCurrentPointer { expected, target } => {
                write!(
                    f,
                    "Divergent CURRENT pointer: expected {expected}, but CURRENT points to {target}"
                )
            }
        }
    }
}

impl std::error::Error for BackupIncompletenessViolation {}

/// Descritor do estado do catálogo no momento do congelamento do checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestSnapshotView {
    /// Versão sequencial do MANIFEST.
    pub manifest_seq: u64,
    /// Conjunto de IDs de arquivos SST ativos referenciados nesta versão.
    pub referenced_sst_ids: HashSet<u64>,
}

impl ManifestSnapshotView {
    /// Cria uma nova visão de snapshot de MANIFEST validando integridade estrutural.
    pub fn try_new(
        manifest_seq: u64,
        referenced_sst_ids: HashSet<u64>,
    ) -> Result<Self, BackupIncompletenessViolation> {
        if manifest_seq == 0 {
            return Err(BackupIncompletenessViolation::ZeroManifestSequence);
        }
        for &sst_id in &referenced_sst_ids {
            if sst_id == 0 {
                return Err(BackupIncompletenessViolation::InvalidReferencedSstId { file_id: 0 });
            }
        }
        Ok(Self {
            manifest_seq,
            referenced_sst_ids,
        })
    }
}

/// Descritor do diretório de backup gerado com hard-links físicos.
#[derive(Debug, Clone, Default)]
pub struct BackupDirectoryState {
    /// Arquivos presentes no diretório de destino do backup.
    pub files: HashSet<String>,
}

impl BackupDirectoryState {
    /// Adiciona um arquivo linkado ao backup.
    pub fn add_file(&mut self, file_name: impl Into<String>) {
        self.files.insert(file_name.into());
    }

    /// Retorna o nome canônico do arquivo SST para um dado ID.
    pub fn sst_file_name(file_id: u64) -> String {
        format!("{:06}.sst", file_id)
    }

    /// Tenta extrair o ID numérico do SST a partir do nome do arquivo (ex: "000042.sst").
    pub fn parse_sst_id(file_name: &str) -> Option<u64> {
        if let Some(stem) = file_name.strip_suffix(".sst") {
            stem.parse::<u64>().ok()
        } else {
            None
        }
    }
}

/// Oráculo de verificação do fecho topológico de hot-backups.
pub struct HotBackupClosureOracle;

impl HotBackupClosureOracle {
    /// Valida que o diretório de backup gerado satisfaz estritamente a propriedade de fecho topológico:
    /// Todos os arquivos SST referenciados pelo MANIFEST do checkpoint estão presentes no backup,
    /// e nenhum SST fantasma não catalogado ou MANIFEST causalmente divergente está presente,
    /// garantindo que a recuperação a partir do backup seja 100% autônoma e sem falhas de I/O ou corrupção de catálogo.
    pub fn verify_backup_topological_closure(
        manifest_view: &ManifestSnapshotView,
        backup_state: &BackupDirectoryState,
    ) -> Result<(), BackupIncompletenessViolation> {
        // 0. Valida sanidade da sequência do MANIFEST e dos IDs de SST referenciados
        if manifest_view.manifest_seq == 0 {
            return Err(BackupIncompletenessViolation::ZeroManifestSequence);
        }
        for &sst_id in &manifest_view.referenced_sst_ids {
            if sst_id == 0 {
                return Err(BackupIncompletenessViolation::InvalidReferencedSstId { file_id: 0 });
            }
        }

        // 1. Verifica presença de MANIFEST causalmente compatível com manifest_seq no backup
        let expected_manifest_fmt = format!("MANIFEST-{:06}", manifest_view.manifest_seq);
        let expected_manifest_plain = format!("MANIFEST-{}", manifest_view.manifest_seq);

        let manifest_files: Vec<&String> = backup_state
            .files
            .iter()
            .filter(|f| f.starts_with("MANIFEST"))
            .collect();

        let has_matching_manifest = manifest_files.iter().any(|f| {
            **f == expected_manifest_fmt || **f == expected_manifest_plain
        });

        if !has_matching_manifest {
            return Err(BackupIncompletenessViolation::MissingOrCorruptedManifest);
        }

        // Se múltiplos arquivos MANIFEST existirem no backup, rejeita como conflito causal
        if manifest_files.len() > 1 {
            let conflicting = manifest_files
                .iter()
                .find(|f| {
                    ***f != expected_manifest_fmt && ***f != expected_manifest_plain
                })
                .unwrap_or(&manifest_files[1]);

            return Err(BackupIncompletenessViolation::ConflictingManifestFiles {
                expected: expected_manifest_fmt,
                conflicting: (*conflicting).clone(),
            });
        }

        // 2. Prova de fecho: todo SST referenciado no MANIFEST deve existir no diretório de backup
        for &sst_id in &manifest_view.referenced_sst_ids {
            let expected_name = BackupDirectoryState::sst_file_name(sst_id);
            if !backup_state.files.contains(&expected_name) {
                return Err(BackupIncompletenessViolation::MissingReferencedSstFile {
                    file_id: sst_id,
                    file_name: expected_name,
                });
            }
        }

        // 3. Prova de ausência de arquivos fantasma (Ghost files): qualquer arquivo .sst deve pertencer ao MANIFEST
        for file in &backup_state.files {
            if file.ends_with(".sst") {
                if let Some(sst_id) = BackupDirectoryState::parse_sst_id(file) {
                    if sst_id == 0 || !manifest_view.referenced_sst_ids.contains(&sst_id) {
                        return Err(BackupIncompletenessViolation::GhostFileInBackup {
                            file_name: file.clone(),
                        });
                    }
                } else {
                    return Err(BackupIncompletenessViolation::GhostFileInBackup {
                        file_name: file.clone(),
                    });
                }
            }
        }

        Ok(())
    }

    /// Valida que o ponteiro CURRENT do backup aponta exatamente para o MANIFEST canônico desta versão.
    pub fn verify_current_pointer_alignment(
        manifest_view: &ManifestSnapshotView,
        current_target: &str,
    ) -> Result<(), BackupIncompletenessViolation> {
        if manifest_view.manifest_seq == 0 {
            return Err(BackupIncompletenessViolation::ZeroManifestSequence);
        }
        let target = current_target.trim();
        if target.is_empty() {
            return Err(BackupIncompletenessViolation::MissingCurrentPointer);
        }
        let expected_fmt = format!("MANIFEST-{:06}", manifest_view.manifest_seq);
        let expected_plain = format!("MANIFEST-{}", manifest_view.manifest_seq);
        if target != expected_fmt && target != expected_plain {
            return Err(BackupIncompletenessViolation::DivergentCurrentPointer {
                expected: expected_fmt,
                target: target.to_string(),
            });
        }
        Ok(())
    }

    /// Valida o fecho topológico completo incluindo o arquivo CURRENT e seu apontador canônico.
    pub fn verify_complete_backup_with_current(
        manifest_view: &ManifestSnapshotView,
        backup_state: &BackupDirectoryState,
        current_target: &str,
    ) -> Result<(), BackupIncompletenessViolation> {
        Self::verify_backup_topological_closure(manifest_view, backup_state)?;
        if !backup_state.files.contains("CURRENT") {
            return Err(BackupIncompletenessViolation::MissingCurrentPointer);
        }
        Self::verify_current_pointer_alignment(manifest_view, current_target)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hot_backup_inode_closure_structural_invariants_red_to_green() {
        let mut ssts = HashSet::new();
        ssts.insert(10);
        ssts.insert(20);

        // 1. Zero manifest_seq hazard
        assert_eq!(
            ManifestSnapshotView::try_new(0, ssts.clone()),
            Err(BackupIncompletenessViolation::ZeroManifestSequence)
        );

        let manifest = ManifestSnapshotView::try_new(42, ssts).unwrap();

        // 2. Complete backup setup with CURRENT
        let mut backup = BackupDirectoryState::default();
        backup.add_file("MANIFEST-000042");
        backup.add_file("000010.sst");
        backup.add_file("000020.sst");
        backup.add_file("CURRENT");

        // Valid complete backup
        assert!(HotBackupClosureOracle::verify_complete_backup_with_current(
            &manifest,
            &backup,
            "MANIFEST-000042\n"
        ).is_ok());

        // 3. Divergent CURRENT pointer
        let div_err = HotBackupClosureOracle::verify_complete_backup_with_current(
            &manifest,
            &backup,
            "MANIFEST-000099"
        );
        assert!(matches!(
            div_err,
            Err(BackupIncompletenessViolation::DivergentCurrentPointer { .. })
        ));

        // 4. Missing CURRENT file
        let mut no_curr = backup.clone();
        no_curr.files.remove("CURRENT");
        let miss_curr = HotBackupClosureOracle::verify_complete_backup_with_current(
            &manifest,
            &no_curr,
            "MANIFEST-000042"
        );
        assert_eq!(
            miss_curr,
            Err(BackupIncompletenessViolation::MissingCurrentPointer)
        );

        // 5. Zero manifest_seq in topological closure
        let zero_view = ManifestSnapshotView {
            manifest_seq: 0,
            referenced_sst_ids: HashSet::new(),
        };
        assert_eq!(
            HotBackupClosureOracle::verify_backup_topological_closure(&zero_view, &backup),
            Err(BackupIncompletenessViolation::ZeroManifestSequence)
        );
    }
}
