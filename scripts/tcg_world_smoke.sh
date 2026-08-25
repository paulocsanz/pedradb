#!/usr/bin/env bash
# RFC-0052 P2.1 — native world_smoke vs Linux TCG guest, same seed.
#
# Builds a static musl `world_smoke`, packs it into an initramfs with
# `scripts/tcg-guest/init.c`, boots Alpine linux-virt under
# `qemu-system-x86_64 -accel tcg -smp 1 -icount shift=N,sleep=off`
# (RFC-0005 flags; MTTCG and shift=auto refused), and compares
# `trace_hash`. Wall-clock is never compared.
#
# TCG_REQUIRED=1: missing qemu/musl/kernel is a failure.
# Default: print C2.1=residual_* and exit 0 (honest, not TCG_PASS).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
SEED="${PEDRA_TCG_SEED:-42}"
CACHE="${PEDRA_TCG_CACHE:-$HOME/.cache/pedradb-tcg}"
WORK="${PEDRA_TCG_WORK:-$ROOT/target/tcg-world-smoke}"
KERN_URL="${PEDRA_TCG_KERNEL_URL:-https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/x86_64/netboot/vmlinuz-virt}"
ICOUNT_SHIFT="${ICOUNT_SHIFT:-6}"
mkdir -p "$CACHE" "$WORK"

residual() {
  echo "C2.1=$1"
  echo "note=$2"
  echo "compare_wall_clock=forbidden"
  if [[ "${TCG_REQUIRED:-}" == 1 ]]; then
    echo "tcg_world_smoke: required ($1)" >&2
    exit 1
  fi
  echo "tcg_world_smoke OK residual"
  exit 0
}

case "${ICOUNT_SHIFT}" in
  auto|AUTO) echo "tcg_world_smoke: REFUSE icount shift=auto (RFC-0005)" >&2; exit 2 ;;
esac

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    residual "residual_missing_$1" "install $1 (or set TCG_REQUIRED=1 in CI)"
  fi
}

need qemu-system-x86_64
need gzip
need cpio
need cargo

MUSL_CC="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER:-}"
if [[ -z "$MUSL_CC" ]]; then
  if command -v x86_64-linux-musl-gcc >/dev/null; then
    MUSL_CC=x86_64-linux-musl-gcc
  elif command -v musl-gcc >/dev/null; then
    MUSL_CC=musl-gcc
  else
    residual "residual_missing_musl_cc" "need x86_64-linux-musl-gcc or musl-gcc"
  fi
fi

KERN="$CACHE/vmlinuz-virt"
if [[ ! -s "$KERN" ]]; then
  echo "fetch kernel $KERN_URL"
  curl -fsSL -o "$KERN.part" "$KERN_URL"
  mv "$KERN.part" "$KERN"
fi

echo "== native world_smoke seed=$SEED =="
NATIVE_OUT="$WORK/native.txt"
cargo run -q --release -p pedradb-world --bin world_smoke -- "$SEED" | tee "$NATIVE_OUT"
NATIVE_HASH="$(sed -n 's/.*hash=\([0-9a-fA-F]*\).*/\1/p' "$NATIVE_OUT" | head -1)"
if [[ -z "$NATIVE_HASH" ]]; then
  echo "tcg_world_smoke: native run printed no hash" >&2
  exit 1
fi
echo "native_hash=$NATIVE_HASH"

echo "== musl world_smoke ($MUSL_CC) =="
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$MUSL_CC"
cargo build --release -p pedradb-world --bin world_smoke --target x86_64-unknown-linux-musl
SMOKE=target/x86_64-unknown-linux-musl/release/world_smoke
cp "$SMOKE" "$WORK/world_smoke"
if command -v x86_64-linux-musl-strip >/dev/null; then
  x86_64-linux-musl-strip "$WORK/world_smoke" || true
elif command -v strip >/dev/null; then
  strip "$WORK/world_smoke" || true
fi

echo "== guest init =="
"$MUSL_CC" -static -Os -s "$ROOT/scripts/tcg-guest/init.c" -o "$WORK/init"

echo "== initramfs =="
STAGE="$WORK/root"
rm -rf "$STAGE"
mkdir -p "$STAGE"/{proc,sys,tmp,dev}
cp "$WORK/init" "$STAGE/init"
cp "$WORK/world_smoke" "$STAGE/world_smoke"
chmod 755 "$STAGE/init" "$STAGE/world_smoke"
( cd "$STAGE" && find . | cpio -o -H newc 2>/dev/null ) | gzip -9 > "$WORK/initramfs.cpio.gz"

echo "== TCG boot (icount shift=$ICOUNT_SHIFT, no wall-clock compare) =="
SERIAL="$WORK/serial.log"
set +e
python3 - "$KERN" "$WORK/initramfs.cpio.gz" "$SERIAL" "$ICOUNT_SHIFT" <<'PY'
import subprocess, sys, time, os
kern, initrd, serial, shift = sys.argv[1:5]
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
    "-append", "console=ttyS0,115200n8 panic=1",
]
print("qemu:", " ".join(cmd), flush=True)
p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
t0 = time.time()
out = []
rc = 1
try:
    while True:
        if time.time() - t0 > 180:
            p.kill()
            print("WATCHDOG 180s", flush=True)
            break
        line = p.stdout.readline()
        if line == "" and p.poll() is not None:
            rc = p.returncode or 0
            break
        if line:
            sys.stdout.write(line)
            sys.stdout.flush()
            out.append(line)
            if "TCG_WORLD_SMOKE_END" in line:
                try:
                    p.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    p.kill()
                rc = 0
                break
finally:
    if p.poll() is None:
        p.kill()
    open(serial, "w").write("".join(out))
sys.exit(0 if "TCG_WORLD_SMOKE_END" in "".join(out) else rc or 1)
PY
QEC=$?
set -e
if [[ "$QEC" -ne 0 ]]; then
  echo "tcg_world_smoke: qemu failed exit=$QEC" >&2
  exit 1
fi

GUEST_LINE="$(grep -E '^seed=' "$SERIAL" | tail -1 || true)"
GUEST_HASH="$(printf '%s\n' "$GUEST_LINE" | sed -n 's/.*hash=\([0-9a-fA-F]*\).*/\1/p')"
echo "guest_line=$GUEST_LINE"
echo "guest_hash=$GUEST_HASH"
echo "native_hash=$NATIVE_HASH"
echo "compare_wall_clock=forbidden"

if [[ -z "$GUEST_HASH" ]]; then
  echo "C2.1=FAIL_no_guest_hash"
  exit 1
fi
if [[ "$GUEST_HASH" != "$NATIVE_HASH" ]]; then
  echo "C2.1=FAIL_hash_mismatch"
  exit 1
fi
echo "C2.1=native_eq_guest"
echo "tcg_world_smoke OK seed=$SEED hash=$NATIVE_HASH"
exit 0
