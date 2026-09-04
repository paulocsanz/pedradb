//! Sorted String Table (SST) — on-disk ordered versions.
//!
//! - **v1** — flat entry list (legacy readable).  
//! - **v2** — data blocks + sparse index.  
//! - **v3** — v2 + on-disk Bloom filter (RFC-0014).
//! - **v4** — v3 + lz4-compressed data blocks (legacy readable).
//! - **v5** (compressed writer default) — v4 + per-block CRC32C (RFC-0077 P1.1).
//!
//! All versions may carry a trailing file CRC32C (fail-stop on bitrot).

mod scan_kernel;
mod table;

pub use scan_kernel::{
    key_in_window, point_bounds_overlap, scan_reads_file, scan_reads_file_as_is, sst_block_crc_ok,
    sst_block_crc_ok_as_is, sst_crc_fate, sst_crc_fate_as_is, tombstone_reaches_window,
    tombstone_reaches_window_as_is, zero_glue_admitted, zero_glue_admitted_as_is, SstCrcFate,
    SST_LEGACY_NO_CRC_MAX,
};
pub use table::{
    force_get_stages, reset_get_stages, reset_sst_block_crc_skipped, reset_sst_blocks_decoded,
    sst_block_crc_skipped, sst_blocks_decoded, take_get_stages, write_l0_sst,
    write_l0_sst_for_family, write_sst, write_sst_bulk_arrays, write_sst_entries,
    write_sst_entries_on, write_sst_on, write_sst_on_with, write_sst_sorted_on,
    write_sst_try_sorted_on, write_sst_try_sorted_with, PointSeekScratch, SstInternalStream,
    SstRangeIter, SstTable, SST_VERSION, SST_VERSION_V1, SST_VERSION_V2, SST_VERSION_V3,
    SST_VERSION_V4, SST_VERSION_V6,
};
pub(crate) use table::{put_tls_point_seek_scratch, take_tls_point_seek_scratch};
