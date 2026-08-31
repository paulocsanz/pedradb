#!/usr/bin/env bash
# RFC-0157 P0.3 — K-parallel REAL TCP campaign (R-swarm-real anchor).
#
# Runs K simultaneous 3-process TCP clusters via the shipped `cluster_real`
# binary (`--remove-member` path), one distinct seed per cluster, distinct
# port ranges via the L28_BASE_PORT seam, ≤3 attempts per seed with retry
# accounting. Aggregates per-seed fingerprints (napply / kernel bits /
# attempts) and registers the run under findings/rfc0157-tcp-campaign/.
#
# Piso que este multiplicador NÃO derruba:
#   - K seeds é evidência, NÃO ∀ TCP traces (R-swarm-real segue);
#     `l28_tcp_napply_retry_admitted` continua false.
#   - retry ≤3 é harness, não teorema de liveness (R-es segue).
#
# Usage: scripts/rfc0157_tcp_campaign.sh [K]   (default K=8)
# Env:   RFC0157_SEED_PREFIX (default 0x157c0) — a numeric u64 BASE; seeds
#        are base+i and must stay disjoint from every numeric seed any prior
#        campaign actually ran. Correction 2026-08-31: the old mnemonic
#        prefixes (0x0157_C001…, 0x015A_N01…) were not valid u64 literals
#        and cluster_real silently collapsed them ALL to its 0x641e28
#        default — every prior campaign ran ONE world (findings/2026-08-31-
#        campaign-seed-collapse). Seeds are numeric now and each attempt
#        cross-checks the fingerprint's echoed seed against the request.
#        RFC0157_OUT (default the findings dir above) + RFC0157_TITLE for
#        registered nightly runs. Each fingerprint must echo kill=node{n}
#        (or leader{n}); registration FAILS if K>=2 seeds collapse to a
#        single kill target (KILL-TARGET COLLAPSE) — target coverage is
#        part of the diversity claim, so it is checked, not assumed.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
K="${1:-8}"
STAGGER="${RFC0157_STAGGER:-1.5}"
SEED_PREFIX_RAW="${RFC0157_SEED_PREFIX:-0x157c0}"
# A failed $(( )) assignment still exits 0 in bash, so validate the token
# explicitly: numeric hex/decimal only — a mnemonic prefix must stop here.
[[ "$SEED_PREFIX_RAW" =~ ^(0[xX])?[0-9A-Fa-f]+$ ]] || {
  echo "error: RFC0157_SEED_PREFIX must be numeric (0x… hex or decimal), not a mnemonic: $SEED_PREFIX_RAW" >&2
  exit 2
}
SEED_BASE_NUM=$(( SEED_PREFIX_RAW ))
SEED_PREFIX="$(printf '0x%x' "$SEED_BASE_NUM")"
TITLE="${RFC0157_TITLE:-RFC-0157 P0.3 — K-parallel REAL TCP campaign registration}"
[[ "$K" =~ ^[1-9][0-9]*$ ]] || { echo "error: K must be a positive integer" >&2; exit 2; }
OUT="${RFC0157_OUT:-$ROOT/findings/rfc0157-tcp-campaign}"
LOGS="$OUT/logs"
mkdir -p "$LOGS"

# Numeric seeds (base+i), disjoint from every numeric seed a prior campaign
# actually ran. Historically "used" 0x0156_1E28/0x0157_C001+/0x015A_N001+
# never reached the binary (see header) — the only numeric seeds ever
# exercised were 0x641e28 (the silent default) and decimal 671.
seed_of() { printf '0x%x' "$(( SEED_BASE_NUM + $1 ))"; }
# Outside the pid-derived default range (23000..24503) so a concurrently
# running test binary cannot collide with the campaign.
port_of() { echo $((26000 + 3 * ($1 - 1))); }

echo "== rfc0157_tcp_campaign K=$K =="
cargo build -q -p pedradb-store --bin cluster_real --bin montanha-tcp || {
  echo "error: build failed" >&2
  exit 1
}
REAL="$ROOT/target/debug/cluster_real"
TCP="$ROOT/target/debug/montanha-tcp"
[[ -x "$REAL" && -x "$TCP" ]] || { echo "error: binaries missing" >&2; exit 1; }

# One seed: ≤3 attempts (fresh process each; log APPENDS across attempts
# for forensics). An attempt is good when the process printed a
# fingerprint carrying every required kernel bit (campaign flake mode: a
# SIGKILL of n3 before leave lands is napply=0 — retry the same seed,
# not a kernel skip).
# Output row: "<seed> <attempts> <seconds-of-last-attempt> <fingerprint...>"
run_seed() {
  local idx="$1" seed port log attempt fp dur ok start stagger
  seed="$(seed_of "$idx")"
  port="$(port_of "$idx")"
  log="$LOGS/seed_${idx}.txt"
  # Stagger first-wave starts so the wall-tick election bursts of K
  # clusters do not collide (contention stretches the bounded waits and
  # turns the n3-leave race into napply=0 flakes). 0 disables.
  stagger="$(python3 -c "print(($idx - 1) * $STAGGER)")"
  awk "BEGIN{exit !($stagger > 0)}" && sleep "$stagger"
  dur=0
  for attempt in 1 2 3; do
    echo "== attempt $attempt $(date -u +%H:%M:%S) base_port=$port ==" >>"$log"
    start=$(date +%s)
    L28_BASE_PORT="$port" MONTANHA_TCP="$TCP" "$REAL" "$seed" --remove-member \
      >>"$log" 2>&1
    dur=$(( $(date +%s) - start ))
    fp="$(grep '^cluster_real ' "$log" | tail -1 | tail -c +14 || true)"
    ok=1
    got_seed="$(grep -oE 'seed=[0-9a-f]+' <<<"$fp" | head -1 | cut -d= -f2)"
    if [[ -z "$got_seed" || "0x$got_seed" != "$seed" ]]; then
      echo "attempt $attempt SEED COLLAPSE: asked $seed, cluster ran ${got_seed:-no-seed}" >>"$log"
      ok=0
    fi
    # The fingerprint must echo WHICH node was killed (kill=node{n}) so
    # target coverage is checkable from the artifact (post-script in
    # findings/2026-08-31-campaign-seed-collapse/README.md). A bare
    # `kill=node` means an outdated binary — refuse, don't register.
    if ! grep -qE 'kill=(leader|node)[0-9]+' <<<"$fp"; then
      echo "attempt $attempt KILL TARGET NOT ECHOED (old cluster_real?): $fp" >>"$log"
      ok=0
    fi
    for bit in get=1 after=1 restart=1 remove=1 napply=1; do
      grep -q " $bit" <<<" $fp" || ok=0
    done
    if [[ $ok == 1 ]]; then
      echo "$seed $attempt $dur $fp"
      return 0
    fi
    echo "attempt $attempt failed after ${dur}s (bits missing in: $fp)" >>"$log"
  done
  echo "$seed 3 $dur NO-FINGERPRINT"
  return 1
}

# Reference: seed 1 solo, timed (validates the harness pre-fanout).
t0=$(date +%s)
run_seed 1 >"$LOGS/.row_1" || {
  echo "error: reference solo run failed (see $LOGS/seed_1.txt)" >&2
  rm -f "$LOGS/.row_1"
  exit 1
}
solo=$(( $(date +%s) - t0 ))
echo "solo reference: ${solo}s"

# K parallel clusters.
t0=$(date +%s)
for i in $(seq 2 "$K"); do
  run_seed "$i" >"$LOGS/.row_$i" 2>&1 &
done
fail=0
wait
wall=$(( $(date +%s) - t0 ))
factor=$(awk "BEGIN{printf \"%.2f\", $wall / ($solo + 0.001)}")

# Aggregate + register.
{
  echo "# $TITLE"
  echo
  echo "- date: $(date -u +%Y-%m-%dT%H:%M:%SZ)  host: $(uname -sm)  K=$K"
  echo "- port ranges: 26000..$((26000 + 3 * K - 1)) (L28_BASE_PORT seam); retries ≤3/seed"
  echo "- wall K-paralelo: ${wall}s vs 1 seed solo: ${solo}s (fator $factor)"
  echo
  echo "| seed | attempts | s | fingerprint |"
  echo "|------|----------|---|-------------|"
} >"$OUT/README.md"
clean=0
all_targets=""
for i in $(seq 1 "$K"); do
  row="$(grep '^0x' "$LOGS/.row_$i" 2>/dev/null || echo "$(seed_of "$i") - - MISSING")"
  rm -f "$LOGS/.row_$i"
  read -r s a d rest <<<"$row"
  echo "| $s | $a | $d | $rest |" >>"$OUT/README.md"
  if [[ "$rest" == "seed="* ]]; then
    clean=$((clean + 1))
  else
    fail=$((fail + 1))
  fi
  tgt="$(grep -oE 'kill=(leader|node)[0-9]+' <<<"$rest" | head -1)"
  [[ -n "$tgt" ]] && all_targets="${all_targets}${tgt}"$'\n'
  echo "$row"
done
target_summary="$(sort <<<"$all_targets" | grep -E 'kill=' | uniq -c | awk '{printf "%s×%s " , $2, $1}')"
distinct_targets="$(sort -u <<<"$all_targets" | grep -cE 'kill=' || true)"
{
  echo
  echo "## Piso"
  echo "- K seeds com get/after/restart/remove/napply=1 é EVIDÊNCIA, não ∀ TCP (R-swarm-real segue no residual)."
  echo "- retry ≤3 é harness; liveness continua não admitida (R-es)."
  echo "- kill targets: ${target_summary:-none} (distinct=${distinct_targets:-0}) — uma seed dirige só o alvo do kill (seed % 3), KV e cluster-id; RNG/timing dos nós não são seed-driven."
} >>"$OUT/README.md"

echo
echo "campaign: $clean/$K seeds clean, wall=${wall}s solo=${solo}s factor=$factor"
echo "campaign: kill targets ${target_summary:-none} (distinct=${distinct_targets:-0})"
if [[ "$fail" -gt 0 ]]; then
  echo "campaign: FAILED ($fail seeds without a clean fingerprint)"
  exit 1
fi
if [[ "$K" -ge 2 && "${distinct_targets:-0}" -lt 2 ]]; then
  echo "campaign: FAILED (KILL-TARGET COLLAPSE: every seed killed the same node — the diversity claim would be void)"
  exit 1
fi
awk -v f="$factor" 'BEGIN { exit !(f < 2.0) }' || {
  echo "campaign: FAILED (parallel factor $factor >= 2x — not actually parallel)"
  exit 1
}
echo "campaign: OK"
