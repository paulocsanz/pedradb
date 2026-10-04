//! RFC-0310: Torn-write recovery kernel and resilient frame parser.
//!
//! Provides mathematically bounded parsing and crash-recovery guarantees for WAL
//! and block-level framing under partial power loss and torn sector writes.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TornWriteError {
    TruncatedHeader { needed: usize, available: usize },
    InvalidMagic { expected: [u8; 4], found: [u8; 4] },
    TruncatedPayload { needed: usize, available: usize },
    CrcMismatch { expected: u32, calculated: u32 },
    /// Empty payload provided
    EmptyPayload,
    /// Zero sequence number provided
    ZeroSequence,
    /// Payload length exceeds safe maximum
    PayloadExceedsMaxBound { len: usize, max: usize },
    /// Non-monotonic sequence number detected during recovery
    NonMonotonicSequence { previous: u64, current: u64 },
}

impl std::fmt::Display for TornWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TruncatedHeader { needed, available } => {
                write!(f, "truncated header: needed {needed} bytes, got {available}")
            }
            Self::InvalidMagic { expected, found } => {
                write!(f, "invalid magic: expected {expected:?}, found {found:?}")
            }
            Self::TruncatedPayload { needed, available } => {
                write!(f, "truncated payload: needed {needed} bytes, got {available}")
            }
            Self::CrcMismatch { expected, calculated } => {
                write!(f, "crc mismatch: expected {expected:#x}, got {calculated:#x}")
            }
            Self::EmptyPayload => write!(f, "frame payload cannot be empty"),
            Self::ZeroSequence => write!(f, "frame sequence number cannot be zero"),
            Self::PayloadExceedsMaxBound { len, max } => {
                write!(f, "frame payload length {len} exceeds max bound {max}")
            }
            Self::NonMonotonicSequence { previous, current } => {
                write!(
                    f,
                    "non-monotonic frame sequence: previous {previous} >= current {current}"
                )
            }
        }
    }
}

impl std::error::Error for TornWriteError {}

pub const FRAME_MAGIC: [u8; 4] = *b"PDR1";
pub const HEADER_LEN: usize = 20; // 4 magic + 4 len + 8 seq + 4 crc
pub const MAX_PAYLOAD_LEN: usize = 64 * 1024 * 1024; // 64 MiB

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TornWriteFrame {
    pub seq: u64,
    pub crc32c: u32,
    pub payload: Vec<u8>,
}

#[allow(dead_code)]
impl TornWriteFrame {
    /// Creates a validated TornWriteFrame.
    pub(crate) fn try_new(seq: u64, payload: Vec<u8>) -> Result<Self, TornWriteError> {
        if seq == 0 {
            return Err(TornWriteError::ZeroSequence);
        }
        if payload.is_empty() {
            return Err(TornWriteError::EmptyPayload);
        }
        if payload.len() > MAX_PAYLOAD_LEN {
            return Err(TornWriteError::PayloadExceedsMaxBound {
                len: payload.len(),
                max: MAX_PAYLOAD_LEN,
            });
        }
        let crc = crc32c::crc32c(&payload);
        Ok(Self {
            seq,
            crc32c: crc,
            payload,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredLogReport {
    pub frames: Vec<TornWriteFrame>,
    pub clean_offset: usize,
    pub torn_detected: bool,
    pub reason: Option<TornWriteError>,
}

#[allow(dead_code)]
impl RecoveredLogReport {
    #[must_use]
    pub(crate) fn is_clean(&self) -> bool {
        !self.torn_detected && self.reason.is_none()
    }

    #[must_use]
    pub(crate) fn frame_count(&self) -> usize {
        self.frames.len()
    }

    #[must_use]
    pub(crate) fn last_sequence(&self) -> Option<u64> {
        self.frames.last().map(|f| f.seq)
    }
}

/// Encodes a single atomic frame with framing metadata and CRC32C.
pub fn encode_frame(seq: u64, payload: &[u8]) -> Vec<u8> {
    let payload_len = payload.len() as u32;
    let crc = crc32c::crc32c(payload);

    let mut buf = Vec::with_capacity(HEADER_LEN + payload.len());
    buf.extend_from_slice(&FRAME_MAGIC);
    buf.extend_from_slice(&payload_len.to_le_bytes());
    buf.extend_from_slice(&seq.to_le_bytes());
    buf.extend_from_slice(&crc.to_le_bytes());
    buf.extend_from_slice(payload);
    buf
}

/// Validates and encodes a single atomic frame.
#[allow(dead_code)]
pub(crate) fn try_encode_frame(seq: u64, payload: &[u8]) -> Result<Vec<u8>, TornWriteError> {
    if seq == 0 {
        return Err(TornWriteError::ZeroSequence);
    }
    if payload.is_empty() {
        return Err(TornWriteError::EmptyPayload);
    }
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(TornWriteError::PayloadExceedsMaxBound {
            len: payload.len(),
            max: MAX_PAYLOAD_LEN,
        });
    }
    Ok(encode_frame(seq, payload))
}

/// Attempts to decode a single frame from the start of `buf`.
/// Returns `Ok((frame, consumed_bytes))` or `Err(TornWriteError)`.
pub fn decode_single_frame(buf: &[u8]) -> Result<(TornWriteFrame, usize), TornWriteError> {
    if buf.len() < HEADER_LEN {
        return Err(TornWriteError::TruncatedHeader {
            needed: HEADER_LEN,
            available: buf.len(),
        });
    }

    let mut magic = [0u8; 4];
    magic.copy_from_slice(&buf[0..4]);
    if magic != FRAME_MAGIC {
        return Err(TornWriteError::InvalidMagic {
            expected: FRAME_MAGIC,
            found: magic,
        });
    }

    let len = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
    if len > MAX_PAYLOAD_LEN {
        return Err(TornWriteError::PayloadExceedsMaxBound {
            len,
            max: MAX_PAYLOAD_LEN,
        });
    }
    if len == 0 {
        return Err(TornWriteError::EmptyPayload);
    }

    let seq = u64::from_le_bytes([
        buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
    ]);
    if seq == 0 {
        return Err(TornWriteError::ZeroSequence);
    }

    let expected_crc = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);

    let total_len = HEADER_LEN + len;
    if buf.len() < total_len {
        return Err(TornWriteError::TruncatedPayload {
            needed: total_len,
            available: buf.len(),
        });
    }

    let payload = &buf[HEADER_LEN..total_len];
    let calculated_crc = crc32c::crc32c(payload);
    if calculated_crc != expected_crc {
        return Err(TornWriteError::CrcMismatch {
            expected: expected_crc,
            calculated: calculated_crc,
        });
    }

    Ok((
        TornWriteFrame {
            seq,
            crc32c: expected_crc,
            payload: payload.to_vec(),
        },
        total_len,
    ))
}

/// Recovers all consecutive intact frames from a raw log buffer.
/// Stops on the first torn write, truncated sector, or corruption,
/// computing the exact clean truncation offset.
pub fn recover_frames(buf: &[u8]) -> RecoveredLogReport {
    let mut offset = 0;
    let mut frames = Vec::new();
    let mut torn_detected = false;
    let mut reason = None;

    while offset < buf.len() {
        match decode_single_frame(&buf[offset..]) {
            Ok((frame, consumed)) => {
                frames.push(frame);
                offset += consumed;
            }
            Err(err) => {
                torn_detected = true;
                reason = Some(err);
                break;
            }
        }
    }

    RecoveredLogReport {
        frames,
        clean_offset: offset,
        torn_detected,
        reason,
    }
}

/// Recovers all consecutive intact frames with strict sequence monotonicity enforcement.
pub(crate) fn recover_frames_strict(buf: &[u8]) -> RecoveredLogReport {
    let mut offset = 0;
    let mut frames = Vec::new();
    let mut torn_detected = false;
    let mut reason = None;
    let mut last_seq = 0u64;

    while offset < buf.len() {
        match decode_single_frame(&buf[offset..]) {
            Ok((frame, consumed)) => {
                if frame.seq <= last_seq {
                    torn_detected = true;
                    reason = Some(TornWriteError::NonMonotonicSequence {
                        previous: last_seq,
                        current: frame.seq,
                    });
                    break;
                }
                last_seq = frame.seq;
                frames.push(frame);
                offset += consumed;
            }
            Err(err) => {
                torn_detected = true;
                reason = Some(err);
                break;
            }
        }
    }

    RecoveredLogReport {
        frames,
        clean_offset: offset,
        torn_detected,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let payload = b"hello durable world";
        let encoded = encode_frame(42, payload);
        let (frame, consumed) = decode_single_frame(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        assert_eq!(frame.seq, 42);
        assert_eq!(frame.payload, payload);
    }

    #[test]
    fn test_torn_write_recovery_at_any_truncation_point() {
        let mut log = Vec::new();
        let mut boundaries = vec![0];
        for i in 1..=10 {
            let p = format!("payload-{}", i);
            log.extend_from_slice(&encode_frame(i, p.as_bytes()));
            boundaries.push(log.len());
        }

        // Test truncation at every single byte from 0 to log.len()
        for cut in 0..=log.len() {
            let truncated = &log[..cut];
            let report = recover_frames(truncated);
            assert!(report.clean_offset <= cut);
            // Every recovered frame must have correct sequence
            for (idx, f) in report.frames.iter().enumerate() {
                assert_eq!(f.seq, (idx + 1) as u64);
            }
            if !boundaries.contains(&cut) {
                assert!(report.torn_detected);
            } else {
                assert!(!report.torn_detected);
            }
        }
    }

    #[test]
    fn test_bit_flip_detection() {
        let encoded = encode_frame(1, b"unaltered message");
        let mut corrupted = encoded.clone();
        // Flip one bit in payload
        corrupted[HEADER_LEN + 2] ^= 0x01;
        let report = recover_frames(&corrupted);
        assert!(report.torn_detected);
        assert_eq!(report.clean_offset, 0);
        assert!(matches!(report.reason, Some(TornWriteError::CrcMismatch { .. })));
    }

    #[test]
    fn test_torn_write_recovery_structural_invariants_red_to_green() {
        // 1. Error Display & std::error::Error conformance
        let errs: Vec<TornWriteError> = vec![
            TornWriteError::TruncatedHeader {
                needed: 20,
                available: 10,
            },
            TornWriteError::InvalidMagic {
                expected: FRAME_MAGIC,
                found: [0; 4],
            },
            TornWriteError::TruncatedPayload {
                needed: 50,
                available: 30,
            },
            TornWriteError::CrcMismatch {
                expected: 0x1234,
                calculated: 0x5678,
            },
            TornWriteError::EmptyPayload,
            TornWriteError::ZeroSequence,
            TornWriteError::PayloadExceedsMaxBound {
                len: 100,
                max: 50,
            },
            TornWriteError::NonMonotonicSequence {
                previous: 10,
                current: 5,
            },
        ];
        for err in &errs {
            let msg = format!("{err}");
            assert!(!msg.is_empty());
            let dyn_err: &dyn std::error::Error = err;
            assert_eq!(dyn_err.to_string(), msg);
        }

        // 2. TornWriteFrame::try_new validation
        assert_eq!(
            TornWriteFrame::try_new(0, b"data".to_vec()),
            Err(TornWriteError::ZeroSequence)
        );
        assert_eq!(
            TornWriteFrame::try_new(1, vec![]),
            Err(TornWriteError::EmptyPayload)
        );
        let valid_frame = TornWriteFrame::try_new(1, b"durable".to_vec()).unwrap();
        assert_eq!(valid_frame.seq, 1);
        assert_eq!(valid_frame.payload, b"durable");

        // 3. try_encode_frame validation
        assert_eq!(
            try_encode_frame(0, b"data"),
            Err(TornWriteError::ZeroSequence)
        );
        assert_eq!(
            try_encode_frame(1, b""),
            Err(TornWriteError::EmptyPayload)
        );
        let encoded = try_encode_frame(10, b"payload10").unwrap();

        // 4. decode_single_frame validation
        let (decoded, consumed) = decode_single_frame(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        assert_eq!(decoded.seq, 10);
        assert_eq!(decoded.payload, b"payload10");

        // 5. recover_frames_strict monotonic check
        let f1 = encode_frame(1, b"frame1");
        let f2 = encode_frame(2, b"frame2");
        let f_stale = encode_frame(1, b"frame_stale");

        let mut log = Vec::new();
        log.extend_from_slice(&f1);
        log.extend_from_slice(&f2);
        log.extend_from_slice(&f_stale);

        let report = recover_frames_strict(&log);
        assert!(report.torn_detected);
        assert_eq!(report.frame_count(), 2);
        assert_eq!(report.last_sequence(), Some(2));
        assert!(matches!(
            report.reason,
            Some(TornWriteError::NonMonotonicSequence { previous: 2, current: 1 })
        ));
    }
}
