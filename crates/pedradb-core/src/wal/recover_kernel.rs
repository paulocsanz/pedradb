//! Pure WAL recover choices (F4 / F14 / EXPLODE).
//!
//! Production [`crate::wal::reader::WalReader::collect_all`] and
//! [`crate::wal::reader::WalReader::read_record`] call these. Bytes on disk,
//! torn writes, and fsync are **caller + axiom**.
//!
//! Named decisions (the ones that were `SilentWrong` when inverted):
//! - length / torn / unknown-type may **resync**; CRC and orphan **fail-stop**
//! - first-record torn on an otherwise empty prefix is **fail-stop**, not a
//!   silent empty WAL (F4)
//! - orphan `Middle` / `Last` is **fail-stop**, not clean EOF (F14)

#![forbid(unsafe_code)]

use super::format::RecordType;

/// Consecutive one-byte resync steps before `collect_all` fail-stops.
pub const MAX_CONSECUTIVE_SKIPS: u64 = 4 * 1024 * 1024;

/// What [`crate::wal::reader::WalReader::read_record`] just observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoverKind {
    /// A complete logical record (`Ok(Some)`).
    Record,
    /// Clean end of log (`Ok(None)`).
    CleanEof,
    /// Header parsed, payload cut by a short final block.
    Truncated,
    /// Length exceeds max payload or remainder of a full block.
    LengthCorrupt,
    /// Type byte is not a known [`RecordType`].
    UnknownType,
    /// `Middle` / `Last` with an empty scratch (no matching `First`).
    OrphanFragment,
    /// Stored CRC ≠ recomputed CRC.
    Crc,
    /// I/O or other internal — always fail-stop.
    Other,
}

/// What `collect_all` does with one observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoverAct {
    /// Append the record and continue.
    KeepRecord,
    /// Clean EOF — return the prefix collected so far.
    Stop,
    /// Advance one byte and try a new alignment.
    Resync,
    /// Stream exhausted after a resyncable error; keep a non-empty prefix.
    KeepPrefix,
    /// Return the error (CRC, orphan, empty torn WAL, budget).
    FailStop,
}

/// Physical payload vs block bounds (F4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhysicalAct {
    /// Header + payload sit inside the bytes we have; check CRC next.
    Continue,
    /// Oversize length or length past a full block — fail-stop, not EOF.
    FailStop,
    /// Payload past a short final block — torn tail.
    Truncated,
    /// AS-IS F4 only: pretend this is clean EOF.
    CleanEof,
}

/// First / Middle / Last / Full / Zero once the CRC has passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FragKind {
    /// Single-fragment logical record.
    Full,
    /// Start of a multi-fragment record.
    First,
    /// Interior fragment.
    Middle,
    /// Final fragment.
    Last,
    /// Non-padding zero (length ≠ 0) — skip.
    Zero,
}

/// What to do with a CRC-valid physical fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FragAct {
    /// Return a complete logical record (`Full` or completing `Last`).
    Yield,
    /// Begin a new scratch (`First`).
    Start,
    /// Append to scratch (`Middle`).
    Accumulate,
    /// Orphan `Middle` / `Last` — fail-stop (F14).
    FailStop,
    /// Non-padding zero — skip.
    Skip,
    /// AS-IS F14 only: orphan looks like clean EOF.
    CleanEof,
}

impl FragKind {
    /// Map the on-disk type byte enum.
    #[must_use]
    pub fn from_record_type(t: RecordType) -> Self {
        match t {
            RecordType::Zero => Self::Zero,
            RecordType::Full => Self::Full,
            RecordType::First => Self::First,
            RecordType::Middle => Self::Middle,
            RecordType::Last => Self::Last,
        }
    }
}

/// F4: length / framing / unknown type may resync. CRC and orphan do not.
#[must_use]
pub fn is_length_resyncable(kind: RecoverKind) -> bool {
    matches!(
        kind,
        RecoverKind::Truncated | RecoverKind::LengthCorrupt | RecoverKind::UnknownType
    )
}

/// AS-IS dense-sweep `SilentWrong`: also resync on CRC (false alignments skip
/// durable later records).
#[must_use]
pub fn is_length_resyncable_as_is(kind: RecoverKind) -> bool {
    is_length_resyncable(kind) || matches!(kind, RecoverKind::Crc)
}

/// What `collect_all` does after one `read_record` outcome.
///
/// `consecutive_skips` is the count **including** this skip when `can_skip`.
#[must_use]
pub fn recover_collect_act(
    kind: RecoverKind,
    prefix_n: u64,
    can_skip: bool,
    consecutive_skips: u64,
) -> RecoverAct {
    match kind {
        RecoverKind::Record => RecoverAct::KeepRecord,
        RecoverKind::CleanEof => RecoverAct::Stop,
        RecoverKind::Truncated | RecoverKind::LengthCorrupt | RecoverKind::UnknownType => {
            if !can_skip {
                if prefix_n == 0 {
                    RecoverAct::FailStop
                } else {
                    RecoverAct::KeepPrefix
                }
            } else if consecutive_skips > MAX_CONSECUTIVE_SKIPS {
                RecoverAct::FailStop
            } else {
                RecoverAct::Resync
            }
        }
        RecoverKind::Crc | RecoverKind::OrphanFragment | RecoverKind::Other => RecoverAct::FailStop,
    }
}

/// AS-IS F4 + CRC `SilentWrong`: torn/length look like clean EOF; CRC resyncs.
#[must_use]
pub fn recover_collect_act_as_is(
    kind: RecoverKind,
    _prefix_n: u64,
    _can_skip: bool,
    _consecutive_skips: u64,
) -> RecoverAct {
    match kind {
        RecoverKind::Record => RecoverAct::KeepRecord,
        RecoverKind::CleanEof
        | RecoverKind::Truncated
        | RecoverKind::LengthCorrupt
        | RecoverKind::UnknownType => RecoverAct::Stop,
        RecoverKind::Crc => RecoverAct::Resync,
        RecoverKind::OrphanFragment | RecoverKind::Other => RecoverAct::FailStop,
    }
}

/// F4: oversize / full-block overrun fail-stop; short-block overrun is torn.
#[must_use]
pub fn physical_payload_act(
    length: u64,
    max_payload: u64,
    payload_end: u64,
    block_end: u64,
    block_size: u64,
) -> PhysicalAct {
    if length > max_payload {
        return PhysicalAct::FailStop;
    }
    if payload_end > block_end {
        if block_end == block_size {
            return PhysicalAct::FailStop;
        }
        return PhysicalAct::Truncated;
    }
    PhysicalAct::Continue
}

/// AS-IS F4: oversize / torn look like clean EOF (silently drop the rest).
#[must_use]
pub fn physical_payload_act_as_is(
    length: u64,
    max_payload: u64,
    payload_end: u64,
    block_end: u64,
    _block_size: u64,
) -> PhysicalAct {
    if length > max_payload || payload_end > block_end {
        return PhysicalAct::CleanEof;
    }
    PhysicalAct::Continue
}

/// F14: orphan Middle/Last fail-stop; otherwise assemble.
#[must_use]
pub fn fragment_act(kind: FragKind, scratch_empty: bool) -> FragAct {
    match kind {
        FragKind::Full => FragAct::Yield,
        FragKind::First => FragAct::Start,
        FragKind::Middle => {
            if scratch_empty {
                FragAct::FailStop
            } else {
                FragAct::Accumulate
            }
        }
        FragKind::Last => {
            if scratch_empty {
                FragAct::FailStop
            } else {
                FragAct::Yield
            }
        }
        FragKind::Zero => FragAct::Skip,
    }
}

/// AS-IS F14: orphan Middle/Last looks like clean EOF.
#[must_use]
pub fn fragment_act_as_is(kind: FragKind, scratch_empty: bool) -> FragAct {
    if scratch_empty && matches!(kind, FragKind::Middle | FragKind::Last) {
        FragAct::CleanEof
    } else {
        fragment_act(kind, scratch_empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resync_only_length_class() {
        assert!(is_length_resyncable(RecoverKind::Truncated));
        assert!(is_length_resyncable(RecoverKind::LengthCorrupt));
        assert!(is_length_resyncable(RecoverKind::UnknownType));
        assert!(!is_length_resyncable(RecoverKind::Crc));
        assert!(!is_length_resyncable(RecoverKind::OrphanFragment));
        assert!(is_length_resyncable_as_is(RecoverKind::Crc));
        assert_ne!(
            is_length_resyncable(RecoverKind::Crc),
            is_length_resyncable_as_is(RecoverKind::Crc)
        );
    }

    #[test]
    fn f4_empty_torn_is_fail_stop_not_silent_eof() {
        assert_eq!(
            recover_collect_act(RecoverKind::Truncated, 0, false, 0),
            RecoverAct::FailStop
        );
        assert_eq!(
            recover_collect_act_as_is(RecoverKind::Truncated, 0, false, 0),
            RecoverAct::Stop
        );
        assert_eq!(
            recover_collect_act(RecoverKind::LengthCorrupt, 0, false, 0),
            RecoverAct::FailStop
        );
        assert_eq!(
            recover_collect_act_as_is(RecoverKind::LengthCorrupt, 0, false, 0),
            RecoverAct::Stop
        );
    }

    #[test]
    fn torn_after_prefix_keeps_prefix() {
        assert_eq!(
            recover_collect_act(RecoverKind::Truncated, 1, false, 0),
            RecoverAct::KeepPrefix
        );
    }

    #[test]
    fn crc_and_orphan_fail_stop() {
        assert_eq!(
            recover_collect_act(RecoverKind::Crc, 3, true, 0),
            RecoverAct::FailStop
        );
        assert_eq!(
            recover_collect_act(RecoverKind::OrphanFragment, 3, true, 0),
            RecoverAct::FailStop
        );
        assert_eq!(
            recover_collect_act_as_is(RecoverKind::Crc, 3, true, 0),
            RecoverAct::Resync
        );
    }

    #[test]
    fn resync_budget() {
        assert_eq!(
            recover_collect_act(RecoverKind::Truncated, 1, true, MAX_CONSECUTIVE_SKIPS),
            RecoverAct::Resync
        );
        assert_eq!(
            recover_collect_act(RecoverKind::Truncated, 1, true, MAX_CONSECUTIVE_SKIPS + 1),
            RecoverAct::FailStop
        );
    }

    #[test]
    fn physical_oversize_and_torn() {
        let max = 32_768 - 7;
        assert_eq!(
            physical_payload_act(max + 1, max, 0, 100, 32_768),
            PhysicalAct::FailStop
        );
        assert_eq!(
            physical_payload_act(10, max, 32_800, 32_768, 32_768),
            PhysicalAct::FailStop
        );
        assert_eq!(
            physical_payload_act(10, max, 40, 30, 32_768),
            PhysicalAct::Truncated
        );
        assert_eq!(
            physical_payload_act(10, max, 20, 30, 32_768),
            PhysicalAct::Continue
        );
        assert_eq!(
            physical_payload_act_as_is(max + 1, max, 0, 100, 32_768),
            PhysicalAct::CleanEof
        );
        assert_eq!(
            physical_payload_act_as_is(10, max, 40, 30, 32_768),
            PhysicalAct::CleanEof
        );
    }

    #[test]
    fn orphan_middle_last_fail_stop() {
        assert_eq!(fragment_act(FragKind::Middle, true), FragAct::FailStop);
        assert_eq!(fragment_act(FragKind::Last, true), FragAct::FailStop);
        assert_eq!(fragment_act(FragKind::Middle, false), FragAct::Accumulate);
        assert_eq!(fragment_act(FragKind::Last, false), FragAct::Yield);
        assert_eq!(
            fragment_act_as_is(FragKind::Middle, true),
            FragAct::CleanEof
        );
        assert_eq!(fragment_act_as_is(FragKind::Last, true), FragAct::CleanEof);
        assert_ne!(
            fragment_act(FragKind::Middle, true),
            fragment_act_as_is(FragKind::Middle, true)
        );
    }

    #[test]
    fn theorem_on_small_domain() {
        let kinds = [
            RecoverKind::Record,
            RecoverKind::CleanEof,
            RecoverKind::Truncated,
            RecoverKind::LengthCorrupt,
            RecoverKind::UnknownType,
            RecoverKind::OrphanFragment,
            RecoverKind::Crc,
            RecoverKind::Other,
        ];
        let mut n = 0u32;
        for kind in kinds {
            for prefix_n in [0u64, 1, 7] {
                for can_skip in [false, true] {
                    for skips in [0u64, 1, MAX_CONSECUTIVE_SKIPS, MAX_CONSECUTIVE_SKIPS + 1] {
                        let d = recover_collect_act(kind, prefix_n, can_skip, skips);
                        match kind {
                            RecoverKind::Record => assert_eq!(d, RecoverAct::KeepRecord),
                            RecoverKind::CleanEof => assert_eq!(d, RecoverAct::Stop),
                            RecoverKind::Crc | RecoverKind::OrphanFragment | RecoverKind::Other => {
                                assert_eq!(d, RecoverAct::FailStop);
                            }
                            RecoverKind::Truncated
                            | RecoverKind::LengthCorrupt
                            | RecoverKind::UnknownType => {
                                if !can_skip {
                                    if prefix_n == 0 {
                                        assert_eq!(d, RecoverAct::FailStop);
                                    } else {
                                        assert_eq!(d, RecoverAct::KeepPrefix);
                                    }
                                } else if skips > MAX_CONSECUTIVE_SKIPS {
                                    assert_eq!(d, RecoverAct::FailStop);
                                } else {
                                    assert_eq!(d, RecoverAct::Resync);
                                }
                            }
                        }
                        n += 1;
                    }
                }
            }
        }
        assert_eq!(n, 8 * 3 * 2 * 4);
    }
}
