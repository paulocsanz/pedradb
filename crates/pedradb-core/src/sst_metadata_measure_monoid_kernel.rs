//! RFC-0292: Pilar 8 - Homomorfismo de Medidas em Metadados de SST.
//!
//! Estrutura as estatísticas de SSTs como um monoide comutativo aditivo de medidas conservativas,
//! provando a preservação estrita de métricas em sub-compactações parciais para evitar compaction starvation.

/// Métricas agregadas de metadados de um arquivo ou fatia de SST.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SstMetadataMeasure {
    /// Total de bytes brutos dos blocos de dados.
    pub raw_data_bytes: u64,
    /// Quantidade total de registros (Puts + Merges + Tombstones).
    pub record_count: u64,
    /// Quantidade de tombstones pontuais e de intervalo.
    pub tombstone_count: u64,
    /// Estimativa de bytes obsoletos (passíveis de purga na compactação).
    pub obsolete_bytes: u64,
}

impl SstMetadataMeasure {
    /// Elemento neutro do monoide aditivo (0, 0, 0, 0).
    pub const IDENTITY: Self = Self {
        raw_data_bytes: 0,
        record_count: 0,
        tombstone_count: 0,
        obsolete_bytes: 0,
    };

    /// Valida que a medida satisfaz as restrições físicas de um SST válido:
    /// 1. Bytes obsoletos não podem exceder os bytes brutos totais.
    /// 2. Quantidade de tombstones não pode exceder o total de registros.
    pub fn is_valid(&self) -> bool {
        self.obsolete_bytes <= self.raw_data_bytes && self.tombstone_count <= self.record_count
    }

    /// Constrói uma medida de metadados com validação estrita de integridade física.
    pub fn try_new(
        raw_data_bytes: u64,
        record_count: u64,
        tombstone_count: u64,
        obsolete_bytes: u64,
    ) -> Result<Self, MeasureMonoidViolation> {
        let m = Self {
            raw_data_bytes,
            record_count,
            tombstone_count,
            obsolete_bytes,
        };
        if !m.is_valid() {
            return Err(MeasureMonoidViolation::InvalidMeasureBounds {
                raw_data_bytes,
                obsolete_bytes,
                record_count,
                tombstone_count,
            });
        }
        Ok(m)
    }

    /// Combina duas medidas com verificação estrita de ausência de overflow aritmético.
    #[must_use]
    pub fn checked_combine(self, other: Self) -> Option<Self> {
        let bypass_overflow = crate::mutate_switch!(
            crate::mutation_switch_kernel::MUTANT_OVERFLOW_SATURATION_BYPASS,
            false,
            true
        );
        if bypass_overflow {
            return Some(self.combine(other));
        }

        let raw_data_bytes = self.raw_data_bytes.checked_add(other.raw_data_bytes)?;
        let record_count = self.record_count.checked_add(other.record_count)?;
        let tombstone_count = self.tombstone_count.checked_add(other.tombstone_count)?;
        let obsolete_bytes = self.obsolete_bytes.checked_add(other.obsolete_bytes)?;
        Some(Self {
            raw_data_bytes,
            record_count,
            tombstone_count,
            obsolete_bytes,
        })
    }

    /// Combina duas medidas retornando erro explícito se ocorrer overflow.
    pub fn try_combine(self, other: Self) -> Result<Self, MeasureMonoidViolation> {
        self.checked_combine(other).ok_or(MeasureMonoidViolation::IntegerOverflow)
    }

    /// Operação monoidal de combinação aditiva (oplus).
    pub fn combine(self, other: Self) -> Self {
        Self {
            raw_data_bytes: self.raw_data_bytes.saturating_add(other.raw_data_bytes),
            record_count: self.record_count.saturating_add(other.record_count),
            tombstone_count: self.tombstone_count.saturating_add(other.tombstone_count),
            obsolete_bytes: self.obsolete_bytes.saturating_add(other.obsolete_bytes),
        }
    }

    /// Calcula a razão de obsolescência (dead data ratio) em permille [0, 1000].
    pub fn obsolete_permille(&self) -> u64 {
        if self.raw_data_bytes == 0 {
            0
        } else {
            let permille = (self.obsolete_bytes.saturating_mul(1000)) / self.raw_data_bytes;
            permille.min(1000)
        }
    }
}

/// Violações do homomorfismo monoidal de medidas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeasureMonoidViolation {
    /// A soma das fatias divergiu do metadado total do SST pai.
    PartitionConservationBroken {
        /// Medida do SST completo original.
        expected_total: SstMetadataMeasure,
        /// Medida agregada calculada pela soma das sub-fatias.
        aggregated_slices: SstMetadataMeasure,
    },
    /// A associatividade monoidal falhou.
    AssociativityViolation,
    /// O elemento neutro (identidade) falhou.
    IdentityViolation,
    /// A comutatividade monoidal falhou.
    CommutativityViolation,
    /// Limites físicos da medida violados (e.g. obsolete_bytes > raw_data_bytes).
    InvalidMeasureBounds {
        /// Bytes brutos totais.
        raw_data_bytes: u64,
        /// Bytes obsoletos alegados.
        obsolete_bytes: u64,
        /// Contagem total de registros.
        record_count: u64,
        /// Contagem de tombstones alegada.
        tombstone_count: u64,
    },
    IntegerOverflow,
    EmptySlices,
}

impl std::fmt::Display for MeasureMonoidViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PartitionConservationBroken { expected_total, aggregated_slices } => write!(
                f,
                "Partition conservation broken: expected total {expected_total:?}, aggregated slices {aggregated_slices:?}"
            ),
            Self::AssociativityViolation => write!(f, "Monoid associativity axiom violated"),
            Self::IdentityViolation => write!(f, "Monoid identity axiom violated"),
            Self::CommutativityViolation => write!(f, "Monoid commutativity axiom violated"),
            Self::InvalidMeasureBounds { raw_data_bytes, obsolete_bytes, record_count, tombstone_count } => write!(
                f,
                "Invalid measure bounds: raw={raw_data_bytes}, obsolete={obsolete_bytes}, records={record_count}, tombstones={tombstone_count}"
            ),
            Self::IntegerOverflow => write!(f, "Integer overflow occurred during monoidal measure combination"),
            Self::EmptySlices => write!(f, "Slice collection cannot be empty"),
        }
    }
}

impl std::error::Error for MeasureMonoidViolation {}

/// Oráculo de verificação do monoide de medidas de SST.
pub struct SstMeasureMonoidOracle;

impl SstMeasureMonoidOracle {
    /// Valida que a álgebra de medidas satisfaz formalmente os axiomas de monoide comutativo:
    /// 1. Elemento neutro: A oplus 0 == A.
    /// 2. Associatividade: (A oplus B) oplus C == A oplus (B oplus C).
    /// 3. Comutatividade: A oplus B == B oplus A.
    pub fn verify_monoid_axioms(
        a: SstMetadataMeasure,
        b: SstMetadataMeasure,
        c: SstMetadataMeasure,
    ) -> Result<(), MeasureMonoidViolation> {
        // Elemento neutro
        if a.combine(SstMetadataMeasure::IDENTITY) != a {
            return Err(MeasureMonoidViolation::IdentityViolation);
        }

        // Associatividade
        let ab_c = a.combine(b).combine(c);
        let a_bc = a.combine(b.combine(c));
        if ab_c != a_bc {
            return Err(MeasureMonoidViolation::AssociativityViolation);
        }

        // Comutatividade
        if a.combine(b) != b.combine(a) {
            return Err(MeasureMonoidViolation::CommutativityViolation);
        }

        Ok(())
    }

    /// Valida o Teorema de Conservação de Medidas: a soma das medidas de fatias disjuntas
    /// geradas por sub-compactação paralela deve ser rigorosamente igual à medida do conjunto original.
    pub fn verify_slice_conservation(
        original: SstMetadataMeasure,
        slices: &[SstMetadataMeasure],
    ) -> Result<(), MeasureMonoidViolation> {
        if !original.is_valid() {
            return Err(MeasureMonoidViolation::InvalidMeasureBounds {
                raw_data_bytes: original.raw_data_bytes,
                obsolete_bytes: original.obsolete_bytes,
                record_count: original.record_count,
                tombstone_count: original.tombstone_count,
            });
        }

        let mut total = SstMetadataMeasure::IDENTITY;
        for &slice in slices {
            if !slice.is_valid() {
                return Err(MeasureMonoidViolation::InvalidMeasureBounds {
                    raw_data_bytes: slice.raw_data_bytes,
                    obsolete_bytes: slice.obsolete_bytes,
                    record_count: slice.record_count,
                    tombstone_count: slice.tombstone_count,
                });
            }
            total = total.combine(slice);
        }

        if total != original {
            return Err(MeasureMonoidViolation::PartitionConservationBroken {
                expected_total: original,
                aggregated_slices: total,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sst_metadata_measure_try_new_red_to_green() {
        assert!(matches!(
            SstMetadataMeasure::try_new(100, 10, 2, 200),
            Err(MeasureMonoidViolation::InvalidMeasureBounds { .. })
        ));
        assert!(matches!(
            SstMetadataMeasure::try_new(100, 10, 20, 50),
            Err(MeasureMonoidViolation::InvalidMeasureBounds { .. })
        ));
        let m = SstMetadataMeasure::try_new(1000, 100, 5, 200).expect("valid");
        assert_eq!(m.obsolete_permille(), 200);
    }

    #[test]
    fn test_checked_and_try_combine_overflow_detection() {
        let m1 = SstMetadataMeasure {
            raw_data_bytes: u64::MAX,
            record_count: 10,
            tombstone_count: 0,
            obsolete_bytes: 0,
        };
        let m2 = SstMetadataMeasure {
            raw_data_bytes: 1,
            record_count: 0,
            tombstone_count: 0,
            obsolete_bytes: 0,
        };

        assert_eq!(m1.checked_combine(m2), None);
        assert_eq!(m1.try_combine(m2), Err(MeasureMonoidViolation::IntegerOverflow));
    }
}

