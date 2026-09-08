# L28 abort path is clean only when the removed node is closed

**Date:** 2026-09-07
**Primary source:** RFC-0134 / this kernel (`l28_tcp_abort_ok`): the
removed-node abort is L28-clean iff the close completed. AS-IS ignores
the close bit.

## Used this turn

Catalog pair `l28_tcp_abort` (`data_fate`, entry `l28_tcp_abort_ok`):
Verus on production `crates/pedradb-store/src/l28.rs`. rustc body
last-wins. Plant `l28_tcp_abort_ok_requires_closed`. Other `l28_tcp_*`
keep twins.

## Not claimed

Dump of `cluster_real`. Invented `l28_tcp_*_ok` gates. “somos seL4”.
