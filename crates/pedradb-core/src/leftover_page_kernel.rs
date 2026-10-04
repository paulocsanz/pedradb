//! kernel: leftover page policy (RFC-0194 P0.1) — when may the compaction
//! leftover consumption drop SST pages from the OS cache
//! (`POSIX_FADV_DONTNEED` through the `Env::advise` seam)?
//!
//! Fire 118 (2026-09-09): the UNCONDITIONAL drop regressed the hot 2M
//! (0.795 → 0.681) — a fitting store must keep its pages. Fire 119 left the
//! correct condition designed: drop **iff** (a) the page-keep budget is 0
//! (default is keep — today's behavior), (b) no live SST covers the live
//! one-slash family (the SSTs are pure cold leftover), and (c) the store is
//! in bounded-cache mode (SST bytes above the WARM cap). Pure integer
//! units; the I/O decision that CALLS this kernel lives in `db.rs`.

#![forbid(unsafe_code)]

use std::fmt;

/// Verdict of the leftover page policy for one compaction install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftoverPageAdvice {
    /// Fire-119 condition holds — the installed leftover SSTs may be
    /// `DONTNEED`-advised off-lock.
    Drop,
    /// Store fits the WARM cap (hot) — never advise (the Fire-118 lesson).
    KeepHot,
    /// A live SST covers the live one-slash family — SSTs are not pure
    /// leftover; keep pages.
    KeepCovered,
    /// Budget ≠ 0 (default) — today's behavior: never advise.
    KeepDefault,
}

impl fmt::Display for LeftoverPageAdvice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Drop => write!(f, "Drop"),
            Self::KeepHot => write!(f, "KeepHot"),
            Self::KeepCovered => write!(f, "KeepCovered"),
            Self::KeepDefault => write!(f, "KeepDefault"),
        }
    }
}

/// Erros de validação estrutural de intervalos de chaves ou prefixos na política de leftover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeftoverPageError {
    /// Intervalo de chaves invertido (`smallest > largest`).
    InvertedRange {
        smallest: Vec<u8>,
        largest: Vec<u8>,
    },
    /// Prefixo de família vazio.
    EmptyPrefix,
}

impl fmt::Display for LeftoverPageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvertedRange { smallest, largest } => {
                write!(
                    f,
                    "Inverted key range: smallest {smallest:?} is strictly greater than largest {largest:?}"
                )
            }
            Self::EmptyPrefix => write!(f, "Family prefix cannot be empty"),
        }
    }
}

impl std::error::Error for LeftoverPageError {}

/// RFC-0194 P0.1: the Fire-119 condition as a total function.
///
/// `sst_page_keep_budget` 0 = drop-enabled (the finding's
/// `sst_page_keep_budget=0`); any nonzero value keeps today's behavior.
#[must_use]
pub fn leftover_page_advice(
    sst_page_keep_budget: u64,
    sst_bytes: u64,
    warm_cap_bytes: u64,
    live_family_covered_by_sst: bool,
) -> LeftoverPageAdvice {
    if sst_page_keep_budget != 0 {
        return LeftoverPageAdvice::KeepDefault;
    }
    if live_family_covered_by_sst {
        return LeftoverPageAdvice::KeepCovered;
    }
    if sst_bytes <= warm_cap_bytes {
        return LeftoverPageAdvice::KeepHot;
    }
    LeftoverPageAdvice::Drop
}

/// AS-IS twin: today's engine never advises leftover pages.
#[must_use]
pub fn leftover_page_advice_as_is(
    _sst_page_keep_budget: u64,
    _sst_bytes: u64,
    _warm_cap_bytes: u64,
    _live_family_covered_by_sst: bool,
) -> LeftoverPageAdvice {
    LeftoverPageAdvice::KeepDefault
}

/// Exclusive upper bound of the key interval of a one-slash family prefix
/// (`c/` → keys `c/..`, interval `[c/, "c0")`). `None` = unbounded above
/// (every byte of `pfx` is `0xFF`; cannot happen for slash-terminated
/// families, but the function stays total).
#[must_use]
pub fn family_upper_bound(pfx: &[u8]) -> Option<Vec<u8>> {
    // Index `while` (not `iter().rev()`) so Charon/Aeneas emit a `def`.
    // `wrapping_add` after `b < 0xFF` is the successor; raw `u8 + 1` is
    // unimplemented in the Aeneas pin.
    let mut i = pfx.len();
    while i > 0 {
        i -= 1;
        let b = pfx[i];
        if b < 0xFF {
            let mut hi = pfx.to_vec();
            hi[i] = b.wrapping_add(1);
            hi.truncate(i + 1);
            return Some(hi);
        }
    }
    None
}

/// Whether an SST's inclusive key range `[smallest, largest]` intersects the
/// key interval of one-slash family `pfx` (the "covering" test of the
/// Fire-119 condition (b)). A table with no known bounds never proves
/// coverage (conservative: `false` lets the drop fire only when the budget
/// and cap conditions hold anyway — same envelope as the finding).
#[must_use]
pub fn sst_range_covers_family(
    smallest: Option<&[u8]>,
    largest: Option<&[u8]>,
    pfx: &[u8],
) -> bool {
    let (Some(lo), Some(hi)) = (smallest, largest) else {
        return false;
    };
    if lo > hi {
        return false;
    }
    let fam_lo = pfx;
    match family_upper_bound(pfx) {
        Some(fam_hi_excl) => hi >= fam_lo && lo < fam_hi_excl.as_slice(),
        None => hi >= fam_lo,
    }
}

/// Versão com checagem estrita de integridade que rejeita prefixos vazios e intervalos invertidos.
pub fn sst_range_covers_family_checked(
    smallest: Option<&[u8]>,
    largest: Option<&[u8]>,
    pfx: &[u8],
) -> Result<bool, LeftoverPageError> {
    if pfx.is_empty() {
        return Err(LeftoverPageError::EmptyPrefix);
    }
    let (Some(lo), Some(hi)) = (smallest, largest) else {
        return Ok(false);
    };
    if lo > hi {
        return Err(LeftoverPageError::InvertedRange {
            smallest: lo.to_vec(),
            largest: hi.to_vec(),
        });
    }
    let fam_lo = pfx;
    let covers = match family_upper_bound(pfx) {
        Some(fam_hi_excl) => hi >= fam_lo && lo < fam_hi_excl.as_slice(),
        None => hi >= fam_lo,
    };
    Ok(covers)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default budget (nonzero) keeps today's behavior — the engine
    /// without the opt-in never advises, whatever the store shape.
    #[test]
    fn default_budget_never_drops() {
        assert_eq!(
            leftover_page_advice(u64::MAX, 25 << 30, 3 << 30, false),
            LeftoverPageAdvice::KeepDefault
        );
        assert_eq!(
            leftover_page_advice(1024, 1, 1, false),
            LeftoverPageAdvice::KeepDefault
        );
    }

    /// Fire 118 turned into a unit: a fitting store (hot) never drops,
    /// even with budget 0 and no covering SST.
    #[test]
    fn hot_store_never_drops() {
        // 2M @ 4 GiB leftover shape: store fits the cap.
        assert_eq!(
            leftover_page_advice(0, 490 << 20, 3 << 30, false),
            LeftoverPageAdvice::KeepHot
        );
        assert_eq!(
            leftover_page_advice(0, 3 << 30, 3 << 30, false),
            LeftoverPageAdvice::KeepHot,
            "boundary: sst_bytes == warm cap is still hot (cap is inclusive)"
        );
    }

    /// Fire-119 drop: bounded-cache (above cap) AND no covering SST AND
    /// budget 0 — the 25M @ 4 GiB leftover shape.
    #[test]
    fn bounded_uncovered_store_drops() {
        assert_eq!(
            leftover_page_advice(0, (3 << 30) + 1, 3 << 30, false),
            LeftoverPageAdvice::Drop
        );
    }

    /// A live SST covering the live family keeps pages even in
    /// bounded-cache mode with budget 0.
    #[test]
    fn covering_sst_keeps_pages() {
        assert_eq!(
            leftover_page_advice(0, 25 << 30, 3 << 30, true),
            LeftoverPageAdvice::KeepCovered
        );
    }

    /// AS-IS twin: today's engine keeps regardless.
    #[test]
    fn as_is_always_keeps() {
        assert_eq!(
            leftover_page_advice_as_is(0, 25 << 30, 3 << 30, false),
            LeftoverPageAdvice::KeepDefault
        );
    }

    /// Family intervals: `c/` covers `c/000123`, not `c0`, `ycsb/…`.
    #[test]
    fn family_upper_bound_splits_one_slash_families() {
        let c = family_upper_bound(b"c/").expect("slash terminates");
        assert_eq!(c, b"c0".to_vec());
        assert!(b"c/".as_slice() <= b"c/000123".as_slice());
        assert!(b"c/000123".as_slice() < c.as_slice());
        assert!(b"c.".as_slice() < b"c/".as_slice() && b"c/".as_slice() < c.as_slice());
        let ycsb = family_upper_bound(b"ycsb/").expect("slash terminates");
        assert_eq!(ycsb, b"ycsb0".to_vec());
        // All-0xFF prefix: unbounded (total function).
        assert_eq!(family_upper_bound(&[0xFF, 0xFF]), None);
    }

    /// Coverage: a `ycsb/`-ranged SST does not cover the live `c/` family;
    /// a `c/…`-ranged SST does. Unknown bounds never prove coverage.
    #[test]
    fn coverage_follows_key_ranges() {
        assert!(!sst_range_covers_family(
            Some(b"ycsb/000000".as_slice()),
            Some(b"ycsb/999999".as_slice()),
            b"c/"
        ));
        assert!(sst_range_covers_family(
            Some(b"c/000001".as_slice()),
            Some(b"c/000004".as_slice()),
            b"c/"
        ));
        // SST spanning families (mixed) covers the live family.
        assert!(sst_range_covers_family(
            Some(b"c/000001".as_slice()),
            Some(b"ycsb/000001".as_slice()),
            b"c/"
        ));
        // Entirely below the family's inclusive lower bound does not cover.
        assert!(!sst_range_covers_family(
            Some(b"a".as_slice()),
            Some(b"c/".as_slice()),
            b"c0"
        ));
        // Unknown bounds: conservative false.
        assert!(!sst_range_covers_family(None, None, b"c/"));
    }

    #[test]
    fn test_leftover_page_structural_invariants_red_to_green() {
        // 1. Display trait implementation
        assert_eq!(LeftoverPageAdvice::Drop.to_string(), "Drop");
        assert_eq!(LeftoverPageAdvice::KeepHot.to_string(), "KeepHot");
        assert_eq!(LeftoverPageAdvice::KeepCovered.to_string(), "KeepCovered");
        assert_eq!(LeftoverPageAdvice::KeepDefault.to_string(), "KeepDefault");

        // 2. Inverted range hazard: lo > hi (e.g. smallest = "c/999", largest = "c/111")
        // Under flawed logic, "c/111" >= "c/" and "c/999" < "c0" returned true!
        let inverted_lo = b"c/999".as_slice();
        let inverted_hi = b"c/111".as_slice();
        assert!(!sst_range_covers_family(Some(inverted_lo), Some(inverted_hi), b"c/"));

        let checked_err = sst_range_covers_family_checked(Some(inverted_lo), Some(inverted_hi), b"c/");
        assert_eq!(
            checked_err,
            Err(LeftoverPageError::InvertedRange {
                smallest: inverted_lo.to_vec(),
                largest: inverted_hi.to_vec(),
            })
        );

        // 3. Empty prefix hazard
        let empty_pfx_err = sst_range_covers_family_checked(
            Some(b"c/001".as_slice()),
            Some(b"c/002".as_slice()),
            b""
        );
        assert_eq!(empty_pfx_err, Err(LeftoverPageError::EmptyPrefix));

        // 4. Valid range works as expected
        let valid_cov = sst_range_covers_family_checked(
            Some(b"c/001".as_slice()),
            Some(b"c/002".as_slice()),
            b"c/"
        );
        assert_eq!(valid_cov, Ok(true));
    }
}
