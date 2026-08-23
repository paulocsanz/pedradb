#!/usr/bin/env bash
# Compare native ARM64 vs Docker linux/amd64 (QEMU TCG on Apple Silicon)
# wall-time ratio of 1 KiB vs 16 KiB memcpy, and (if qemu-user plugins
# exist) guest instruction counts. Not a pedradb ratio.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
SRC="$ROOT/workloads/copy_bias.c"
OUT="$ROOT/copy_bias.txt"
: >"$OUT"

echo "== host $(uname -m) native clang ==" | tee -a "$OUT"
clang -O2 -o /tmp/copy_bias_host "$SRC"
/usr/bin/time -p /tmp/copy_bias_host | tee -a "$OUT"
/usr/bin/time -p /tmp/copy_bias_host | tee -a "$OUT"

run_plat() {
  local plat="$1"
  echo "== docker $plat ==" | tee -a "$OUT"
  docker run --rm --platform "$plat" \
    -v "$SRC":/src/copy_bias.c:ro \
    ubuntu:24.04 \
    bash -lc 'set -euo pipefail
      export DEBIAN_FRONTEND=noninteractive
      apt-get update -qq
      apt-get install -y -qq gcc time >/dev/null
      gcc -O2 -o /tmp/copy_bias /src/copy_bias.c
      echo uname=$(uname -m)
      /usr/bin/time -p /tmp/copy_bias
      /usr/bin/time -p /tmp/copy_bias
    ' | tee -a "$OUT"
}

run_plat linux/arm64
run_plat linux/amd64

echo "== qemu-user insn count (arm64 container, x86_64 guest binary) ==" | tee -a "$OUT"
docker run --rm --platform linux/arm64 \
  -v "$SRC":/src/copy_bias.c:ro \
  -v "$ROOT/plugin/insn_count.c":/src/insn_count.c:ro \
  ubuntu:24.04 \
  bash -lc 'set -euo pipefail
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq gcc gcc-x86-64-linux-gnu qemu-user qemu-user-static libglib2.0-dev pkg-config time >/dev/null
    QEMU_VER=$(qemu-x86_64 -version | head -1)
    echo "qemu=$QEMU_VER"
    # Match plugin ABI to the installed qemu.
    HDR=$(find /usr -name qemu-plugin.h 2>/dev/null | head -1 || true)
    if [ -z "$HDR" ]; then
      apt-get install -y -qq wget >/dev/null
      mkdir -p /tmp/qhdr
      # Ubuntu 24.04 is qemu 8.2.x (plugin API 4 or 5).
      wget -q -O /tmp/qhdr/qemu-plugin.h \
        https://raw.githubusercontent.com/qemu/qemu/v8.2.2/include/qemu/qemu-plugin.h || true
      HDR=/tmp/qhdr/qemu-plugin.h
    fi
    echo "hdr=$HDR"
    x86_64-linux-gnu-gcc -O2 -static -o /tmp/copy_bias_x64 /src/copy_bias.c || \
      x86_64-linux-gnu-gcc -O2 -o /tmp/copy_bias_x64 /src/copy_bias.c
    gcc -shared -fPIC -o /tmp/libinsn_count.so /src/insn_count.c \
      -I"$(dirname "$HDR")" $(pkg-config --cflags glib-2.0) || {
        echo "plugin compile failed; falling back to wall-only TCG"
        /usr/bin/time -p qemu-x86_64 /tmp/copy_bias_x64
        /usr/bin/time -p qemu-x86_64 /tmp/copy_bias_x64
        exit 0
      }
    qemu-x86_64 -plugin /tmp/libinsn_count.so -d plugin /tmp/copy_bias_x64
    qemu-x86_64 -plugin /tmp/libinsn_count.so -d plugin /tmp/copy_bias_x64
  ' | tee -a "$OUT"

echo "wrote $OUT"
