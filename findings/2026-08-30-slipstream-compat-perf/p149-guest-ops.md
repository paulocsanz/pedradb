# p149 guest ops: injection procedure, known races, and repairs

Guest `linux-gate-p149b` on gate 192.168.68.109 (user paulo, BatchMode
ssh). Container prefix `cnt_0293db0846dc46e1b1d7d1f080e61473`.
Two disks: `-rootfs.qcow2` (system + /src sources, ~2.9 GB) and
`-data.qcow2` (persistent `/data` incl. the cargo target dir; 40 GB
virtual with tens of GB of stale allocated clusters — deleted files
never shrink a qcow2). Serial:
`/var/lib/caixote/runtime/cnt_...serial.log`. Bench legs run
automatically from the entrypoint; end marker `=== v17 trim done ===`.

## Standard injection (rootfs)

`/tmp/inject-v32-gate.sh` is the current template: caixote API stop →
`fuser SOCK` wait → `qemu-img convert` rootfs→raw → loop mount →
verify BASE md5s **computed from staged /tmp copies at runtime**
(never hand-transcribe a hash) → copy+md5-verify → entrypoint grep →
convert back → `mv` → guard `RACE_SUPERVISOR_RESTARTED_VM` → start.

Staged source copies live in /tmp on the gate (`*.rs.vNN`,
`wal-mod.rs.vNN`); they are the injected truth — the NEXT injection's
BASE_* must come from them.

## Known races and the 2026-09-01 incident

1. **Supervisor auto-restart during teardown**: between `mv` and the
   `fuser` guard, the caixote supervisor may resurrect the VM. Exit 2
   (`RACE_SUPERVISOR_RESTARTED_VM`) means the run may be on the OLD
   inode — check the new serial for `Compiling pedradb-core` before
   trusting it. Occurred during v29ac (wasted run) and v32 (turned out
   to have restarted on the OLD inode → free repeat run #32b).

2. **Never re-issue `container start` after a 500 "VM already exists
   with a running cloud-hypervisor process" without checking for a
   spawned process.** On 2026-09-01 09:47 a failed start HAD spawned a
   hypervisor; a second start attempt raced it, and TWO cloud-hypervisors
   opened the same data qcow2 read-write. Result: torn writes across the
   cargo target dir (bench ELF `Exec format error`, then
   `E0786 invalid metadata` for the whole rlib chain built at that
   instant — pedradb_core/io_uring/sim/ops/rocksdb_compat/slipstream).
   Cost: ~1 h of diagnosis/repair.

3. **Process discovery traps**: `pgrep cloud-hypervisor` silently
   matches nothing (name > 15 chars — use `ps aux | grep
   cloud-hypervisor | grep 0293db08 | grep -v grep`); `pkill -f
   <container-id>` self-matches the ssh wrapper and kills your own
   script. Kill by explicit PID from `ps`.

4. **`bash script | tee log` returns tee's exit code** — the v32 repair
   script "succeeded" (exit 0) while its qemu-img convert had failed.
   Use `bash script 2>&1 | tee log; echo EXIT=${PIPESTATUS[0]}` or run
   without the pipe.

5. **Disk**: the gate root fs is 239 GB and was at ~205 GB used before
   the incident. `qemu-img convert` of the DATA qcow2 to raw allocates
   42 GB and filled the disk to 100% in ~10 min (other VMs at risk).
   Never full-convert the data disk. Rootfs converts (~3 GB) are fine.

## Repair: in-place data-disk surgery via qemu-nbd (3 s, no copy)

`sudo modprobe nbd` (already loaded) then:

```
sudo qemu-nbd --connect=/dev/nbd0 <data.qcow2>
sudo mount /dev/nbd0 /mnt/p149-data      # whole-disk fs, no partition
# ...fix /mnt/p149-data/target...
sudo umount /mnt/p149-data && sudo qemu-nbd -d /dev/nbd0
```

For torn-write corruption from a known time window, delete by mtime
(files only) plus the affected crates' artifacts and fingerprints:

```
find /mnt/target -type f -newermt '<start>' ! -newermt '<end>' -delete
find /mnt/target -type f \( -name '*slipstream*' -o -name 'snapshot_backends*' \) -delete
```

Deleting artifacts but keeping fingerprints makes cargo think it is
fresh ("Finished in 0.62s") while the binary is a torn ELF — the
fingerprint dirs for deleted artifacts must go too (the mtime-window
find covers them when the fingerprint was written in the window).

`librocksdb-sys` (the expensive C++ build) predates any incident
window; keep it. A from-scratch `rm -rf target` would cost a full
rocksdb rebuild — avoid.

## After any guest reboot of the same image

Verify which image booted: `Compiling pedradb-core` must appear in the
serial if any core source changed (de-ANSI first:
`sed 's/\x1b\[[0-9;]*m//g'`). A 2-second leg (`BENCH_EXIT=101`,
`Exec format error`) means the target cache is torn — see repair above.
