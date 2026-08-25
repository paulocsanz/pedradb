#!/usr/bin/env bash
# RFC-0050 P2.3 — block-layer EIO (QEMU blkdebug), not FailingEnv / not libc.
#
# Boot 1: format a raw IDE disk (vfat) with no injector.
# Boot 2: same image under blkdebug flush_to_disk → EIO once; world_smoke
# writes TMPDIR on that disk. Oracle: silent_wrong=0 and the kernel
# reports I/O error (or puts_err>0). Wall-clock is forbidden.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
CACHE="${PEDRA_TCG_CACHE:-$HOME/.cache/pedradb-tcg}"
WORK="${PEDRA_TCG_WORK:-$ROOT/target/tcg-blk-eio}"
KERN_URL="${PEDRA_TCG_KERNEL_URL:-https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/x86_64/netboot/vmlinuz-virt}"
ICOUNT_SHIFT="${ICOUNT_SHIFT:-6}"
mkdir -p "$CACHE" "$WORK"

residual() {
  echo "C2.4=$1"
  echo "note=$2"
  if [[ "${TCG_REQUIRED:-}" == 1 ]]; then
    echo "tcg_blk_eio: required ($1)" >&2
    exit 1
  fi
  echo "tcg_blk_eio OK residual"
  exit 0
}

case "${ICOUNT_SHIFT}" in
  auto|AUTO) echo "tcg_blk_eio: REFUSE icount shift=auto" >&2; exit 2 ;;
esac

need() { command -v "$1" >/dev/null || residual "residual_missing_$1" "install $1"; }
need qemu-system-x86_64
need gzip
need cpio
need cargo
need python3
need curl

MUSL_CC="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER:-}"
if [[ -z "$MUSL_CC" ]]; then
  if command -v x86_64-linux-musl-gcc >/dev/null; then MUSL_CC=x86_64-linux-musl-gcc
  elif command -v musl-gcc >/dev/null; then MUSL_CC=musl-gcc
  else residual "residual_missing_musl_cc" "need musl gcc"; fi
fi

KERN="$CACHE/vmlinuz-virt"
if [[ ! -s "$KERN" ]]; then
  curl -fsSL -o "$KERN.part" "$KERN_URL"
  mv "$KERN.part" "$KERN"
fi

BB="$CACHE/busybox.static"
if [[ ! -s "$BB" ]]; then
  curl -fsSL -o "$CACHE/busybox-static.apk" \
    "https://dl-cdn.alpinelinux.org/alpine/v3.20/main/x86_64/busybox-static-1.36.1-r31.apk"
  mkdir -p "$CACHE/bbx"
  tar -xzf "$CACHE/busybox-static.apk" -C "$CACHE/bbx"
  cp "$CACHE/bbx/bin/busybox.static" "$BB"
fi

echo "== static musl world_smoke =="
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$MUSL_CC"
# Default musl target is crt-static; do not inherit a -crt-static from P2.3.
unset RUSTFLAGS
cargo build --release -p pedradb-world --bin world_smoke --target x86_64-unknown-linux-musl
SMOKE=target/x86_64-unknown-linux-musl/release/world_smoke
cp "$SMOKE" "$WORK/world_smoke"
if command -v x86_64-linux-musl-strip >/dev/null; then
  x86_64-linux-musl-strip "$WORK/world_smoke" || true
fi

"$MUSL_CC" -static -Os -s "$ROOT/scripts/tcg-guest/init_blk.c" -o "$WORK/init"

STAGE="$WORK/root"
rm -rf "$STAGE"
mkdir -p "$STAGE"/{proc,sys,tmp,dev,data,mod}
cp "$WORK/init" "$STAGE/init"
cp "$WORK/world_smoke" "$STAGE/world_smoke"
cp "$BB" "$STAGE/busybox"
MOD="$CACHE/virt-initrd/lib/modules/6.6.134-0-virt/kernel"
if [[ ! -d "$MOD" ]]; then
  mkdir -p "$CACHE/virt-initrd"
  [[ -s "$CACHE/initramfs-virt" ]] || curl -fsSL -o "$CACHE/initramfs-virt" \
    https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/x86_64/netboot/initramfs-virt
  gzip -dc "$CACHE/initramfs-virt" | (cd "$CACHE/virt-initrd" && cpio -id >/dev/null 2>&1)
fi
cp "$MOD/drivers/block/virtio_blk.ko" "$STAGE/mod/"
cp "$MOD/fs/fat/fat.ko" "$STAGE/mod/"
cp "$MOD/fs/fat/vfat.ko" "$STAGE/mod/"
cp "$MOD/fs/nls/nls_cp437.ko" "$STAGE/mod/"
cp "$MOD/fs/nls/nls_iso8859-1.ko" "$STAGE/mod/"
cp "$MOD/fs/nls/nls_utf8.ko" "$STAGE/mod/"
chmod 755 "$STAGE/init" "$STAGE/world_smoke" "$STAGE/busybox"
( cd "$STAGE" && find . | cpio -o -H newc 2>/dev/null ) | gzip -9 > "$WORK/initramfs.cpio.gz"

DISK="$WORK/disk.img"
dd if=/dev/zero of="$DISK" bs=1048576 count=32 status=none

boot() {
  local mode=$1 inj=$2 serial=$3
  local drive
  if [[ "$inj" == "1" ]]; then
    drive="file=blkdebug:${ROOT}/scripts/tcg-guest/blkdebug.ini:${DISK},if=none,id=hd0,format=raw"
  else
    drive="file=${DISK},if=none,id=hd0,format=raw"
  fi
  python3 - "$KERN" "$WORK/initramfs.cpio.gz" "$drive" "$mode" "$serial" "$ICOUNT_SHIFT" <<'PY'
import subprocess, sys, time
kern, initrd, drive, mode, serial, shift = sys.argv[1:7]
cmd = [
    "qemu-system-x86_64",
    "-accel", "tcg",
    "-cpu", "qemu64",
    "-machine", "pc",
    "-smp", "1",
    "-m", "512",
    "-display", "none",
    "-serial", "stdio",
    "-no-reboot",
    "-icount", f"shift={shift},sleep=off",
    "-rtc", "clock=vm,base=2000-01-01T00:00:00",
    "-kernel", kern,
    "-initrd", initrd,
    "-drive", drive,
    "-device", "virtio-blk-pci,drive=hd0",
    "-append", f"console=ttyS0,115200n8 panic=1 pedra_mode={mode}",
]
print("qemu:", " ".join(cmd), flush=True)
p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
t0 = time.time()
out = []
end = "FORMAT_OK" if mode == "format" else "BLK_RUN_END"
try:
    while True:
        if time.time() - t0 > 180:
            p.kill()
            print("WATCHDOG 180s", flush=True)
            break
        line = p.stdout.readline()
        if line == "" and p.poll() is not None:
            break
        if line:
            sys.stdout.write(line)
            sys.stdout.flush()
            out.append(line)
            if end in line or "MKFS_FAIL" in line or "MOUNT_FAIL" in line or "NO_DISK" in line:
                try:
                    p.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    p.kill()
                break
finally:
    if p.poll() is None:
        p.kill()
    open(serial, "w").write("".join(out))
text = "".join(out)
if mode == "format":
    sys.exit(0 if "FORMAT_OK" in text else 1)
sys.exit(0 if "BLK_RUN_END" in text or "seed=" in text else 1)
PY
}

echo "== format disk (no injector) =="
boot format 0 "$WORK/serial-format.log"

echo "== run world on blkdebug EIO =="
boot run 1 "$WORK/serial-eio.log"

SER="$WORK/serial-eio.log"
echo "compare_wall_clock=forbidden"
if grep -q 'NO_DISK\|MKFS_FAIL\|MOUNT_FAIL' "$WORK/serial-format.log" "$SER"; then
  echo "C2.4=FAIL_setup"
  exit 1
fi
SW="$(sed -n 's/.*silent_wrong=\([0-9]*\).*/\1/p' "$SER" | tail -1 || true)"
if [[ -n "$SW" && "$SW" != 0 ]]; then
  echo "C2.4=FAIL_silent_wrong silent_wrong=$SW"
  exit 1
fi
# Vacuity: the injector must have fired at the block layer.
if ! grep -qiE 'I/O error|blkdebug|critical target error|end_request.*I/O error|errno=5' "$SER"; then
  # Fallback: World saw the EIO as a put/get error.
  PERR="$(sed -n 's/.*puts_err=\([0-9]*\).*/\1/p' "$SER" | tail -1 || true)"
  if [[ -z "$PERR" || "$PERR" == 0 ]]; then
    echo "C2.4=FAIL_no_eio (injector did not fire)"
    exit 1
  fi
  echo "block_eio=puts_err=$PERR (no dmesg match; World saw I/O err)"
else
  echo "block_eio=kernel_log"
fi
echo "silent_wrong=${SW:-absent_run_aborted}"
echo "C2.4=block_eio_fail_closed"
echo "tcg_blk_eio OK"
exit 0
