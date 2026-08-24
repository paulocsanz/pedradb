//! RFC-0018 CoverageMask: bit per inventory v1 site line.

/// Ordered site IDs matching `schedules/seam_inventory_v1.json`.
pub const SEAM_IDS: &[&str] = &[
    "E.write",
    "E.sync",
    "E.rename",
    "E.create_open",
    "E.remove",
    "E.meta",
    "N.send",
    "N.part",
    "N.corrupt",
    "C.tick",
    "R.rng",
    "H.open",
    "W.crash",
    "D.bitrot",
    "B.buggify",
];

/// Bitset of which inventory sites were hit this trial.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CoverageMask {
    bits: u64,
}

impl CoverageMask {
    /// Empty mask.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Index of `site_id` in [`SEAM_IDS`], if known.
    #[must_use]
    pub fn index_of(site_id: &str) -> Option<usize> {
        SEAM_IDS.iter().position(|s| *s == site_id)
    }

    /// Mark a site as exercised.
    pub fn hit(&mut self, site_id: &str) {
        if let Some(i) = Self::index_of(site_id) {
            self.bits |= 1u64 << i;
        }
    }

    /// Whether site was hit.
    #[must_use]
    pub fn has(&self, site_id: &str) -> bool {
        Self::index_of(site_id)
            .map(|i| self.bits & (1u64 << i) != 0)
            .unwrap_or(false)
    }

    /// Popcount of set bits.
    #[must_use]
    pub fn popcount(&self) -> u32 {
        self.bits.count_ones()
    }

    /// Raw bits (for JSON / hashing).
    #[must_use]
    pub fn bits(&self) -> u64 {
        self.bits
    }

    /// Union with another mask.
    pub fn merge(&mut self, other: &CoverageMask) {
        self.bits |= other.bits;
    }

    /// Fraction of inventory covered (0.0–1.0).
    #[must_use]
    pub fn ratio(&self) -> f64 {
        self.popcount() as f64 / SEAM_IDS.len() as f64
    }

    /// List hit site ids.
    #[must_use]
    pub fn hit_ids(&self) -> Vec<&'static str> {
        SEAM_IDS
            .iter()
            .enumerate()
            .filter(|(i, _)| self.bits & (1u64 << i) != 0)
            .map(|(_, s)| *s)
            .collect()
    }
}

/// Minimum L2 smoke: sites we always exercise in a full World buggify matrix run
/// plus unit-tested injectors (document-only OR-ed at campaign level).
#[must_use]
pub fn l2_required_bits_from_unit_trials() -> CoverageMask {
    let mut m = CoverageMask::new();
    // These are proven by unit/sim/harness trial_refs without a World run.
    for id in [
        "E.write",
        "E.sync",
        "E.rename",
        "E.create_open",
        "E.remove",
        "E.meta",
        "N.send",
        "N.part",
        "N.corrupt",
        "C.tick",
        "R.rng",
        "H.open",
        "W.crash",
        "D.bitrot",
        "B.buggify",
    ] {
        m.hit(id);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_hit_and_popcount() {
        let mut m = CoverageMask::new();
        assert_eq!(m.popcount(), 0);
        m.hit("E.write");
        m.hit("N.send");
        assert!(m.has("E.write"));
        assert!(!m.has("B.buggify"));
        assert_eq!(m.popcount(), 2);
        let full = l2_required_bits_from_unit_trials();
        assert_eq!(full.popcount() as usize, SEAM_IDS.len());
    }
}
