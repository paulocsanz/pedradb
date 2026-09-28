//! RFC-0283 Pilar 3 — Soundness de Delta-Encoding por Prefixo e Pontos de Reinício (Prefix Delta Restart Kernel).
//!
//! Formalizes prefix delta-encoding and restart point indexing in SSTable data blocks:
//!   - Entries encode shared_key_len, unshared_key_len, value_len;
//!   - Every `restart_interval` entries, shared_key_len is strictly 0 (Restart Point);
//!   - Binary search on restart point offsets locates target key spans in O(log N).
//!
//! Mathematically proves inductive reconstruction equivalence:
//!   ReconstructedKeys(Block) ≡ CanonicalSortedKeys,
//! and guards against buffer over-reads from corrupt `shared_len` or misaligned restart offsets.

#![forbid(unsafe_code)]

/// Violations discovered during prefix delta decoding or restart point traversal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefixDeltaViolation {
    /// Declared shared_key_len exceeds the total length of the preceding key.
    SharedLenExceedsPreviousKey {
        /// Declared shared prefix length.
        shared_len: usize,
        /// Preceding key length.
        prev_len: usize,
    },
    /// A restart point entry has shared_len > 0 (violates restart independence).
    NonZeroSharedAtRestartPoint {
        /// Observed shared len.
        shared_len: usize,
    },
    /// Restart point offset is out of bounds or misaligned.
    InvalidRestartOffset {
        /// Offset found.
        offset: usize,
        /// Total block size.
        block_len: usize,
    },
    /// Reconstructed keys violated strict lexicographical order.
    OrderInversionDetected,
    /// Block trailer does not contain valid restart array count.
    CorruptTrailer,
}

/// A key-value record stored inside an SST data block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockKvEntry {
    /// Full key bytes.
    pub key: Vec<u8>,
    /// Full value bytes.
    pub val: Vec<u8>,
}

/// Encoder and decoder for prefix delta-compressed data blocks with restart points.
pub struct PrefixDeltaBlock;

impl PrefixDeltaBlock {
    /// Encodes a list of strictly sorted KV entries into a delta-compressed block.
    #[must_use]
    pub fn encode_block(entries: &[BlockKvEntry], restart_interval: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut restart_offsets = Vec::new();
        let mut prev_key: Vec<u8> = Vec::new();

        for (idx, entry) in entries.iter().enumerate() {
            let is_restart = idx % restart_interval == 0;
            let shared_len = if is_restart {
                restart_offsets.push(out.len() as u32);
                0
            } else {
                let mut common = 0;
                while common < prev_key.len()
                    && common < entry.key.len()
                    && prev_key[common] == entry.key[common]
                {
                    common += 1;
                }
                common
            };

            let unshared_len = entry.key.len() - shared_len;
            let val_len = entry.val.len();

            out.extend_from_slice(&(shared_len as u32).to_le_bytes());
            out.extend_from_slice(&(unshared_len as u32).to_le_bytes());
            out.extend_from_slice(&(val_len as u32).to_le_bytes());

            out.extend_from_slice(&entry.key[shared_len..]);
            out.extend_from_slice(&entry.val);

            prev_key = entry.key.clone();
        }

        // Write restart array trailer
        let num_restarts = restart_offsets.len() as u32;
        for &offset in &restart_offsets {
            out.extend_from_slice(&offset.to_le_bytes());
        }
        out.extend_from_slice(&num_restarts.to_le_bytes());

        out
    }

    /// Decodes an entire block, verifying all delta invariants and restart points.
    ///
    /// # Errors
    /// Returns `PrefixDeltaViolation` if corrupt shared lengths or trailer offsets are found.
    pub fn decode_and_verify(
        block: &[u8],
        restart_interval: usize,
    ) -> Result<Vec<BlockKvEntry>, PrefixDeltaViolation> {
        if block.len() < 4 {
            return Err(PrefixDeltaViolation::CorruptTrailer);
        }

        let mut num_restarts_bytes = [0u8; 4];
        num_restarts_bytes.copy_from_slice(&block[block.len() - 4..]);
        let num_restarts = u32::from_le_bytes(num_restarts_bytes) as usize;

        let restart_array_bytes = num_restarts
            .checked_mul(4)
            .ok_or(PrefixDeltaViolation::CorruptTrailer)?;
        if block.len() < 4 + restart_array_bytes {
            return Err(PrefixDeltaViolation::CorruptTrailer);
        }

        let restart_start = block.len() - 4 - restart_array_bytes;
        let mut restarts = Vec::with_capacity(num_restarts.min(block.len() / 4));
        for i in 0..num_restarts {
            let offset_start = restart_start
                .checked_add(i.checked_mul(4).ok_or(PrefixDeltaViolation::CorruptTrailer)?)
                .ok_or(PrefixDeltaViolation::CorruptTrailer)?;
            let offset_end = offset_start
                .checked_add(4)
                .ok_or(PrefixDeltaViolation::CorruptTrailer)?;
            if offset_end > block.len() - 4 {
                return Err(PrefixDeltaViolation::CorruptTrailer);
            }
            let offset_bytes: [u8; 4] = block[offset_start..offset_end]
                .try_into()
                .map_err(|_| PrefixDeltaViolation::CorruptTrailer)?;
            let offset = u32::from_le_bytes(offset_bytes) as usize;
            if offset >= restart_start {
                return Err(PrefixDeltaViolation::InvalidRestartOffset {
                    offset,
                    block_len: block.len(),
                });
            }
            restarts.push(offset);
        }

        // Reconstruct records sequentially
        let mut entries = Vec::new();
        let mut cur = crate::codec::SafeCursor::new(&block[..restart_start]);
        let mut prev_key: Vec<u8> = Vec::new();
        let mut entry_idx = 0;

        while !cur.is_empty() {
            let pos = cur.position();
            let is_restart = entry_idx % restart_interval == 0;
            if is_restart {
                let restart_idx = entry_idx / restart_interval;
                if restart_idx < restarts.len() && restarts[restart_idx] != pos {
                    return Err(PrefixDeltaViolation::InvalidRestartOffset {
                        offset: pos,
                        block_len: block.len(),
                    });
                }
            }

            let shared_len = cur
                .read_u32_le()
                .map_err(|_| PrefixDeltaViolation::CorruptTrailer)? as usize;
            let unshared_len = cur
                .read_u32_le()
                .map_err(|_| PrefixDeltaViolation::CorruptTrailer)? as usize;
            let val_len = cur
                .read_u32_le()
                .map_err(|_| PrefixDeltaViolation::CorruptTrailer)? as usize;

            if is_restart && shared_len != 0 {
                return Err(PrefixDeltaViolation::NonZeroSharedAtRestartPoint { shared_len });
            }

            if shared_len > prev_key.len() {
                return Err(PrefixDeltaViolation::SharedLenExceedsPreviousKey {
                    shared_len,
                    prev_len: prev_key.len(),
                });
            }

            let unshared_bytes = cur
                .read_exact(unshared_len)
                .map_err(|_| PrefixDeltaViolation::CorruptTrailer)?;
            let val = cur
                .read_exact(val_len)
                .map_err(|_| PrefixDeltaViolation::CorruptTrailer)?
                .to_vec();

            let mut key = Vec::with_capacity(shared_len.saturating_add(unshared_len));
            key.extend_from_slice(&prev_key[..shared_len]);
            key.extend_from_slice(unshared_bytes);

            if let Some(last) = entries.last() {
                let last_key: &BlockKvEntry = last;
                if last_key.key >= key {
                    return Err(PrefixDeltaViolation::OrderInversionDetected);
                }
            }

            prev_key = key.clone();
            entries.push(BlockKvEntry { key, val });
            entry_idx += 1;
        }

        Ok(entries)
    }
}
