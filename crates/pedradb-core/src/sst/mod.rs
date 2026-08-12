//! Sorted String Table (SST) — on-disk ordered versions (P1.1).
//!
//! Simple v1 format (correctness first, not block/Bloom optimized):
//!
//! ```text
//! magic:        8 bytes  "PEDRSST\0"
//! version:      u32 LE   = 1
//! num_entries:  u64 LE
//! for each entry (InternalKey order: user_key asc, seq desc, kind desc):
//!   ikey_len:   u32 LE
//!   ikey:       [u8; ikey_len]   // InternalKey::encode
//!   val_len:    u32 LE
//!   value:      [u8; val_len]
//! max_sequence: u64 LE           // highest seq in file (for open bookkeeping)
//! ```

mod table;

pub use table::{
    write_sst, write_sst_entries, write_sst_entries_on, write_sst_on, SstTable,
};
