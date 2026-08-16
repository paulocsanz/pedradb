//! Sorted String Table (SST) — on-disk ordered versions.
//!
//! - **v1** — flat entry list (legacy readable).  
//! - **v2** — data blocks + sparse index.  
//! - **v3** — v2 + on-disk Bloom filter (RFC-0014).
//! - **v4** (writer default) — v3 + lz4-compressed data blocks.
//!
//! All versions may carry a trailing CRC32C (fail-stop on bitrot).

mod scan_kernel;
mod table;

pub use scan_kernel::{
    key_in_window, point_bounds_overlap, scan_reads_file, scan_reads_file_as_is,
    tombstone_reaches_window, tombstone_reaches_window_as_is,
};
pub use table::{
    reset_sst_blocks_decoded, sst_blocks_decoded, write_sst, write_sst_entries,
    write_sst_entries_on, write_sst_on, write_sst_sorted_on, write_sst_try_sorted_on,
    SstInternalStream, SstRangeIter, SstTable, SST_VERSION, SST_VERSION_V1, SST_VERSION_V2,
    SST_VERSION_V3,
};
