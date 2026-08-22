//! EXPLODE-style recover *choices* (OSDI’06): named mutations of a WAL image.
//!
//! This is not a second recover policy. Production
//! [`crate::wal::reader::WalReader::collect_all`] already calls
//! [`super::recover_kernel`]. These helpers only **inject** the observations
//! (torn tail, CRC, length, orphan, unknown type) so a sweep can drive the
//! real reader. Persist / fsync stay axioms.

#![forbid(unsafe_code)]

use super::crc::record_checksum;
use super::format::{decode_length, RecordType, BLOCK_SIZE, HEADER_SIZE};
use super::recover_kernel::RecoverKind;

/// One recover injection. Applied to an in-memory WAL image (no I/O).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoverChoice {
    /// Leave the image alone.
    Clean,
    /// Drop the last `bytes` of the file (crash torn tail).
    TearTail {
        /// How many trailing bytes to drop.
        bytes: usize,
    },
    /// XOR the first payload byte of physical record `index` (CRC mismatch).
    FlipCrc {
        /// 0-based physical record.
        index: usize,
    },
    /// XOR the length high-byte of physical record `index` (F4).
    FlipLength {
        /// 0-based physical record.
        index: usize,
    },
    /// Rewrite type to `Middle` and restamp CRC (F14 orphan).
    ForgeOrphanMiddle {
        /// 0-based physical record.
        index: usize,
    },
    /// Rewrite type to `0x7f` (unknown type, resyncable).
    ForgeUnknownType {
        /// 0-based physical record.
        index: usize,
    },
    /// Append `bytes` of pure zeros (prealloc / post-crash padding shape).
    /// A zero tail is legitimate padding — the reader must recover every
    /// record and stop cleanly (F170 kept this distinct from a *live* zero
    /// header).
    ZeroTail {
        /// How many trailing zero bytes to append.
        bytes: usize,
    },
    /// Zero the length+type of record `index` while its payload (and any
    /// later records) stay live in the same block — the F170 shape the
    /// AS-IS reader swallowed as block padding, losing the rest of the log.
    ForgeZeroHeaderAlive {
        /// 0-based physical record.
        index: usize,
    },
}

/// Bounded EXPLODE panel for a WAL with `n_records` physical Full records.
#[must_use]
pub fn explode_choices(n_records: usize) -> Vec<RecoverChoice> {
    let mut out = vec![
        RecoverChoice::Clean,
        RecoverChoice::TearTail { bytes: 5 },
        RecoverChoice::ZeroTail { bytes: 1024 },
    ];
    for index in 0..n_records {
        out.push(RecoverChoice::FlipCrc { index });
        out.push(RecoverChoice::FlipLength { index });
        out.push(RecoverChoice::ForgeZeroHeaderAlive { index });
        if index > 0 {
            out.push(RecoverChoice::ForgeOrphanMiddle { index });
            out.push(RecoverChoice::ForgeUnknownType { index });
        }
    }
    out
}

/// Mutate `buf` in place. Returns `false` when the choice cannot apply
/// (index past the last physical record, or empty image).
#[must_use]
pub fn apply_recover_choice(buf: &mut Vec<u8>, choice: RecoverChoice) -> bool {
    match choice {
        RecoverChoice::Clean => true,
        RecoverChoice::TearTail { bytes } => {
            if buf.is_empty() {
                return false;
            }
            let keep = buf.len().saturating_sub(bytes);
            buf.truncate(keep);
            true
        }
        RecoverChoice::FlipCrc { index } => {
            let Some((h, len, _)) = nth_phys(buf, index) else {
                return false;
            };
            if len == 0 || h + HEADER_SIZE >= buf.len() {
                return false;
            }
            buf[h + HEADER_SIZE] ^= 0xff;
            true
        }
        RecoverChoice::FlipLength { index } => {
            let Some((h, _, _)) = nth_phys(buf, index) else {
                return false;
            };
            buf[h + 5] ^= 0x01;
            true
        }
        RecoverChoice::ForgeOrphanMiddle { index } => {
            let Some((h, len, _)) = nth_phys(buf, index) else {
                return false;
            };
            let Ok(len_u16) = u16::try_from(len) else {
                return false;
            };
            let end = h + HEADER_SIZE + len;
            if end > buf.len() {
                return false;
            }
            buf[h + 6] = RecordType::Middle as u8;
            let payload = buf[h + HEADER_SIZE..end].to_vec();
            let crc = record_checksum(RecordType::Middle as u8, len_u16, &payload);
            buf[h..h + 4].copy_from_slice(&crc.to_le_bytes());
            true
        }
        RecoverChoice::ForgeUnknownType { index } => {
            let Some((h, _, _)) = nth_phys(buf, index) else {
                return false;
            };
            buf[h + 6] = 0x7f;
            true
        }
        RecoverChoice::ZeroTail { bytes } => {
            buf.extend(std::iter::repeat_n(0u8, bytes));
            true
        }
        RecoverChoice::ForgeZeroHeaderAlive { index } => {
            let Some((h, len, _)) = nth_phys(buf, index) else {
                return false;
            };
            // Length + type zeroed; CRC bytes stay — the zero-header check
            // fires before either is consulted. The record's own payload
            // (and later records) remain as the live bytes after it.
            if h + HEADER_SIZE + len > buf.len() {
                return false;
            }
            buf[h + 4] = 0;
            buf[h + 5] = 0;
            buf[h + 6] = 0;
            true
        }
    }
}

/// Kind the injection is *trying* to produce (first faulting observation).
#[must_use]
pub fn injected_kind(choice: RecoverChoice) -> Option<RecoverKind> {
    match choice {
        RecoverChoice::Clean | RecoverChoice::ZeroTail { .. } => None,
        RecoverChoice::TearTail { .. } | RecoverChoice::FlipLength { .. } => {
            Some(RecoverKind::Truncated)
        }
        RecoverChoice::FlipCrc { .. } => Some(RecoverKind::Crc),
        RecoverChoice::ForgeOrphanMiddle { .. } => Some(RecoverKind::OrphanFragment),
        RecoverChoice::ForgeUnknownType { .. } => Some(RecoverKind::UnknownType),
        RecoverChoice::ForgeZeroHeaderAlive { .. } => Some(RecoverKind::ZeroHeaderTail),
    }
}

/// What production `collect_all` must do after this injection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChooseExpect {
    /// Image is intact — every logical record comes back.
    AllRecords,
    /// CRC / orphan — return `Err`, never a later suffix.
    FailStop,
    /// Torn tail — keep a non-empty decoded prefix, drop the incomplete last.
    PrefixOnly,
    /// Length / unknown type — resync; never silently empty when a prefix existed.
    Resync,
}

/// Terminal `collect_all` contract for one choice.
#[must_use]
pub fn choose_expect(choice: RecoverChoice) -> ChooseExpect {
    match choice {
        RecoverChoice::Clean | RecoverChoice::ZeroTail { .. } => ChooseExpect::AllRecords,
        RecoverChoice::TearTail { .. } => ChooseExpect::PrefixOnly,
        RecoverChoice::FlipCrc { .. }
        | RecoverChoice::ForgeOrphanMiddle { .. }
        | RecoverChoice::ForgeZeroHeaderAlive { .. } => ChooseExpect::FailStop,
        RecoverChoice::FlipLength { .. } | RecoverChoice::ForgeUnknownType { .. } => {
            ChooseExpect::Resync
        }
    }
}

fn nth_phys(buf: &[u8], n: usize) -> Option<(usize, usize, u8)> {
    let mut off = 0usize;
    let mut i = 0usize;
    loop {
        let (h, len, typ) = next_phys(buf, off)?;
        if i == n {
            return Some((h, len, typ));
        }
        i += 1;
        off = h.checked_add(HEADER_SIZE)?.checked_add(len)?;
    }
}

fn next_phys(buf: &[u8], start: usize) -> Option<(usize, usize, u8)> {
    let mut o = start;
    loop {
        if o + HEADER_SIZE > buf.len() {
            return None;
        }
        let typ = buf[o + 6];
        let len = decode_length([buf[o + 4], buf[o + 5]]);
        if typ == RecordType::Zero as u8 && len == 0 {
            let block = o / BLOCK_SIZE;
            let block_end = block.saturating_add(1).saturating_mul(BLOCK_SIZE);
            if block_end >= buf.len() || block_end <= o {
                return None;
            }
            o = block_end;
            continue;
        }
        return Some((o, len, typ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expect_matches_named_faults() {
        assert_eq!(
            choose_expect(RecoverChoice::Clean),
            ChooseExpect::AllRecords
        );
        assert_eq!(
            choose_expect(RecoverChoice::FlipCrc { index: 1 }),
            ChooseExpect::FailStop
        );
        assert_eq!(
            choose_expect(RecoverChoice::ForgeOrphanMiddle { index: 1 }),
            ChooseExpect::FailStop
        );
        assert_eq!(
            choose_expect(RecoverChoice::FlipLength { index: 1 }),
            ChooseExpect::Resync
        );
        assert_eq!(
            choose_expect(RecoverChoice::TearTail { bytes: 5 }),
            ChooseExpect::PrefixOnly
        );
        assert_eq!(
            injected_kind(RecoverChoice::ForgeUnknownType { index: 1 }),
            Some(RecoverKind::UnknownType)
        );
    }

    #[test]
    fn explode_panel_covers_named_faults() {
        let c = explode_choices(3);
        assert!(c.contains(&RecoverChoice::Clean));
        assert!(c.contains(&RecoverChoice::TearTail { bytes: 5 }));
        assert!(c.contains(&RecoverChoice::ZeroTail { bytes: 1024 }));
        assert!(c.contains(&RecoverChoice::FlipCrc { index: 0 }));
        assert!(c.contains(&RecoverChoice::ForgeZeroHeaderAlive { index: 0 }));
        assert!(c.contains(&RecoverChoice::ForgeZeroHeaderAlive { index: 2 }));
        assert!(c.contains(&RecoverChoice::ForgeOrphanMiddle { index: 1 }));
        assert!(!c.contains(&RecoverChoice::ForgeOrphanMiddle { index: 0 }));
        // 3 base + per-record (crc, length, zero-header-alive) + mid-only (orphan, unknown)
        assert_eq!(c.len(), 3 + 3 * 3 + 2 * 2);
    }

    #[test]
    fn new_kinds_map_to_their_contract() {
        assert_eq!(
            injected_kind(RecoverChoice::ZeroTail { bytes: 64 }),
            None,
            "pure zero tail is padding, not a fault"
        );
        assert_eq!(
            injected_kind(RecoverChoice::ForgeZeroHeaderAlive { index: 1 }),
            Some(RecoverKind::ZeroHeaderTail)
        );
        assert_eq!(
            choose_expect(RecoverChoice::ZeroTail { bytes: 64 }),
            ChooseExpect::AllRecords
        );
        assert_eq!(
            choose_expect(RecoverChoice::ForgeZeroHeaderAlive { index: 1 }),
            ChooseExpect::FailStop
        );
    }
}
