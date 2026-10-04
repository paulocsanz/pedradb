#!/usr/bin/env bash
# C+ASan product gate for pedradb-capi (in-process ABI — not libfdb_c).
#
# PASS  = well-behaved C + rotten handles + oversize lens (F215).
# FAIL  = malicious short-buffer slices; ASan must fire on each case.
#
# The Rust unit tests never link a C TU. This script is the gate.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

ASAN_REQUIRED="${ASAN_REQUIRED:-}"
CC="${CC:-clang}"
OUT="${CAPI_ASAN_OUT:-$ROOT/target/capi-asan}"
HARNESS="$ROOT/crates/pedradb-capi/harness"
HEADER_DIR="$ROOT/include"

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    if [[ "$ASAN_REQUIRED" == 1 ]]; then
      echo "capi-asan: missing $1 (ASAN_REQUIRED=1)" >&2
      exit 1
    fi
    echo "CAPI_ASAN_RESIDUAL: install $1 (or set ASAN_REQUIRED=1 in CI)"
    exit 0
  fi
}

need cargo
need "$CC"

mkdir -p "$OUT"

echo "== cargo build -p pedradb-capi (staticlib) =="
cargo build -p pedradb-capi --offline 2>/dev/null || cargo build -p pedradb-capi

LIB=""
for cand in \
  "$ROOT/target/debug/libpedradb_capi.a" \
  "$ROOT/target/${CARGO_BUILD_TARGET:-}/debug/libpedradb_capi.a"; do
  if [[ -f "$cand" ]]; then
    LIB="$cand"
    break
  fi
done
# Workspace target-dir may be nested; last-resort find.
if [[ -z "$LIB" ]]; then
  LIB="$(find "$ROOT/target" -name 'libpedradb_capi.a' -print -quit 2>/dev/null || true)"
fi
if [[ -z "$LIB" || ! -f "$LIB" ]]; then
  echo "capi-asan: libpedradb_capi.a not found after cargo build" >&2
  exit 1
fi
echo "staticlib: $LIB"

# `--print native-static-libs` needs a crate input; a dummy staticlib
# prints the same host extras we must pass when linking the C TU.
echo '#![crate_type="staticlib"]' > "$OUT/nsl.rs"
NATIVE_LIBS="$(rustc --print native-static-libs "$OUT/nsl.rs" -o "$OUT/nsl.a" 2>&1 | sed -n 's/.*native-static-libs: //p' | tail -1)"
if [[ -z "$NATIVE_LIBS" ]]; then
  echo "capi-asan: rustc did not print native-static-libs" >&2
  exit 1
fi
echo "native-static-libs: $NATIVE_LIBS"
# Apple clang + ASan: keep the C TU instrumented; the .a is not. memcpy /
# memchr are interposed (Darwin does not intercept strnlen).
CFLAGS=(-fsanitize=address -fno-omit-frame-pointer -g -O1 -Wall -Wextra)
# Rust TLS / std startup can look like leaks against a C main.
export ASAN_OPTIONS="${ASAN_OPTIONS:-abort_on_error=1:halt_on_error=1:detect_leaks=0}"

link_one() {
  local src="$1" bin="$2"
  # shellcheck disable=SC2086
  "$CC" "${CFLAGS[@]}" -I "$HEADER_DIR" -o "$bin" "$src" "$LIB" $NATIVE_LIBS
}

echo "== link C PASS binary =="
link_one "$HARNESS/capi_asan.c" "$OUT/capi_asan"
echo "== link C malicious binary =="
link_one "$HARNESS/capi_asan_malicious.c" "$OUT/capi_asan_malicious"

echo "== PASS (honest C + rotten handles + oversize caps) =="
pass_log="$OUT/capi_asan.log"
set +e
"$OUT/capi_asan" >"$pass_log" 2>&1
pass_rc=$?
set -e
cat "$pass_log"
if [[ "$pass_rc" -ne 0 ]]; then
  echo "capi-asan: PASS binary failed (rc=$pass_rc)" >&2
  exit 1
fi
# RFC-0075 P1.2: PASS must name the same LIMIT gate as `c_len_admitted`
# (null-handle oversize + live create+tx). Dropping those CHECKs without
# the banners fails this script even if the binary still exits 0.
for tooth in "LIMIT key" "LIMIT value" "LIMIT get" "LIMIT live-key"; do
  if ! grep -q "capi_asan: ${tooth}" "$pass_log"; then
    echo "capi-asan: PASS binary missing ${tooth} (RFC-0075 P1.2)" >&2
    exit 1
  fi
done
echo "capi-asan: PASS LIMIT teeth present"

asan_hit() {
  local log="$1"
  grep -Eqi 'ERROR: AddressSanitizer|AddressSanitizer:.*(heap-buffer-overflow|stack-buffer-overflow|buffer-overflow)' "$log"
}

expect_asan() {
  local case="$1"
  local log="$OUT/malicious-${case}.log"
  echo "== malicious ${case} (expect ASan FAIL) =="
  set +e
  # Subshell so SIGABRT is not printed as "Abort trap" on the script line.
  ( "$OUT/capi_asan_malicious" "$case" >"$log" 2>&1 )
  local rc=$?
  set -e
  if [[ $rc -eq 0 ]]; then
    echo "capi-asan: malicious ${case} exited 0 — ASan did not catch the slice lie" >&2
    cat "$log" >&2
    exit 1
  fi
  if ! asan_hit "$log"; then
    echo "capi-asan: malicious ${case} died (rc=$rc) but without an ASan report" >&2
    cat "$log" >&2
    exit 1
  fi
  echo "malicious ${case}: ASan FAIL as required (rc=$rc)"
}

expect_asan key
expect_asan value
expect_asan path

echo "capi-asan: ok (PASS binary green; malicious key/value/path ASan-red)"
