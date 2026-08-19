# RFC-0041 — catch-up default 50 µs (fat-batch full window)

2026-08-18. **No remesure.** Load ~59–109 on 12 CPUs.

`CATCHUP_WINDOW_DEFAULT` is **50 µs** again (quiet head3). 1-op puts still
wait `min(window, fd_ema/2)`. Raftlog (≥16 ops) waits the **full** 50 µs
so MC siblings can share one `fdatasync`. `PEDRA_CATCHUP_US=0` still off.

TLS get/count skip `check_cf` on a hit (epoch still gates staleness).

Tests: `catchup_window_knob_roundtrip` asserts 50 µs; `catchup_bound_policy`
(1-op fd/2 vs 16-op full window); TLS get/count invalidate-on-put.

Official map unchanged: `findings/rfc0041-p11/head3/`.
