#!/usr/bin/env bash
# RFC-0052 P2.3 — TCG guest × det_io (AND, PRELOAD inside the guest).
#
# Same seed (42) as P2.1. Dynamic musl world_smoke + libdet_io.so
# (STALL_SO/LD_PRELOAD). det_io is armed drop_fsync_all+log. Serial must
# contain [det_io] intercepts (PRELOAD is not vacuous). trace_hash must
# match native (fsync drop still returns 0). Wall-clock is forbidden.
#
# TCG_REQUIRED=1: missing qemu/musl is a failure.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
SEED="${PEDRA_TCG_SEED:-42}"
CACHE="${PEDRA_TCG_CACHE:-$HOME/.cache/pedradb-tcg}"
WORK="${PEDRA_TCG_WORK:-$ROOT/target/tcg-world-smoke-detio}"
KERN_URL="${PEDRA_TCG_KERNEL_URL:-https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/x86_64/netboot/vmlinuz-virt}"
ICOUNT_SHIFT="${ICOUNT_SHIFT:-6}"
mkdir -p "$CACHE" "$WORK"

residual() {
  echo "C2.3=$1"
  echo "note=$2"
  echo "compare_wall_clock=forbidden"
  if [[ "${TCG_REQUIRED:-}" == 1 ]]; then
    echo "tcg_world_smoke_detio: required ($1)" >&2
    exit 1
  fi
  echo "tcg_world_smoke_detio OK residual"
  exit 0
}

case "${ICOUNT_SHIFT}" in
  auto|AUTO) echo "tcg_world_smoke_detio: REFUSE icount shift=auto (RFC-0005)" >&2; exit 2 ;;
esac

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    residual "residual_missing_$1" "install $1"
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
[[ -n "$NATIVE_HASH" ]] || { echo "no native hash" >&2; exit 1; }
echo "native_hash=$NATIVE_HASH"

echo "== dynamic musl world_smoke =="
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$MUSL_CC"
# Isolated target dir so we don't clobber P2.1's static musl artifact.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target/tcg-detio-cargo}"
RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=-crt-static" \
  cargo build --release -p pedradb-world --bin world_smoke --target x86_64-unknown-linux-musl
SMOKE="$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/world_smoke"
cp "$SMOKE" "$WORK/world_smoke"

echo "== libdet_io.so (musl) =="
"$MUSL_CC" -shared -fPIC -O2 -o "$WORK/libdet_io.so" \
  "$ROOT/scripts/tcg-guest/det_io.c" -ldl
"$MUSL_CC" -static -Os -s "$ROOT/scripts/tcg-guest/init_detio.c" -o "$WORK/init"

STAGE="$WORK/root"
rm -rf "$STAGE"
mkdir -p "$STAGE"/{proc,sys,tmp,dev,lib}
cp "$WORK/init" "$STAGE/init"
cp "$WORK/world_smoke" "$STAGE/world_smoke"
cp "$WORK/libdet_io.so" "$STAGE/libdet_io.so"
# Ubuntu musl-tools: /lib/ld-musl-x86_64.so.1 is the real ELF (libc.so may
# be a GNU ld script). Homebrew musl-cross: libc.so is the ELF.
is_elf() {
  python3 -c 'import pathlib,sys; sys.exit(0 if pathlib.Path(sys.argv[1]).read_bytes()[:4]==b"\x7fELF" else 1)' "$1"
}
copy_musl_dynroot() {
  local dest=$1
  mkdir -p "$dest/lib"
  if [[ -e /lib/ld-musl-x86_64.so.1 ]] && is_elf /lib/ld-musl-x86_64.so.1; then
    cp -L /lib/ld-musl-x86_64.so.1 "$dest/lib/ld-musl-x86_64.so.1"
    cp -L /lib/ld-musl-x86_64.so.1 "$dest/lib/libc.so"
  else
    local libc
    libc=$($MUSL_CC -print-file-name=libc.so)
    if [[ -f "$libc" ]] && is_elf "$libc"; then
      cp -L "$libc" "$dest/lib/libc.so"
      cp -L "$libc" "$dest/lib/ld-musl-x86_64.so.1"
    elif [[ -f "$libc" ]]; then
      local real
      real=$(grep -oE '/[^ ]+\.so[^ ]*' "$libc" | head -1 || true)
      [[ -n "$real" && -f "$real" ]] || {
        echo "cannot resolve musl libc from $libc" >&2
        exit 1
      }
      cp -L "$real" "$dest/lib/libc.so"
      cp -L "$real" "$dest/lib/ld-musl-x86_64.so.1"
    else
      echo "no musl libc ($libc)" >&2
      exit 1
    fi
  fi
  local gccs
  gccs=$($MUSL_CC -print-file-name=libgcc_s.so.1)
  if [[ -f "$gccs" ]] && is_elf "$gccs"; then
    cp -L "$gccs" "$dest/lib/libgcc_s.so.1"
  fi
}
copy_musl_dynroot "$STAGE"
chmod 755 "$STAGE/init" "$STAGE/world_smoke" "$STAGE/libdet_io.so" "$STAGE/lib/"*
( cd "$STAGE" && find . | cpio -o -H newc 2>/dev/null ) | gzip -9 > "$WORK/initramfs.cpio.gz"

echo "== TCG + det_io boot =="
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
            if "TCG_DETIO_END" in line:
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
sys.exit(0 if "TCG_DETIO_END" in text else 1)
PY
QEC=$?
set -e
if [[ "$QEC" -ne 0 ]]; then
  echo "tcg_world_smoke_detio: qemu failed" >&2
  exit 1
fi

if ! grep -q '\[det_io\]' "$SERIAL"; then
  echo "C2.3=FAIL_no_det_io_intercept"
  echo "PRELOAD did not log; libdet_io.so was not in the libc path"
  exit 1
fi
DROPS="$(grep -c 'drop=1' "$SERIAL" || true)"
echo "det_io_drop_lines=$DROPS"
GUEST_LINE="$(grep -E '^seed=' "$SERIAL" | tail -1 || true)"
GUEST_HASH="$(printf '%s\n' "$GUEST_LINE" | sed -n 's/.*hash=\([0-9a-fA-F]*\).*/\1/p')"
echo "guest_line=$GUEST_LINE"
echo "guest_hash=$GUEST_HASH"
echo "native_hash=$NATIVE_HASH"
echo "compare_wall_clock=forbidden"
echo "STALL_SO=/libdet_io.so"

if [[ -z "$GUEST_HASH" ]]; then
  echo "C2.3=FAIL_no_guest_hash"
  exit 1
fi
if [[ "$GUEST_HASH" != "$NATIVE_HASH" ]]; then
  echo "C2.3=FAIL_hash_mismatch"
  exit 1
fi
if [[ "${DROPS:-0}" -lt 1 ]]; then
  echo "C2.3=FAIL_no_fsync_drop"
  exit 1
fi
echo "C2.3=tcg_and_detio"
echo "tcg_world_smoke_detio OK seed=$SEED hash=$NATIVE_HASH drops=$DROPS"
exit 0
