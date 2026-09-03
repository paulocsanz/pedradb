#!/usr/bin/env bash
# RFC-0157 P0.1 — aggregator for machine-checked Verus twins.
#
# Runs the existing per-twin runners (scripts/verus_*.sh) against a pinned
# Verus toolchain and reports PASS/FAIL per twin, exiting nonzero on any FAIL.
#
# Piso que este multiplicador NÃO derruba (RFC-0157):
#   - não prova o SO nem o dispositivo (R-fsync-lie); os gêmeos são do kernel
#     de decisão, não do mundo físico;
#   - corpus completo desde P2.1: --all cobre também os data_fate;
#   - R-verus segue no never_floor: um bug no verus/z3 é residual.
#
# Toolchain (pinned): GitHub release release/0.2026.08.23.fbbbbcf.
#   arm64-macos / x86-macos / x86-linux zips; installed under ~/.local/verus/.
# Resolution order (mirrors the per-twin runners):
#   1. $VERUS (executable)
#   2. ~/.local/verus/verus-arm64-macos/verus
#   3. `verus` on PATH
#   4. auto-install from the pinned release (VERUS_AUTO_INSTALL=0 disables)
#   5. exit 127 with the documented container fallback below
#
# Documented container fallback (hosts where the pinned binary does not run):
#   VERUS_CHECK_CONTAINER=1 ./scripts/formal/verus_check.sh [--all]
# runs the SAME twin set inside a throwaway container (docker required) that
# installs the pinned x86-linux build into the container and mounts the repo.
#
# Usage:
#   ./scripts/formal/verus_check.sh          # first twin set (0155:
#                                            #   group_commit, sst_crc_fate, wal_recover)
#   ./scripts/formal/verus_check.sh --all    # every scripts/verus_*.sh twin
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

VERUS_RELEASE="release/0.2026.08.23.fbbbbcf"
VERUS_VER="0.2026.08.23.fbbbbcf"
DEFAULT_SET=(group_commit sst_crc_fate wal_recover)

die() { echo "error: $*" >&2; exit 1; }

host_asset() {
  local s m
  s="$(uname -s)"; m="$(uname -m)"
  if [[ "$s" == Darwin && "$m" == arm64 ]]; then echo "arm64-macos"
  elif [[ "$s" == Darwin ]]; then echo "x86-macos"
  elif [[ "$s" == Linux && ( "$m" == x86_64 || "$m" == amd64 ) ]]; then echo "x86-linux"
  else return 1
  fi
}

auto_install() {
  local asset dir zip url
  asset="$(host_asset)" || die "no pinned verus asset for $(uname -s)/$(uname -m); use the container fallback (see header)"
  dir="$HOME/.local/verus/verus-$asset"
  [[ -x "$dir/verus" ]] && { echo "$dir/verus"; return 0; }
  zip="$HOME/.local/verus/verus-$VERUS_VER-$asset.zip"
  url="https://github.com/verus-lang/verus/releases/download/${VERUS_RELEASE//\//%2F}/verus-$VERUS_VER-$asset.zip"
  echo "verus_check: installing pinned toolchain $VERUS_VER ($asset) to $dir" >&2
  mkdir -p "$HOME/.local/verus"
  curl -sL --retry 3 -o "$zip" "$url" || die "download failed: $url"
  rm -rf "$dir.unpacked"
  mkdir -p "$dir.unpacked"
  if (command -v ditto >/dev/null 2>&1); then ditto -x -k "$zip" "$dir.unpacked"
  else unzip -q "$zip" -d "$dir.unpacked"; fi
  rm -rf "$dir"
  mkdir -p "$dir"
  mv "$dir.unpacked"/* "$dir"/
  rm -rf "$dir.unpacked" "$zip"
  chmod +x "$dir/verus"
  [[ -x "$dir/verus" ]] || die "install incomplete: $dir/verus missing"
  echo "$dir/verus"
}

resolve_verus() {
  if [[ -x "${VERUS:-}" ]]; then echo "$VERUS"; return 0; fi
  if [[ -x "$HOME/.local/verus/verus-arm64-macos/verus" ]]; then
    echo "$HOME/.local/verus/verus-arm64-macos/verus"; return 0
  fi
  if command -v verus >/dev/null 2>&1; then command -v verus; return 0; fi
  if [[ "${VERUS_AUTO_INSTALL:-1}" != 0 ]]; then auto_install; return $?; fi
  return 1
}

container_fallback() {
  command -v docker >/dev/null 2>&1 || die "docker not found for container fallback"
  echo "verus_check: running the same set in a container (pinned $VERUS_VER x86-linux)" >&2
  exec docker run --rm --platform linux/amd64 \
    -v "$ROOT":/work -w /work -e VERUS_AUTO_INSTALL=0 \
    debian:bookworm-slim bash -c '
      set -e
      apt-get update -qq && apt-get install -y -qq curl unzip ca-certificates >/dev/null
      d=/opt/verus; zip=/tmp/verus.zip
      u="https://github.com/verus-lang/verus/releases/download/release%2F0.2026.08.23.fbbbbcf/verus-0.2026.08.23.fbbbbcf-x86-linux.zip"
      curl -sL --retry 3 -o "$zip" "$u"
      mkdir -p "$d.unpacked" && unzip -q "$zip" -d "$d.unpacked"
      mkdir -p "$d" && mv "$d.unpacked"/* "$d"/
      export VERUS="$d/verus"
      bash scripts/formal/verus_check.sh "$@"
    ' _ "$@"
}

twin_names() {
  if [[ "${1:-}" == "--all" ]]; then
    (cd "$ROOT/scripts" && ls verus_*.sh | sed 's/^verus_//; s/\.sh$//' | sort)
  else
    printf '%s\n' "${DEFAULT_SET[@]}"
  fi
}

[[ "${1:-}" == "--all" || $# -eq 0 ]] || die "usage: $0 [--all]"

if [[ "${VERUS_CHECK_CONTAINER:-0}" == 1 ]]; then container_fallback "$@"; fi

VERUS_BIN="$(resolve_verus)" || die "verus not found; set VERUS=, install the pinned release to ~/.local/verus/, or use VERUS_CHECK_CONTAINER=1 (see header)"

echo "verus: $VERUS_BIN"
"$VERUS_BIN" --version || die "pinned toolchain failed to run"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

pass=0; fail=0; failed=()
printf '%-28s %-6s %s\n' TWIN RESULT TIME
while IFS= read -r name; do
  runner="$ROOT/scripts/verus_$name.sh"
  [[ -f "$runner" ]] || { printf '%-28s %-6s %s\n' "$name" MISS "no scripts/verus_$name.sh"; fail=$((fail+1)); failed+=("$name (missing runner)"); continue; }
  log="$TMP/$name.log"
  start=$(date +%s)
  if VERUS="$VERUS_BIN" bash "$runner" >"$log" 2>&1; then
    printf '%-28s %-6s %ss\n' "$name" PASS "$(( $(date +%s) - start ))"
    pass=$((pass+1))
  else
    printf '%-28s %-6s %ss\n' "$name" FAIL "$(( $(date +%s) - start ))"
    fail=$((fail+1)); failed+=("$name")
    grep -E "error\[|error:" "$log" | head -5 | sed 's/^/    /'
  fi
done < <(twin_names "${1:-}")

echo
echo "verus_check: $pass pass, $fail fail (pinned $VERUS_VER)"
if [[ $fail -gt 0 ]]; then
  printf 'failed twins: %s\n' "${failed[*]}"
  exit 1
fi
exit 0
