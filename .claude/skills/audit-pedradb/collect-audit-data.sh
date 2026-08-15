#!/usr/bin/env bash
# Collect audit data for PedraDB / Montanha crates in one pass.
# Output is structured for parsing by the audit-pedradb skill.

set -euo pipefail

# Default: all workspace crates. Override with paths as args.
if [[ $# -gt 0 ]]; then
  TARGET_DIRS=("$@")
else
  TARGET_DIRS=(crates)
fi

echo "=== PEDRADB AUDIT DATA ==="
echo "TARGET_DIRS: ${TARGET_DIRS[*]}"
echo "cwd: $(pwd)"
echo ""

search() {
  local name="$1"
  shift
  echo "--- $name ---"
  rg "$@" "${TARGET_DIRS[@]}" 2>/dev/null || true
  echo ""
}

# ---------------------------------------------------------------------------
# CRITICAL: durability & integrity contracts
# ---------------------------------------------------------------------------
echo "=== CRITICAL: DURABILITY CONTRACT ==="
search "open_options_sync" 'sync\s*:\s*(true|false)|OpenOptions\s*\{' --type rust -n -C 2
search "sync_data_calls" 'sync_data\(' --type rust -n
search "sync_all_calls" 'sync_all\(' --type rust -n
search "sync_dir_calls" 'sync_dir\(' --type rust -n
search "fsync_mentions" 'fsync|fdatasync' --type rust -n -i
search "write_options" 'WriteOptions|sync_wal|disable_wal' --type rust -n -C 2

echo "=== CRITICAL: INTEGRITY / CHECKSUMS ==="
search "crc_usage" 'crc32c|checksum|Crc|CRC' --type rust -n
search "verify_checksums" 'verify_checksums|ChecksumMismatch|Corrupt' --type rust -n -C 2
search "length_in_checksum" 'length.*checksum|checksum.*length|masked' --type rust -n -i

echo "=== CRITICAL: ENV / HOST SEAM BYPASS ==="
# Engine should talk to disk via Env; wall time / entropy via Clock / Rng.
search "std_fs_in_core" 'std::fs::' --type rust -n crates/pedradb-core
search "std_fs_workspace" 'std::fs::' --type rust -n
search "file_open_direct" 'OpenOptions::new|File::(open|create|create_new)' --type rust -n
search "system_time" 'SystemTime|Instant::now|std::time::SystemTime' --type rust -n
search "thread_rng" 'thread_rng|rand::random|OsRng' --type rust -n
search "thread_sleep" 'thread::sleep|std::thread::sleep' --type rust -n

echo "=== CRITICAL: UNSAFE / FORBID ==="
search "forbid_unsafe" 'forbid\(unsafe_code\)' --type rust -n
search "unsafe_blocks" 'unsafe\s*\{|unsafe fn|unsafe impl' --type rust -n
search "allow_unsafe" 'allow\(unsafe' --type rust -n

# ---------------------------------------------------------------------------
# CRITICAL / HIGH: recovery, TX, WAL/SST/MANIFEST
# ---------------------------------------------------------------------------
echo "=== HIGH: RECOVERY / TORN WRITE ==="
search "wal_recover" 'recover|torn|orphan|Middle|Last|First' --type rust -n crates/pedradb-core/src/wal
search "manifest_paths" 'CURRENT|MANIFEST|VersionSet' --type rust -n crates/pedradb-core
search "tmp_rename" 'rename\(|tmp|TEMPORARY|\.tmp' --type rust -n crates/pedradb-core

echo "=== HIGH: TRANSACTION ATOMICITY ==="
search "tx_commit" 'fn commit|\.commit\(' --type rust -n crates/pedradb-core
search "write_record" 'WriteRecord|WriteOp' --type rust -n crates/pedradb-core
search "begin_tx" 'fn begin|\.begin\(' --type rust -n

echo "=== HIGH: CONCURRENCY / LOCKS ==="
search "mutex_fields" ':\s*(Arc<)?(std::sync::|parking_lot::|tokio::sync::)?(Mutex|RwLock)<' --type rust -n
search "lock_acq" '\.lock\(|\.read\(|\.write\(|\.try_lock\(' --type rust -n
search "concurrent_db" 'ConcurrentDb' --type rust -n
search "std_sync_mutex" 'std::sync::Mutex|std::sync::RwLock' --type rust -n
search "parking_lot" 'parking_lot::' --type rust -n

echo "=== DETAILED LOCK CONTEXT ==="
search "lock_acq_full" '\.lock\(\)|\.read\(\)|\.write\(\)' --type rust -n -A 15
echo "--- lock_containing_structs ---"
rg -U 'struct\s+\w+[^{]*\{[^}]*(Mutex|RwLock)<' --type rust -n "${TARGET_DIRS[@]}" 2>/dev/null || true
echo ""

# ---------------------------------------------------------------------------
# HIGH: silent wrong / fail-closed / panics
# ---------------------------------------------------------------------------
echo "=== HIGH: FAIL-CLOSED VS SILENT WRONG ==="
search "unwrap_non_test" '\.unwrap\(\)|\.expect\(' --type rust -n --glob '!**/*test*'
search "unwrap_all" '\.unwrap\(\)|\.expect\(' --type rust -n
search "panic_unreachable" 'panic!|unreachable!|todo!|unimplemented!' --type rust -n
search "default_on_error" 'unwrap_or_default\(\)|\.ok\(\)\s*;' --type rust -n -C 2

echo "=== HIGH: SWALLOWED ERRORS ==="
search "let_underscore_all" 'let\s+_\s*=' --type rust -n -C 3
search "let_underscore_named" 'let\s+_[a-z_]+\s*=' --type rust -n -C 3
search "if_let_err" 'if let Err\(' --type rust -n -C 5
search "match_err_empty" 'Err\(_\)\s*=>\s*\{\s*\}' --type rust -n -C 3
search "drop_discard" 'drop\(' --type rust -n -C 2

# ---------------------------------------------------------------------------
# HIGH: layer boundaries
# ---------------------------------------------------------------------------
echo "=== HIGH: LAYER BOUNDARIES ==="
echo "--- core_must_not_import_upper ---"
rg 'use pedradb_(raft|store|dcs|http|sql|stream|replicate|apply|dst)' --type rust -n crates/pedradb-core 2>/dev/null || true
echo ""
echo "--- store_imports ---"
rg 'use pedradb_' --type rust -n crates/pedradb-store/src 2>/dev/null || true
echo ""
echo "--- raft_imports ---"
rg 'use pedradb_' --type rust -n crates/pedradb-raft/src 2>/dev/null || true
echo ""
echo "--- dcs_imports ---"
rg 'use pedradb_' --type rust -n crates/pedradb-dcs/src 2>/dev/null || true
echo ""

# ---------------------------------------------------------------------------
# HIGH: DST seams (determinism)
# ---------------------------------------------------------------------------
echo "=== HIGH: DST / DETERMINISM SEAMS ==="
search "env_trait" 'trait Env|open_with_env|open_with_host|FailingEnv|RecordingEnv|DetHost' --type rust -n
search "clock_trait" 'trait Clock|ManualClock|SystemClock|open_with_clock' --type rust -n
search "rng_trait" 'trait Rng|SeedRng|SystemRng' --type rust -n
search "rpc_mode" 'RpcMode|Queued|Direct|PeerMsg|drain_outbound|handle_inbound' --type rust -n
search "wall_sleep_in_prod" 'thread::sleep' --type rust -n --glob '!**/*test*'

# ---------------------------------------------------------------------------
# HIGH: Montanha-Store / majority / leadership
# ---------------------------------------------------------------------------
echo "=== HIGH: MONTANHA STORE INVARIANTS (surface) ==="
search "majority" 'majority|NotCommitted|LocalApplied|Strong' --type rust -n crates/pedradb-store
search "dual_leader" 'dual.?leader|Role::Leader|range_leader' --type rust -n crates/pedradb-store
search "failover" 'failover|leader_loss|InstallSnapshot' --type rust -n crates/pedradb-store

# ---------------------------------------------------------------------------
# MEDIUM: bounds / OOM / scalability
# ---------------------------------------------------------------------------
echo "=== MEDIUM: BOUNDS / OOM RISK ==="
search "unbounded_range" 'fn range\(|range_limited|visible_range' --type rust -n crates/pedradb-core
search "collect_vec" '\.collect::<Vec|\.collect\(\)' --type rust -n crates/pedradb-core/src
search "vec_new_unbounded" 'Vec::new\(\)|VecDeque::new\(\)' --type rust -n crates/pedradb-core/src
search "with_capacity" 'with_capacity|MAX_|limit|LIMIT|max_' --type rust -n crates/pedradb-core/src

echo "=== MEDIUM: PUBLIC API SURFACE ==="
search "pub_fn_core" 'pub fn ' --type rust -n crates/pedradb-core/src/db.rs crates/pedradb-core/src/tx.rs crates/pedradb-core/src/lib.rs
search "error_enums" 'enum \w*Error' --type rust -n -A 25

# ---------------------------------------------------------------------------
# MEDIUM: dependency / CI supply chain (workspace-wide)
# ---------------------------------------------------------------------------
echo "=== MEDIUM: SUPPLY CHAIN / CI ==="
echo "--- deny_audit_config ---"
find . \( -iname "deny.toml" -o -iname "audit.toml" \) -not -path "*/target/*" 2>/dev/null || true
echo ""
echo "--- cargo_audit_ci ---"
rg -l 'cargo audit|cargo-audit|cargo deny|cargo-deny' .github/workflows 2>/dev/null || true
ls .github/workflows 2>/dev/null || echo "(no .github/workflows)"
echo ""
echo "--- forbid_unsafe_per_crate ---"
rg 'forbid\(unsafe_code\)' --type rust -n crates 2>/dev/null || true
echo ""

# ---------------------------------------------------------------------------
# Docs / invariant map presence (reference only)
# ---------------------------------------------------------------------------
echo "=== REFERENCE DOCS (presence) ==="
for f in \
  docs/dst-seams.md \
  docs/montanha-invariants-and-tests.md \
  docs/robustness-vs-rocks-pebble-fdb.md \
  docs/open-items.md \
  docs/rfc/0014-rocks-pebble-redwood-maturity.md
do
  if [[ -f "$f" ]]; then
    echo "OK  $f"
  else
    echo "MISS $f"
  fi
done
echo ""

echo "=== DONE ==="
