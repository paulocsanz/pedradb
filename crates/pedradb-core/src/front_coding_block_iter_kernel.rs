//! RFC-0316: Differential Prefix Front-Coding Block Iterator Kernel
//!
//! Provides zero-panic, verified decoding and binary-search traversal of
//! SSTable data blocks compressed via front-coding / delta prefix sharing.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockDecodeError {
    BufferUnderflow,
    SharedPrefixOverflow { shared: usize, prev_len: usize },
    RestartOffsetOutOfBounds { offset: usize, data_len: usize },
    RestartNonZeroShared { offset: usize, shared: usize },
    KeyOrderViolation,
    InvalidRestartCount,
    InvalidRestartOffset { index: usize, offset: usize },
    NonMonotonicRestarts { prev: usize, current: usize },
    VarintOverflow,
}

impl fmt::Display for BlockDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferUnderflow => write!(f, "Buffer underflow during block decode"),
            Self::SharedPrefixOverflow { shared, prev_len } => write!(
                f,
                "Shared prefix {} exceeds previous key length {}",
                shared, prev_len
            ),
            Self::RestartOffsetOutOfBounds { offset, data_len } => write!(
                f,
                "Restart offset {} exceeds data length {}",
                offset, data_len
            ),
            Self::RestartNonZeroShared { offset, shared } => write!(
                f,
                "Restart point at offset {} has non-zero shared prefix {}",
                offset, shared
            ),
            Self::KeyOrderViolation => write!(f, "Key ordering violation within block"),
            Self::InvalidRestartCount => write!(f, "Invalid restart count in block trailer"),
            Self::InvalidRestartOffset { index, offset } => write!(
                f,
                "Invalid restart offset at index {}: {}",
                index, offset
            ),
            Self::NonMonotonicRestarts { prev, current } => write!(
                f,
                "Non-monotonic restart offsets: prev {} >= current {}",
                prev, current
            ),
            Self::VarintOverflow => write!(f, "Varint decoding overflow"),
        }
    }
}

impl std::error::Error for BlockDecodeError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockEntry {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

impl BlockEntry {
    pub fn new(key: Vec<u8>, value: Vec<u8>) -> Self {
        Self { key, value }
    }
}

/// Helper to encode/decode 32-bit little-endian integers
#[inline]
fn read_u32_le(slice: &[u8]) -> u32 {
    let mut arr = [0u8; 4];
    arr.copy_from_slice(&slice[..4]);
    u32::from_le_bytes(arr)
}

/// Helper for variable-byte encoding/decoding
fn encode_varint32(val: u32, buf: &mut Vec<u8>) {
    let mut v = val;
    while v >= 0x80 {
        buf.push(((v & 0x7F) | 0x80) as u8);
        v >>= 7;
    }
    buf.push(v as u8);
}

fn decode_varint32(slice: &[u8]) -> Option<(u32, usize)> {
    let mut res: u32 = 0;
    let mut shift: u32 = 0;
    for (i, &b) in slice.iter().enumerate() {
        if i >= 5 {
            return None;
        }
        if i == 4 {
            // 5th byte cannot have continuation bit or bits > 3 set (would overflow u32)
            if b & 0x80 != 0 || b & 0xF0 != 0 {
                return None;
            }
        }
        res |= ((b & 0x7F) as u32) << shift;
        if (b & 0x80) == 0 {
            return Some((res, i + 1));
        }
        shift += 7;
    }
    None
}

/// Builder that encodes a sequence of key-value pairs into a front-coded block.
#[derive(Clone, Debug)]
pub struct FrontCodingBlockEncoder {
    restart_interval: usize,
    entries: Vec<BlockEntry>,
}

impl FrontCodingBlockEncoder {
    pub fn new(restart_interval: usize) -> Self {
        Self {
            restart_interval: if restart_interval == 0 { 16 } else { restart_interval },
            entries: Vec::new(),
        }
    }

    pub fn add(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.entries.push(BlockEntry::new(key, value));
    }

    /// Emits the complete block payload: [Entries...] [RestartOffsets: u32...] [NumRestarts: u32].
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut restarts = Vec::new();
        let mut prev_key: Vec<u8> = Vec::new();

        for (i, entry) in self.entries.iter().enumerate() {
            let is_restart = i % self.restart_interval == 0;
            if is_restart {
                restarts.push(buf.len() as u32);
            }

            let shared = if is_restart {
                0
            } else {
                let mut match_len = 0;
                while match_len < prev_key.len()
                    && match_len < entry.key.len()
                    && prev_key[match_len] == entry.key[match_len]
                {
                    match_len += 1;
                }
                match_len
            };

            let unshared = entry.key.len() - shared;
            let val_len = entry.value.len();

            encode_varint32(shared as u32, &mut buf);
            encode_varint32(unshared as u32, &mut buf);
            encode_varint32(val_len as u32, &mut buf);

            buf.extend_from_slice(&entry.key[shared..]);
            buf.extend_from_slice(&entry.value);

            prev_key = entry.key.clone();
        }

        // Write restart array trailer
        for &offset in &restarts {
            buf.extend_from_slice(&offset.to_le_bytes());
        }
        buf.extend_from_slice(&(restarts.len() as u32).to_le_bytes());

        buf
    }
}

/// Zero-copy viewer and validator for front-coded data blocks.
#[derive(Clone, Debug)]
pub struct FrontCodingBlockViewer<'a> {
    data: &'a [u8],
    data_len: usize,
    restarts: Vec<u32>,
}

impl<'a> FrontCodingBlockViewer<'a> {
    /// Parses and verifies the structure of a raw front-coded block.
    pub fn parse(raw_block: &'a [u8]) -> Result<Self, BlockDecodeError> {
        if raw_block.len() < 4 {
            return Err(BlockDecodeError::BufferUnderflow);
        }

        let num_restarts_offset = raw_block.len() - 4;
        let num_restarts = read_u32_le(&raw_block[num_restarts_offset..]) as usize;

        if num_restarts == 0 {
            if num_restarts_offset > 0 {
                return Err(BlockDecodeError::InvalidRestartCount);
            }
            return Ok(Self {
                data: &raw_block[..0],
                data_len: 0,
                restarts: Vec::new(),
            });
        }

        let restart_array_bytes = num_restarts
            .checked_mul(4)
            .ok_or(BlockDecodeError::BufferUnderflow)?;
        if num_restarts_offset < restart_array_bytes {
            return Err(BlockDecodeError::BufferUnderflow);
        }

        let restart_array_start = num_restarts_offset - restart_array_bytes;
        let data_len = restart_array_start;

        let mut restarts = Vec::with_capacity(num_restarts);
        for i in 0..num_restarts {
            let offset = restart_array_start + i * 4;
            let restart_offset = read_u32_le(&raw_block[offset..offset + 4]);
            if restart_offset as usize > data_len {
                return Err(BlockDecodeError::RestartOffsetOutOfBounds {
                    offset: restart_offset as usize,
                    data_len,
                });
            }
            if i == 0 && restart_offset != 0 {
                return Err(BlockDecodeError::InvalidRestartOffset {
                    index: 0,
                    offset: restart_offset as usize,
                });
            }
            if i > 0 && restart_offset <= restarts[i - 1] {
                return Err(BlockDecodeError::NonMonotonicRestarts {
                    prev: restarts[i - 1] as usize,
                    current: restart_offset as usize,
                });
            }
            restarts.push(restart_offset);
        }

        let viewer = Self {
            data: &raw_block[..data_len],
            data_len,
            restarts,
        };

        // Validate restarts point to shared == 0 entries
        for &r_off in &viewer.restarts {
            let off = r_off as usize;
            if off < data_len {
                let (shared, _) = decode_varint32(&viewer.data[off..])
                    .ok_or(BlockDecodeError::BufferUnderflow)?;
                if shared != 0 {
                    return Err(BlockDecodeError::RestartNonZeroShared {
                        offset: off,
                        shared: shared as usize,
                    });
                }
            }
        }

        Ok(viewer)
    }

    #[inline]
    pub fn num_restarts(&self) -> usize {
        self.restarts.len()
    }

    /// Decodes all entries sequentially, verifying prefix bounds and strict ordering.
    pub fn decode_all_entries(&self) -> Result<Vec<BlockEntry>, BlockDecodeError> {
        let mut entries = Vec::new();
        let mut offset = 0;
        let mut prev_key: Vec<u8> = Vec::new();
        let mut has_prev = false;

        while offset < self.data_len {
            let (shared, len1) = decode_varint32(&self.data[offset..])
                .ok_or(BlockDecodeError::BufferUnderflow)?;
            offset += len1;

            let (unshared, len2) = decode_varint32(&self.data[offset..])
                .ok_or(BlockDecodeError::BufferUnderflow)?;
            offset += len2;

            let (val_len, len3) = decode_varint32(&self.data[offset..])
                .ok_or(BlockDecodeError::BufferUnderflow)?;
            offset += len3;

            let shared_sz = shared as usize;
            let unshared_sz = unshared as usize;
            let val_sz = val_len as usize;

            if shared_sz > prev_key.len() {
                return Err(BlockDecodeError::SharedPrefixOverflow {
                    shared: shared_sz,
                    prev_len: prev_key.len(),
                });
            }

            if offset + unshared_sz + val_sz > self.data_len {
                return Err(BlockDecodeError::BufferUnderflow);
            }

            let mut key = Vec::with_capacity(shared_sz + unshared_sz);
            key.extend_from_slice(&prev_key[..shared_sz]);
            key.extend_from_slice(&self.data[offset..offset + unshared_sz]);
            offset += unshared_sz;

            let value = self.data[offset..offset + val_sz].to_vec();
            offset += val_sz;

            if has_prev && key <= prev_key {
                return Err(BlockDecodeError::KeyOrderViolation);
            }

            prev_key = key.clone();
            has_prev = true;
            entries.push(BlockEntry::new(key, value));
        }

        Ok(entries)
    }

    /// Reads only the full key at a restart point (since shared == 0).
    fn read_restart_key(&self, restart_idx: usize) -> Result<Vec<u8>, BlockDecodeError> {
        let off = self.restarts[restart_idx] as usize;
        let (shared, len1) =
            decode_varint32(&self.data[off..]).ok_or(BlockDecodeError::BufferUnderflow)?;
        if shared != 0 {
            return Err(BlockDecodeError::RestartNonZeroShared {
                offset: off,
                shared: shared as usize,
            });
        }
        let off = off + len1;
        let (unshared, len2) =
            decode_varint32(&self.data[off..]).ok_or(BlockDecodeError::BufferUnderflow)?;
        let off = off + len2;
        let (_val_len, len3) =
            decode_varint32(&self.data[off..]).ok_or(BlockDecodeError::BufferUnderflow)?;
        let off = off + len3;
        let unshared_sz = unshared as usize;
        if off + unshared_sz > self.data_len {
            return Err(BlockDecodeError::BufferUnderflow);
        }
        Ok(self.data[off..off + unshared_sz].to_vec())
    }

    /// Fast binary-search and seek for an exact key within the block.
    pub fn seek_key(&self, target_key: &[u8]) -> Result<Option<BlockEntry>, BlockDecodeError> {
        if self.restarts.is_empty() || self.data_len == 0 {
            return Ok(None);
        }

        // Binary search restart array: find highest restart point with key <= target_key
        let mut left = 0;
        let mut right = self.restarts.len() - 1;
        let mut target_restart = 0;

        while left <= right {
            let mid = left + (right - left) / 2;
            let mid_key = self.read_restart_key(mid)?;
            if mid_key.as_slice() <= target_key {
                target_restart = mid;
                left = mid + 1;
            } else {
                if mid == 0 {
                    break;
                }
                right = mid - 1;
            }
        }

        // Scan from target_restart forward up to next restart or end of block
        let start_off = self.restarts[target_restart] as usize;
        let end_off = if target_restart + 1 < self.restarts.len() {
            self.restarts[target_restart + 1] as usize
        } else {
            self.data_len
        };

        let mut offset = start_off;
        let mut prev_key: Vec<u8> = Vec::new();

        while offset < end_off {
            let (shared, len1) = decode_varint32(&self.data[offset..])
                .ok_or(BlockDecodeError::BufferUnderflow)?;
            offset += len1;

            let (unshared, len2) = decode_varint32(&self.data[offset..])
                .ok_or(BlockDecodeError::BufferUnderflow)?;
            offset += len2;

            let (val_len, len3) = decode_varint32(&self.data[offset..])
                .ok_or(BlockDecodeError::BufferUnderflow)?;
            offset += len3;

            let shared_sz = shared as usize;
            let unshared_sz = unshared as usize;
            let val_sz = val_len as usize;

            if shared_sz > prev_key.len() {
                return Err(BlockDecodeError::SharedPrefixOverflow {
                    shared: shared_sz,
                    prev_len: prev_key.len(),
                });
            }

            if offset + unshared_sz + val_sz > self.data_len {
                return Err(BlockDecodeError::BufferUnderflow);
            }

            let mut key = Vec::with_capacity(shared_sz + unshared_sz);
            key.extend_from_slice(&prev_key[..shared_sz]);
            key.extend_from_slice(&self.data[offset..offset + unshared_sz]);
            offset += unshared_sz;

            let value = self.data[offset..offset + val_sz].to_vec();
            offset += val_sz;

            if key.as_slice() == target_key {
                return Ok(Some(BlockEntry::new(key, value)));
            } else if key.as_slice() > target_key {
                return Ok(None);
            }

            prev_key = key;
        }

        Ok(None)
    }

    /// Formally verifies all internal consistency invariants on the block.
    pub fn verify_internal_invariants(&self) -> Result<bool, BlockDecodeError> {
        let entries = self.decode_all_entries()?;
        for i in 1..entries.len() {
            if entries[i - 1].key >= entries[i].key {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
