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
            (self.obsolete_bytes.saturating_mul(1000)) / self.raw_data_bytes
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
}

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
        assert_eq!(a.combine(SstMetadataMeasure::IDENTITY), a);

        // Associatividade
        let ab_c = a.combine(b).combine(c);
        let a_bc = a.combine(b.combine(c));
        if ab_c != a_bc {
            return Err(MeasureMonoidViolation::AssociativityViolation);
        }

        // Comutatividade
        assert_eq!(a.combine(b), b.combine(a));

        Ok(())
    }

    /// Valida o Teorema de Conservação de Medidas: a soma das medidas de fatias disjuntas
    /// geradas por sub-compactação paralela deve ser rigorosamente igual à medida do conjunto original.
    pub fn verify_slice_conservation(
        original: SstMetadataMeasure,
        slices: &[SstMetadataMeasure],
    ) -> Result<(), MeasureMonoidViolation> {
        let mut total = SstMetadataMeasure::IDENTITY;
        for &slice in slices {
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
