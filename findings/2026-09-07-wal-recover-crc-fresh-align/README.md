# CRC at a fresh alignment is fail-stop, not a silent prefix

**Date:** 2026-09-07
**Primary source:** this kernel’s own F4/F14/G8 contract (module docs of
`wal/recover_kernel.rs`): a CRC mismatch right after a valid record
boundary is a real bad checksum (fail-stop). CRC seen *during* a resync
walk is garbage of the damaged region and must not brick the prefix.

## Used this turn

Catalog pair `wal_recover` (`data_fate`, entry `recover_collect_act`):
Verus on production `crates/pedradb-core/src/wal/recover_kernel.rs`.
rustc bodies last-wins. Plant
`recover_collect_act_on_live_crc_is_not_ok`.

## Not claimed

Dump of `WalReader::collect_all`. “somos seL4”.
