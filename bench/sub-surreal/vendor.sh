#!/bin/sh
# Regenerate the merged vendored crates.io tree for the sub-surreal bench
# (RFC-0059). Makes both sides build with `--offline` — the caixote bench VMs
# sit behind a resolver that intermittently cannot resolve index.crates.io,
# so builds must not touch the network.
#
# Usage: from anywhere, run `sh bench/sub-surreal/vendor.sh` with the repo
# root as CWD. Produces <repo>/vendor plus the two .cargo/config.toml
# source-replacement files (path-relative, so the tree is relocatable).
set -eu

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VENDOR="$ROOT/vendor"

echo "== vendoring pedra side =="
(cd "$ROOT/bench/sub-surreal/pedra" && cargo vendor "$VENDOR" > /dev/null)

echo "== vendoring peer side (merge) =="
(cd "$ROOT/bench/sub-surreal/peer" && cargo vendor "$VENDOR" > /dev/null)

for side in pedra peer; do
  mkdir -p "$ROOT/bench/sub-surreal/$side/.cargo"
  cat > "$ROOT/bench/sub-surreal/$side/.cargo/config.toml" <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "../../../vendor"
EOF
done

echo "== done: $VENDOR (cargo build --release --offline in either side) =="
