# RFC-0052 P2.1 — native `world_smoke` vs Linux TCG guest (2026-08-24)

Seed **42**. Oráculo = `trace_hash`. Wall-clock **não** comparado.

| Lado | ISA | hash |
|---|---|---|
| native (Darwin aarch64, `cargo run --release`) | aarch64 | `61f8a02125b3c69e` |
| guest (`qemu-system-x86_64 -accel tcg -smp 1 -icount shift=6,sleep=off`) | Linux 6.6.134-virt x86_64, musl static | `61f8a02125b3c69e` |

`C2.1=native_eq_guest`. Puts/gets/events/t also identical (`puts_ok=6 events=126 t=166`).

Guest = Alpine virt bzImage (downloaded, not in git) + initramfs with
`scripts/tcg-guest/init.c` + static `world_smoke`. Flags recusam MTTCG e
`shift=auto` (RFC-0005).

P2.1 is G3 logical replay (same seed, same World trace under TCG). tmpfs ≠
durable media.

P2.3 (`scripts/tcg_world_smoke_detio.sh`): same seed, `STALL_SO=/libdet_io.so`
PRELOAD inside the guest, `drop_fsync_all`. **215** `fdatasync drop=1` lines
on serial; hash still `61f8a02125b3c69e`. That is AND TCG×det_io — libc fsync
lied (returned 0 without sync) and the World trace did not change. Not a
proof of media ECC.

Mutant: `ICOUNT_SHIFT=auto` → exit 2.
