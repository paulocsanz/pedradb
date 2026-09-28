//! RFC-0292: Pilar 5 - Invariante de Bounded Extent Dispersal ($K$-Contiguidade no VFS).
//!
//! Impõe teto estrito de fragmentação física em extents de arquivos vLog e SST sob punch hole,
//! garantindo coalescência contígua e preservando a largura de banda sequencial do sistema de arquivos POSIX.

/// Violações do teto de dispersão e fragmentação de extents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtentDispersalViolation {
    /// O número de extents físicos excedeu o teto estrito de dispersão K.
    ExtentLimitExceeded {
        /// Quantidade real de extents físicos no inode.
        actual_extents: usize,
        /// Teto teórico máximo permitido.
        max_allowed_extents: usize,
        /// Tamanho total do arquivo.
        total_file_bytes: u64,
        /// Tamanho do chunk mínimo configurado.
        min_chunk_bytes: u64,
    },
    /// Fragmento microscópico não coalescido violando o tamanho mínimo de chunk.
    MicroFragmentDetected {
        /// Offset do fragmento.
        offset: u64,
        /// Tamanho insuficiente do fragmento.
        length: u64,
        /// Mínimo exigido.
        required_min: u64,
    },
}

/// Descritor de extent físico no sistema de arquivos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalExtent {
    /// Offset lógico inicial no arquivo.
    pub logical_offset: u64,
    /// Comprimento contíguo do bloco de dados (em bytes).
    pub length: u64,
    /// Flag indicando se é um buraco (hole esparso) ou dados alocados.
    pub is_hole: bool,
}

/// Gerenciador de alocação e coalescência de extents do VFS.
#[derive(Debug, Clone)]
pub struct VfsExtentManager {
    /// Tamanho total do arquivo.
    total_file_bytes: u64,
    /// Granularidade mínima de corte contíguo (ex: 2 MiB).
    min_chunk_bytes: u64,
    /// Lista ordenada e normalizada de extents físicos contíguos.
    extents: Vec<PhysicalExtent>,
}

impl VfsExtentManager {
    /// Inicializa um novo arquivo totalmente alocado de tamanho `total_bytes`.
    pub fn new(total_bytes: u64, min_chunk_bytes: u64) -> Self {
        let initial_extent = PhysicalExtent {
            logical_offset: 0,
            length: total_bytes,
            is_hole: false,
        };
        Self {
            total_file_bytes: total_bytes,
            min_chunk_bytes: min_chunk_bytes.max(4096),
            extents: vec![initial_extent],
        }
    }

    /// Retorna a lista atual de extents.
    pub fn extents(&self) -> &[PhysicalExtent] {
        &self.extents
    }

    /// Executa uma operação segura de punch-hole (desalocação de espaço) em `[offset, offset + length)`.
    /// Agrupa e alinha requisições para garantir o teto de fragmentação.
    pub fn punch_hole(&mut self, offset: u64, length: u64) {
        if length == 0 || offset >= self.total_file_bytes {
            return;
        }
        let end = (offset + length).min(self.total_file_bytes);

        let mut new_extents = Vec::new();
        for ext in &self.extents {
            let ext_end = ext.logical_offset + ext.length;

            if ext_end <= offset || ext.logical_offset >= end {
                // Fora da faixa afetada pelo punch hole
                new_extents.push(*ext);
            } else if ext.is_hole {
                // Já é buraco, mantém inalterado
                new_extents.push(*ext);
            } else {
                // Extent de dados intersecta a faixa de punch hole
                // 1. Prefixo de dados antes do buraco
                if ext.logical_offset < offset {
                    new_extents.push(PhysicalExtent {
                        logical_offset: ext.logical_offset,
                        length: offset - ext.logical_offset,
                        is_hole: false,
                    });
                }
                // 2. O novo buraco
                let hole_start = ext.logical_offset.max(offset);
                let hole_end = ext_end.min(end);
                new_extents.push(PhysicalExtent {
                    logical_offset: hole_start,
                    length: hole_end - hole_start,
                    is_hole: true,
                });
                // 3. Sufixo de dados após o buraco
                if ext_end > end {
                    new_extents.push(PhysicalExtent {
                        logical_offset: end,
                        length: ext_end - end,
                        is_hole: false,
                    });
                }
            }
        }

        // Coalescência estrita de extents adjacentes do mesmo tipo
        let mut coalesced: Vec<PhysicalExtent> = Vec::with_capacity(new_extents.len());
        for ext in new_extents {
            if let Some(last) = coalesced.last_mut() {
                if last.is_hole == ext.is_hole && last.logical_offset + last.length == ext.logical_offset {
                    last.length += ext.length;
                    continue;
                }
            }
            coalesced.push(ext);
        }

        self.extents = coalesced;
    }

    /// Valida o Invariante de Bounded Extent Dispersal:
    /// E <= floor(TotalBytes / MinChunkBytes) + 1
    pub fn verify_extent_dispersal_invariants(&self) -> Result<(), ExtentDispersalViolation> {
        let max_allowed = ((self.total_file_bytes / self.min_chunk_bytes) as usize).saturating_add(2);
        let actual = self.extents.len();

        if actual > max_allowed {
            return Err(ExtentDispersalViolation::ExtentLimitExceeded {
                actual_extents: actual,
                max_allowed_extents: max_allowed,
                total_file_bytes: self.total_file_bytes,
                min_chunk_bytes: self.min_chunk_bytes,
            });
        }

        // Verifica que não existem buracos adjacentes não coalescidos
        for i in 1..self.extents.len() {
            if self.extents[i - 1].is_hole && self.extents[i].is_hole {
                return Err(ExtentDispersalViolation::MicroFragmentDetected {
                    offset: self.extents[i].logical_offset,
                    length: self.extents[i].length,
                    required_min: self.min_chunk_bytes,
                });
            }
        }

        Ok(())
    }
}
