//! Kernel: PITR replay window.
//!
//! `pitr_record_in_window` is the filter behind `restore_pitr`: a WAL
//! record is replayed iff `base < seq <= target`. Drop the upper bound
//! and writes after the target appear in the restored DB (silent-wrong
//! PITR — the crate's own test: seq 4 must not appear at target 3).

/// Whether an archived WAL record with `max_sequence = ms` belongs in a
/// PITR restore of `base < seq <= target` (inclusive target, exclusive base).
#[must_use]
pub fn pitr_record_in_window(ms: u64, base: u64, target: u64) -> bool {
    ms > base && ms <= target
}

/// Mutante honesto: só o bound inferior (`ms > base`).
///
/// Sem teto, seqs depois do target entram no restore — PITR mente.
#[must_use]
pub fn pitr_record_in_window_as_is(ms: u64, base: u64, _target: u64) -> bool {
    ms > base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitr_record_in_window_on_future_seq_is_not_ok() {
        // seq 4 at target 3, base 1: FIXED recusa, AS-IS abençoa.
        assert!(!pitr_record_in_window(4, 1, 3));
        assert!(pitr_record_in_window_as_is(4, 1, 3));
    }

    #[test]
    fn pitr_record_in_window_includes_target() {
        assert!(pitr_record_in_window(3, 1, 3));
        assert!(pitr_record_in_window_as_is(3, 1, 3));
    }

    #[test]
    fn pitr_record_in_window_excludes_base() {
        assert!(!pitr_record_in_window(1, 1, 3));
        assert!(!pitr_record_in_window_as_is(1, 1, 3));
    }
}
