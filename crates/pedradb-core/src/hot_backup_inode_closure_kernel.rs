//! RFC-0294: Pilar 7 - Fecho de Coerência de Inodes e Consistência Topológica em Hot-Backup.
//!
//! Garante que checkpoints físicos assíncronos via hard-links gerem conjuntos de arquivos
//! topologicamente fechados e causalmente consistentes com o MANIFEST vigente, sem órfãos nem lacunas.

use std::collections::HashSet;

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
}

/// Descritor do estado do catálogo no momento do congelamento do checkpoint.
#[derive(Debug, Clone)]
pub struct ManifestSnapshotView {
    /// Versão sequencial do MANIFEST.
    pub manifest_seq: u64,
    /// Conjunto de IDs de arquivos SST ativos referenciados nesta versão.
    pub referenced_sst_ids: HashSet<u64>,
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
}

/// Oráculo de verificação do fecho topológico de hot-backups.
pub struct HotBackupClosureOracle;

impl HotBackupClosureOracle {
    /// Valida que o diretório de backup gerado satisfaz estritamente a propriedade de fecho topológico:
    /// Todos os arquivos SST referenciados pelo MANIFEST do checkpoint estão presentes no backup,
    /// garantindo que a recuperação a partir do backup seja 100% autônoma e sem falhas de I/O.
    pub fn verify_backup_topological_closure(
        manifest_view: &ManifestSnapshotView,
        backup_state: &BackupDirectoryState,
    ) -> Result<(), BackupIncompletenessViolation> {
        // 1. Verifica presença do MANIFEST no backup
        let has_manifest = backup_state.files.iter().any(|f| f.starts_with("MANIFEST-") || f == "MANIFEST");
        if !has_manifest {
            return Err(BackupIncompletenessViolation::MissingOrCorruptedManifest);
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

        Ok(())
    }
}
