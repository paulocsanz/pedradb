#!/usr/bin/env bash
# Official sorted-ingest scale-ladder campaign (snapshot_backends harness).
#
# Protocol (docs/benchmarks.md, "Official leg protocol"): one backend per
# process, Pedra leg first, 3 runs per cell, criterion mid of [lo,mid,hi];
# 200 B values, 256 MiB block cache, Pedra bulk-stage clamp 64 MiB, TMPDIR
# on real NVMe (never tmpfs), vm.dirty_ratio=5 / vm.dirty_background_ratio=1.
# A 50k-entry smoke per backend must exit 0 before any official leg —
# this script enforces that order.
#
# Usage (Linux guest, from the repo root):
#   scripts/scale-ladder-campaign.sh                        # 1M..500M, 3 runs
#   SIZES="250000000 500000000" scripts/scale-ladder-campaign.sh
#   RUNS=1 BACKENDS="pedradb rocksdb fjall" scripts/scale-ladder-campaign.sh
#
# Output: bench-results/scale-ladder-<timestamp>/ with one full log per leg
# (stdout+stderr: hydrate/settle/probe percentiles + criterion tables), a
# manifest naming tree/hardware/sysctls, and <out>.tar.gz to hand back.
# Resume-safe: a leg whose run<N>.exitcode says 0 is skipped — re-invoke
# with the same OUT= after a crash and only the missing legs re-run.
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SB="$ROOT/crates/snapshot-bench"

SIZES="${SIZES:-1000000 10000000 25000000 100000000 250000000 500000000}"
RUNS="${RUNS:-3}"
BACKENDS="${BACKENDS:-pedradb rocksdb}"
FILTER="${FILTER:-get_hit|prefix_scan|lookup_100}"
OUT="${OUT:-$ROOT/bench-results/scale-ladder-$(date +%Y%m%d-%H%M%S)}"

die() { echo "FATAL: $*" >&2; exit 1; }
note() { echo "== $*"; }

# ---------------------------------------------------------------- preflight
command -v cargo >/dev/null 2>&1 || die "cargo not found (need a Rust toolchain)"
command -v rustc >/dev/null 2>&1 || die "rustc not found"
if ! command -v cc >/dev/null 2>&1 && ! command -v gcc >/dev/null 2>&1 && \
   ! command -v clang >/dev/null 2>&1; then
    die "no C compiler found: the rocksdb feature needs a C++ toolchain + libclang (debian/ubuntu: apt install build-essential clang libclang-dev)"
fi

# Scratch dir on the fattest writable non-tmpfs mount unless told
# otherwise. On a single-volume VM that mount is / — and that is the NVMe.
if [ -z "${PEDRA_BENCH_TMPDIR:-}" ]; then
    TMP="$(df -k -P \
        | awk 'NR>1 && $1 !~ /^(@?tmpfs|devtmpfs|none|efivarfs|overlay)/ { print $4+0, $6 }' \
        | sort -gr \
        | while read -r _kb _mp; do
              d="${_mp}/.pedra-bench-scratch"
              if mkdir -p "$d" 2>/dev/null && [ -w "$d" ]; then printf '%s\n' "$d"; break; fi
          done)"
    [ -n "$TMP" ] || die "could not find a writable scratch mount; set PEDRA_BENCH_TMPDIR=/path/on/nvme"
else
    TMP="$PEDRA_BENCH_TMPDIR"
fi
mkdir -p "$TMP" || die "cannot create scratch dir $TMP"
AVAIL_KB="$(df -k -P "$TMP" | awk 'NR==2 {print $4}')"
AVAIL_GB=$(( AVAIL_KB / 1024 / 1024 ))
note "scratch: $TMP (${AVAIL_GB} GiB free) — must be real NVMe, not tmpfs"

# Disk: tree ≈ 300 B/entry (Pedra 259, Rocks ~210–274 observed), settle
# headroom 1.3–1.4x (see maybe_settle in the harness). Require 420 B/entry.
TOO_SMALL=""
for n in $SIZES; do
    need_kb=$(( n * 420 / 1024 ))
    if [ "$AVAIL_KB" -lt "$need_kb" ]; then
        TOO_SMALL="$TOO_SMALL
  ${n} entries needs ~$(( need_kb / 1024 / 1024 )) GiB, have ${AVAIL_GB} GiB"
    fi
done
[ -z "$TOO_SMALL" ] || die "not enough free space under $TMP for:$TOO_SMALL
Scope down (SIZES=\"...\") or point PEDRA_BENCH_TMPDIR at a bigger NVMe mount."

# Protocol sysctls. These are measurement-environment control, NOT a
# product requirement — Pedra runs on any default kernel; nobody tunes
# these outside a benchmark lab. The script applies them itself (one
# sudo prompt at start; transient, gone on reboot) so reproducing the
# published ladder stays one command. KERNEL_DEFAULTS=1 skips them
# entirely and measures out-of-the-box kernel behavior instead — a
# deliberate protocol deviation vs the September cells, recorded in the
# manifest (expect more run-to-run variance on write-heavy legs).
if [ "${KERNEL_DEFAULTS:-0}" = "1" ]; then
    note "KERNEL_DEFAULTS=1: kernel untouched — measuring default writeback behavior (protocol deviation, recorded in manifest)"
elif sudo sysctl -w vm.dirty_ratio=5 vm.dirty_background_ratio=1 >/dev/null 2>&1; then
    note "sysctls set (measurement control only): vm.dirty_ratio=5 vm.dirty_background_ratio=1"
else
    echo "== WARNING: could not set vm.dirty_ratio=5 / vm.dirty_background_ratio=1 (no sudo?)."
    echo "   Continuing with the kernel as-is; the manifest records the live values."
fi

RAM_KB="$(awk '/MemTotal/ {print $2}' /proc/meminfo 2>/dev/null || echo 0)"
if [ "$RAM_KB" -gt 0 ] && [ "$RAM_KB" -lt 3600000 ]; then
    echo "== WARNING: only $(( RAM_KB / 1024 / 1024 )) GiB RAM. The official guest is 4 GiB;"
    echo "   at 100M the settle peak was 1.65 GiB. Larger rungs may OOM — that is data, not a crash of the script."
fi

mkdir -p "$OUT"

# ---------------------------------------------------------------- manifest
{
    echo "date:        $(date -Is 2>/dev/null || date)"
    echo "host:        $(uname -a)"
    echo "cpus:        $(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN 2>/dev/null || echo '?')"
    awk '/MemTotal/ {printf "ram:         %.1f GiB\n", $2/1048576}' /proc/meminfo 2>/dev/null
    echo "tree:        $(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
    echo "tree_dirty:  $(git -C "$ROOT" status --porcelain 2>/dev/null | wc -l | tr -d ' ') uncommitted files"
    echo "remote:      $(git -C "$ROOT" remote get-url origin 2>/dev/null || echo unknown)"
    echo "scratch:     $TMP"
    SRC_DEV="$(df -P "$TMP" | awk 'NR==2 {print $1}')"
    echo "scratch_dev: $SRC_DEV $(lsblk -ndo MODEL "$SRC_DEV" 2>/dev/null || true)"
    echo "free:        ${AVAIL_GB} GiB"
    echo "sysctls:     vm.dirty_ratio=$(cat /proc/sys/vm/dirty_ratio 2>/dev/null) vm.dirty_background_ratio=$(cat /proc/sys/vm/dirty_background_ratio 2>/dev/null)"
    echo "rustc:       $(rustc --version 2>/dev/null)"
    echo "cargo:       $(cargo --version 2>/dev/null)"
    echo "sizes:       $SIZES"
    echo "kernel_defaults: ${KERNEL_DEFAULTS:-0} (0 = protocol sysctls vm.dirty_ratio=5 vm.dirty_background_ratio=1 applied)"
    echo "runs:        $RUNS"
    echo "backends:    $BACKENDS"
    echo "filter:      $FILTER"
} > "$OUT/manifest.txt"

# ---------------------------------------------------------------- legs
# One leg = one backend, one run, one size: fresh process, fresh tree.
# Official order: Pedra leg first, then the Rocks leg of the same run.
run_leg() { # $1 backend  $2 runno  $3 entries
    local b="$1" r="$2" entries="$3"
    local dir="$OUT/$entries/$b"
    local log="$dir/run$r.log"
    local marker="$dir/run$r.exitcode"
    mkdir -p "$dir"
    if [ -f "$marker" ] && [ "$(cat "$marker")" = "0" ]; then
        note "$entries/$b run$r: already done, skipping"
        return 0
    fi
    note "$entries/$b run$r: start $(date +%H:%M:%S)"
    local rc=0
    (
        cd "$SB" || exit 1
        SLIPSTREAM_BENCH_BACKENDS="$b" \
        SLIPSTREAM_BENCH_ENTRIES="$entries" \
        SLIPSTREAM_BENCH_VALUE_BYTES=200 \
        SLIPSTREAM_BENCH_CACHE_BYTES=268435456 \
        PEDRA_STAGE_MAX_BYTES=67108864 \
        TMPDIR="$TMP" \
        cargo bench --bench snapshot_backends --features fjall,rocksdb,pedradb \
            -- "$FILTER"
    ) > "$log" 2>&1 || rc=$?
    echo "$rc" > "$marker"
    sync
    if [ "$rc" = 0 ]; then
        echo "   ok -> $log"
    else
        echo "   FAILED (exit $rc) -> $log (campaign continues)"
    fi
    return 0
}

note "smoke: 50k entries per backend must exit 0 before any official leg"
for b in $BACKENDS; do
    run_leg "$b" 1 50000
done
smoke_bad=0
for b in $BACKENDS; do
    [ "$(cat "$OUT/50000/$b/run1.exitcode" 2>/dev/null)" = 0 ] || smoke_bad=1
done
[ "$smoke_bad" = 0 ] || die "smoke failed — fix the build/env before official legs (see $OUT/50000/)"

note "official legs: sizes=[$SIZES] runs=$RUNS backends=[$BACKENDS]"
note "rough wall-clock on the 4 vCPU guest: 1M+10M ~35m, 25M ~25m, 100M ~35m, 250M ~55m, 500M ~105m, plus the first build (10–20m with RocksDB C++)"
for n in $SIZES; do
    for r in $(seq 1 "$RUNS"); do
        for b in $BACKENDS; do
            run_leg "$b" "$r" "$n"
        done
    done
done

# ---------------------------------------------------------------- wrap-up
fails=0
while IFS= read -r m; do
    [ "$(cat "$m")" = 0 ] || { fails=$(( fails + 1 )); echo "FAILED leg: $m"; }
done < <(find "$OUT" -name '*.exitcode' | sort)

TARBALL="$OUT.tar.gz"
tar -czf "$TARBALL" -C "$(dirname "$OUT")" "$(basename "$OUT")"
note "done: $(( $(find "$OUT" -name '*.exitcode' | wc -l | tr -d ' ') - fails )) legs ok, $fails failed"
note "manifest: $OUT/manifest.txt"
note "hand back: $TARBALL"
[ "$fails" = 0 ] || exit 1
exit 0
