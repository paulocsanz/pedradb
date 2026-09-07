#!/usr/bin/env bash
# RFC-0172: translation validation rustc→LLVM IR→object for one pinned
# target, one file (write_admission_kernel.rs). Does not prove rustc.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PIN="$ROOT/findings/2026-09-06-rfc0172-tv/pin.txt"
DUMP_DIR="$ROOT/findings/2026-09-06-rfc0172-tv"
SRC="$ROOT/crates/pedradb-core/src/write_admission_kernel.rs"
UPDATE=0
if [[ "${1:-}" == "--update" ]]; then
  UPDATE=1
fi

die() { echo "FAIL  $*" >&2; exit 1; }

[[ -f "$PIN" ]] || die "missing $PIN"
[[ -f "$SRC" ]] || die "missing $SRC"

pin() { awk -F= -v k="$1" '$1==k {print $2; exit}' "$PIN"; }

WANT_TRIPLE="$(pin triple)"
WANT_RUSTC="$(pin rustc)"
WANT_HASH="$(pin commit-hash)"
WANT_LLVM="$(pin llvm)"
OPT="$(pin opt-level)"
CRATE="$(pin crate-name)"

VV="$(rustc -vV)"
HAVE_HOST="$(printf '%s\n' "$VV" | awk -F': ' '/^host:/{print $2}')"
HAVE_REL="$(printf '%s\n' "$VV" | awk -F': ' '/^release:/{print $2}')"
HAVE_HASH="$(printf '%s\n' "$VV" | awk -F': ' '/^commit-hash:/{print $2}')"
HAVE_LLVM="$(printf '%s\n' "$VV" | awk -F': ' '/^LLVM version:/{print $2}')"

[[ "$HAVE_HOST" == "$WANT_TRIPLE" ]] || die "host $HAVE_HOST != pin triple $WANT_TRIPLE"
[[ "$HAVE_REL" == "$WANT_RUSTC" ]] || die "rustc $HAVE_REL != pin $WANT_RUSTC"
[[ "$HAVE_HASH" == "$WANT_HASH" ]] || die "commit-hash $HAVE_HASH != pin $WANT_HASH"
[[ "$HAVE_LLVM" == "$WANT_LLVM" ]] || die "LLVM $HAVE_LLVM != pin $WANT_LLVM"
echo "ok    pin rustc=$HAVE_REL host=$HAVE_HOST llvm=$HAVE_LLVM"

SYSROOT="$(rustc --print sysroot)"
NM="$SYSROOT/lib/rustlib/${HAVE_HOST}/bin/llvm-nm"
if [[ ! -x "$NM" ]]; then
  NM="$(command -v llvm-nm || true)"
fi
if [[ ! -x "$NM" ]]; then
  NM="/usr/bin/nm"
fi
[[ -x "$NM" ]] || die "nm/llvm-nm not found"

TMP="$(mktemp -d "${TMPDIR:-/tmp}/tv-wa.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

# Unoptimized IR is the rustc translation of the term (O1 DCE'd the rlib).
rustc --crate-type=lib --crate-name="$CRATE" --edition=2021 \
  --emit=llvm-ir,obj \
  -C "opt-level=${OPT}" -C debuginfo=0 \
  "$SRC" -o "$TMP/write_admission" 2>"$TMP/rustc.err" \
  || { cat "$TMP/rustc.err" >&2; die "rustc emit failed"; }

LL="$TMP/write_admission.ll"
OBJ="$TMP/write_admission.o"
[[ -f "$LL" && -f "$OBJ" ]] || die "rustc did not emit .ll/.o"

KERNEL_SHA="$(shasum -a 256 "$SRC" | awk '{print $1}')"
IR_SHA="$(shasum -a 256 "$LL" | awk '{print $1}')"
OBJ_SHA="$(shasum -a 256 "$OBJ" | awk '{print $1}')"

# P1.1: v0-mangled symbols (len-prefixed) for the two exec fns.
SYMS="$("$NM" "$OBJ")"
printf '%s\n' "$SYMS" | grep -q '20write_admission_idle$' \
  || die "object missing write_admission_idle symbol"
printf '%s\n' "$SYMS" | grep -q '11write_admit$' \
  || die "object missing write_admit symbol"
echo "ok    symbols write_admission_idle + write_admit in object"

# P2.1: IR of idle matches the 8-point spec.
python3 "$ROOT/scripts/tv_write_admission_ir.py" "$LL"

STAMP="$DUMP_DIR/SOURCE.tv"
if [[ "$UPDATE" -eq 1 ]]; then
  mkdir -p "$DUMP_DIR"
  cp "$LL" "$DUMP_DIR/write_admission.ll"
  {
    echo "path=crates/pedradb-core/src/write_admission_kernel.rs"
    echo "kernel_sha256=$KERNEL_SHA"
    echo "ir_sha256=$IR_SHA"
    echo "obj_sha256=$OBJ_SHA"
    echo "triple=$WANT_TRIPLE"
    echo "rustc=$WANT_RUSTC"
    echo "opt-level=$OPT"
  } > "$STAMP"
  echo "ok    updated $STAMP"
  exit 0
fi

[[ -f "$STAMP" ]] || die "missing $STAMP (run $0 --update)"
WANT_K="$(awk -F= '/^kernel_sha256=/{print $2}' "$STAMP")"
WANT_IR="$(awk -F= '/^ir_sha256=/{print $2}' "$STAMP")"
[[ "$KERNEL_SHA" == "$WANT_K" ]] || die "kernel sha256 drifted ($KERNEL_SHA vs $WANT_K); re-run $0 --update"
[[ "$IR_SHA" == "$WANT_IR" ]] || die "LLVM IR sha256 drifted ($IR_SHA vs $WANT_IR); rustc translation changed or kernel drifted; re-run $0 --update"
COMMITTED_LL="$DUMP_DIR/write_admission.ll"
[[ -f "$COMMITTED_LL" ]] || die "missing committed IR dump $COMMITTED_LL"
cmp -s "$LL" "$COMMITTED_LL" || die "emitted IR != committed dump"
echo "ok    SOURCE.tv kernel+IR sha256 bind"
echo "ok    tv_write_admission"
