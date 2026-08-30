// Verus proof of WAL recover choices (F4 / F14 / CRC).
// Twin of `src/wal/recover_kernel.rs`. Not linked into production.
//
//   ./scripts/verus_wal_recover.sh

use vstd::prelude::*;

verus! {

pub const MAX_CONSECUTIVE_SKIPS: u64 = 4 * 1024 * 1024;

pub enum RecoverKind {
    Record,
    CleanEof,
    Truncated,
    LengthCorrupt,
    UnknownType,
    OrphanFragment,
    Crc,
    Other,
    /// F170: zero type+len at a fresh alignment with junk after (not padding).
    ZeroHeaderTail,
}

pub enum RecoverAct {
    KeepRecord,
    Stop,
    Resync,
    KeepPrefix,
    FailStop,
}

pub enum PhysicalAct {
    Continue,
    FailStop,
    Truncated,
    CleanEof,
}

pub enum FragKind {
    Full,
    First,
    Middle,
    Last,
    Zero,
}

pub enum FragAct {
    Yield,
    Start,
    Accumulate,
    FailStop,
    Skip,
    CleanEof,
}

pub open spec fn is_length_resyncable_spec(kind: RecoverKind) -> bool {
    match kind {
        RecoverKind::Truncated => true,
        RecoverKind::LengthCorrupt => true,
        RecoverKind::UnknownType => true,
        _ => false,
    }
}

pub fn is_length_resyncable(kind: RecoverKind) -> (b: bool)
    ensures
        b == is_length_resyncable_spec(kind),
{
    match kind {
        RecoverKind::Truncated => true,
        RecoverKind::LengthCorrupt => true,
        RecoverKind::UnknownType => true,
        _ => false,
    }
}

pub open spec fn is_length_resyncable_as_is(kind: RecoverKind) -> bool {
    is_length_resyncable_spec(kind) || match kind {
        RecoverKind::Crc => true,
        _ => false,
    }
}

pub open spec fn recover_collect_act_spec(
    kind: RecoverKind,
    prefix_n: u64,
    can_skip: bool,
    consecutive_skips: u64,
    in_resync: bool,
) -> RecoverAct {
    match kind {
        RecoverKind::Record => RecoverAct::KeepRecord,
        RecoverKind::CleanEof => RecoverAct::Stop,
        RecoverKind::Truncated => recover_resync_act(prefix_n, can_skip, consecutive_skips),
        RecoverKind::LengthCorrupt => recover_resync_act(prefix_n, can_skip, consecutive_skips),
        RecoverKind::UnknownType => recover_resync_act(prefix_n, can_skip, consecutive_skips),
        RecoverKind::Crc => {
            if in_resync {
                recover_resync_act(prefix_n, can_skip, consecutive_skips)
            } else {
                RecoverAct::FailStop
            }
        },
        RecoverKind::ZeroHeaderTail => {
            if in_resync {
                recover_resync_act(prefix_n, can_skip, consecutive_skips)
            } else {
                RecoverAct::FailStop
            }
        },
        RecoverKind::OrphanFragment => RecoverAct::FailStop,
        RecoverKind::Other => RecoverAct::FailStop,
    }
}

pub open spec fn recover_resync_act(prefix_n: u64, can_skip: bool, consecutive_skips: u64) -> RecoverAct {
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

pub fn recover_collect_act(
    kind: RecoverKind,
    prefix_n: u64,
    can_skip: bool,
    consecutive_skips: u64,
    in_resync: bool,
) -> (d: RecoverAct)
    ensures
        d == recover_collect_act_spec(
            kind,
            prefix_n,
            can_skip,
            consecutive_skips,
            in_resync,
        ),
{
    match kind {
        RecoverKind::Record => RecoverAct::KeepRecord,
        RecoverKind::CleanEof => RecoverAct::Stop,
        RecoverKind::Truncated => {
            recover_resync_exec(prefix_n, can_skip, consecutive_skips)
        },
        RecoverKind::LengthCorrupt => {
            recover_resync_exec(prefix_n, can_skip, consecutive_skips)
        },
        RecoverKind::UnknownType => {
            recover_resync_exec(prefix_n, can_skip, consecutive_skips)
        },
        RecoverKind::Crc => {
            if in_resync {
                recover_resync_exec(prefix_n, can_skip, consecutive_skips)
            } else {
                RecoverAct::FailStop
            }
        },
        RecoverKind::ZeroHeaderTail => {
            if in_resync {
                recover_resync_exec(prefix_n, can_skip, consecutive_skips)
            } else {
                RecoverAct::FailStop
            }
        },
        RecoverKind::OrphanFragment => RecoverAct::FailStop,
        RecoverKind::Other => RecoverAct::FailStop,
    }
}

fn recover_resync_exec(prefix_n: u64, can_skip: bool, consecutive_skips: u64) -> (d: RecoverAct)
    ensures
        d == recover_resync_act(prefix_n, can_skip, consecutive_skips),
{
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

pub open spec fn recover_collect_act_as_is(
    kind: RecoverKind,
    _prefix_n: u64,
    _can_skip: bool,
    _consecutive_skips: u64,
) -> RecoverAct {
    match kind {
        RecoverKind::Record => RecoverAct::KeepRecord,
        RecoverKind::CleanEof => RecoverAct::Stop,
        RecoverKind::Truncated => RecoverAct::Stop,
        RecoverKind::LengthCorrupt => RecoverAct::Stop,
        RecoverKind::UnknownType => RecoverAct::Stop,
        RecoverKind::Crc => RecoverAct::Resync,
        RecoverKind::ZeroHeaderTail => RecoverAct::Stop,
        RecoverKind::OrphanFragment => RecoverAct::FailStop,
        RecoverKind::Other => RecoverAct::FailStop,
    }
}

pub fn physical_payload_act(
    length: u64,
    max_payload: u64,
    payload_end: u64,
    block_end: u64,
    block_size: u64,
) -> (d: PhysicalAct)
    ensures
        (length > max_payload) ==> d == PhysicalAct::FailStop,
        (length <= max_payload && payload_end > block_end && block_end == block_size) ==> d == PhysicalAct::FailStop,
        (length <= max_payload && payload_end > block_end && block_end != block_size) ==> d == PhysicalAct::Truncated,
        (length <= max_payload && payload_end <= block_end) ==> d == PhysicalAct::Continue,
{
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

pub open spec fn physical_payload_act_as_is(
    length: u64,
    max_payload: u64,
    payload_end: u64,
    block_end: u64,
    _block_size: u64,
) -> PhysicalAct {
    if length > max_payload || payload_end > block_end {
        PhysicalAct::CleanEof
    } else {
        PhysicalAct::Continue
    }
}

pub fn fragment_act(kind: FragKind, scratch_empty: bool) -> (d: FragAct)
    ensures
        kind is Full ==> d == FragAct::Yield,
        kind is First ==> d == FragAct::Start,
        kind is Zero ==> d == FragAct::Skip,
        (kind is Middle && scratch_empty) ==> d == FragAct::FailStop,
        (kind is Middle && !scratch_empty) ==> d == FragAct::Accumulate,
        (kind is Last && scratch_empty) ==> d == FragAct::FailStop,
        (kind is Last && !scratch_empty) ==> d == FragAct::Yield,
{
    match kind {
        FragKind::Full => FragAct::Yield,
        FragKind::First => FragAct::Start,
        FragKind::Middle => {
            if scratch_empty {
                FragAct::FailStop
            } else {
                FragAct::Accumulate
            }
        },
        FragKind::Last => {
            if scratch_empty {
                FragAct::FailStop
            } else {
                FragAct::Yield
            }
        },
        FragKind::Zero => FragAct::Skip,
    }
}

pub open spec fn fragment_act_as_is(kind: FragKind, scratch_empty: bool) -> FragAct {
    if scratch_empty && (kind is Middle || kind is Last) {
        FragAct::CleanEof
    } else {
        match kind {
            FragKind::Full => FragAct::Yield,
            FragKind::First => FragAct::Start,
            FragKind::Middle => FragAct::Accumulate,
            FragKind::Last => FragAct::Yield,
            FragKind::Zero => FragAct::Skip,
        }
    }
}

proof fn lemma_as_is_torn_is_silent_eof()
    ensures
        recover_collect_act_spec(RecoverKind::Truncated, 0, false, 0, false)
            == RecoverAct::FailStop,
        recover_collect_act_as_is(RecoverKind::Truncated, 0, false, 0) == RecoverAct::Stop,
        recover_collect_act_spec(RecoverKind::LengthCorrupt, 0, false, 0, false)
            == RecoverAct::FailStop,
        recover_collect_act_as_is(RecoverKind::LengthCorrupt, 0, false, 0) == RecoverAct::Stop,
{
}

proof fn lemma_as_is_crc_resyncs()
    ensures
        recover_collect_act_spec(RecoverKind::Crc, 3, true, 0, false) == RecoverAct::FailStop,
        recover_collect_act_as_is(RecoverKind::Crc, 3, true, 0) == RecoverAct::Resync,
        !is_length_resyncable_spec(RecoverKind::Crc),
        is_length_resyncable_as_is(RecoverKind::Crc),
{
}

/// RFC-0053 P1.3 / crash-dictionary: AS-IS swallows ZeroHeaderTail as EOF.
proof fn lemma_as_is_zero_header_silent_eof()
    ensures
        recover_collect_act_spec(RecoverKind::ZeroHeaderTail, 3, true, 0, false)
            == RecoverAct::FailStop,
        recover_collect_act_as_is(RecoverKind::ZeroHeaderTail, 3, true, 0) == RecoverAct::Stop,
{
}

proof fn lemma_as_is_orphan_is_eof()
    ensures
        fragment_act_as_is(FragKind::Middle, true) == FragAct::CleanEof,
        fragment_act_as_is(FragKind::Last, true) == FragAct::CleanEof,
{
}

proof fn lemma_crc_fresh_alignment_fail_stops(prefix_n: u64, can_skip: bool, skips: u64)
    ensures
        recover_collect_act_spec(RecoverKind::Crc, prefix_n, can_skip, skips, false)
            == RecoverAct::FailStop,
        recover_collect_act_spec(
            RecoverKind::ZeroHeaderTail,
            prefix_n,
            can_skip,
            skips,
            false,
        ) == RecoverAct::FailStop,
        recover_collect_act_spec(RecoverKind::OrphanFragment, prefix_n, can_skip, skips, false)
            == RecoverAct::FailStop,
{
}

proof fn lemma_empty_torn_fail_stops()
    ensures
        recover_collect_act_spec(RecoverKind::Truncated, 0, false, 0, false)
            == RecoverAct::FailStop,
        recover_collect_act_spec(RecoverKind::UnknownType, 0, false, 0, false)
            == RecoverAct::FailStop,
{
}

proof fn lemma_prefix_torn_keeps()
    ensures
        recover_collect_act_spec(RecoverKind::Truncated, 1, false, 0, false)
            == RecoverAct::KeepPrefix,
{
}

} // verus!
