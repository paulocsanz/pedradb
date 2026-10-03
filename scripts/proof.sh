#!/usr/bin/env bash
# proof.sh — the ONE parameterized proof driver.
#
# The per-kernel scripts (aeneas_*.sh, verus_*.sh, lean_*.sh) used to be
# ~120 hand copies of the same toolchain-resolution / extract / stamp
# logic, drifting one byte at a time (the caixote-era copy-paste
# problem). They are now generated one-line shims that exec this driver
# with their parameters — see scripts/gen_proof_shims.py. Scripts with
# genuinely custom post-processing (extract patches) stay handwritten.
#
# Zero magic on purpose: plain bash, no framework, readable by an
# external auditor in one sitting. Behavior is preserved from the
# templates it replaced (skip/--required semantics, exit codes, output
# messages, SOURCE stamp format).
#
#   proof.sh verus  <src> [verus args...]          # no skip mode: exits 127 without verus
#   proof.sh aeneas --crate <dir> --src <path> --base <name> --source-id <id> [--required]
#   proof.sh lean   --main <Lib> --build "<libs...>" [--required]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FAMILY="${1:?usage: proof.sh verus|aeneas|lean ...}"
shift

case "$FAMILY" in
verus)
    SRC="${1:?usage: proof.sh verus <src> [args...]}"
    shift
    if [[ -x "${VERUS:-}" ]]; then
        :
    elif [[ -x "$HOME/.local/verus/verus-arm64-macos/verus" ]]; then
        VERUS="$HOME/.local/verus/verus-arm64-macos/verus"
    elif command -v verus >/dev/null 2>&1; then
        VERUS="$(command -v verus)"
    else
        echo "error: verus not found (install to ~/.local/verus/verus-arm64-macos or set VERUS=)" >&2
        exit 127
    fi
    echo "verus: $VERUS"
    "$VERUS" --version
    echo "proving: $SRC"
    exec "$VERUS" "$SRC" --crate-type=lib --multiple-errors 10 --time "$@"
    ;;

aeneas)
    CRATE=""
    SRC=""
    BASE=""
    SOURCE_ID=""
    REQUIRED=0
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --crate)     CRATE="$ROOT/$2"; shift 2 ;;
            --src)       SRC="$ROOT/$2"; shift 2 ;;
            --base)      BASE="$2"; shift 2 ;;
            --source-id) SOURCE_ID="$2"; shift 2 ;;
            --required)  REQUIRED=1; shift ;;
            *) echo "proof.sh aeneas: unknown arg: $1" >&2; exit 2 ;;
        esac
    done
    if [[ -z "$CRATE" || -z "$SRC" || -z "$BASE" || -z "$SOURCE_ID" ]]; then
        echo "proof.sh aeneas: --crate, --src, --base and --source-id are all required" >&2
        exit 2
    fi
    OUT="$ROOT/formal/aeneas/out"

    CHARON="${CHARON:-$(command -v charon || true)}"
    AENEAS="${AENEAS:-$(command -v aeneas || true)}"
    if [[ -z "$CHARON" || -z "$AENEAS" ]]; then
        msg="charon/aeneas not on PATH (set CHARON= AENEAS=). See formal/aeneas/PINS.md"
        if [[ "$REQUIRED" -eq 1 ]]; then
            echo "FAIL  $msg" >&2
            exit 1
        fi
        echo "skip  $msg"
        exit 0
    fi

    mkdir -p "$OUT"
    echo "      charon=$CHARON"
    echo "      aeneas=$AENEAS"
    (
        cd "$CRATE"
        "$CHARON" cargo --preset=aeneas --dest-file "$OUT/${BASE}.llbc"
    )
    "$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/${BASE}.llbc"
    {
        echo "path=${SRC#"$ROOT"/}"
        echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
        echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
        echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
    } > "$OUT/SOURCE.$SOURCE_ID"
    echo "ok    extract $SOURCE_ID → $OUT"
    ;;

lean)
    MAIN=""
    BUILD=""
    REQUIRED=0
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --main)     MAIN="$2"; shift 2 ;;
            --build)    BUILD="$2"; shift 2 ;;
            --required) REQUIRED=1; shift ;;
            *) echo "proof.sh lean: unknown arg: $1" >&2; exit 2 ;;
        esac
    done
    if [[ -z "$MAIN" || -z "$BUILD" ]]; then
        echo "proof.sh lean: --main and --build are required" >&2
        exit 2
    fi
    LEAN_DIR="$ROOT/formal/aeneas/lean"

    export PATH="${HOME}/.elan/bin:${PATH}"
    LAKE="${LAKE:-$(command -v lake || true)}"
    if [[ -z "$LAKE" ]]; then
        msg="lake not on PATH (install elan + leanprover/lean4:v4.31.0). See formal/aeneas/PINS.md"
        if [[ "$REQUIRED" -eq 1 ]]; then
            echo "FAIL  $msg" >&2
            exit 1
        fi
        echo "skip  $msg"
        exit 0
    fi

    if [[ ! -f "$LEAN_DIR/${MAIN}.lean" || ! -e "$LEAN_DIR/${MAIN}Kernel.lean" ]]; then
        echo "FAIL  formal/aeneas/lean/{${MAIN},${MAIN}Kernel}.lean missing" >&2
        exit 1
    fi

    echo "      lake=$LAKE"
    (cd "$LEAN_DIR" && "$LAKE" build $BUILD)
    echo "ok    lean $BUILD"
    ;;

*)
    echo "proof.sh: unknown family: $FAMILY (verus|aeneas|lean)" >&2
    exit 2
    ;;
esac
