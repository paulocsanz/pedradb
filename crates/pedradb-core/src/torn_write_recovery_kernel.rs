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
        }
    }
}

impl std::error::Error for TornWriteError {}

pub const FRAME_MAGIC: [u8; 4] = *b"PDR1";
pub const HEADER_LEN: usize = 20; // 4 magic + 4 len + 8 seq + 4 crc

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TornWriteFrame {
    pub seq: u64,
    pub crc32c: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredLogReport {
    pub frames: Vec<TornWriteFrame>,
    pub clean_offset: usize,
    pub torn_detected: bool,
    pub reason: Option<TornWriteError>,
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
    let seq = u64::from_le_bytes([
        buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
    ]);
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
}
