#!/usr/bin/env bash
# Build / push / deploy montanha-tcp on caixote (real Linux).
#
# Usage:
#   ./scripts/montanha_tcp_caixote.sh local3      # 3 processes one VM (default lab)
#   ./scripts/montanha_tcp_caixote.sh mesh        # 3 services + set-peers + elect + smoke
#   ./scripts/montanha_tcp_caixote.sh wire        # only wire live mesh (no rebuild)
#   ./scripts/montanha_tcp_caixote.sh build|push
#
# Env:
#   TAG / IMAGE              base image for local3 (default p01-health2)
#   IMAGE_BASE               mesh per-node prefix …:m12-n → n1/n2/n3 (default m12-n)
#   MONTANHA_REV             service suffix (default m13; bump to recreate)
#   HOST_SSH                 platform host for set-peers (default paulo@192.168.68.109)
#   PLATFORM                 docker --platform (default linux/amd64)
#   ORG                      caixote org (optional)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TAG="${TAG:-p01-health2}"
IMAGE="${IMAGE:-ghcr.io/paulocsanz/montanha-tcp:${TAG}}"
IMAGE_BASE="${IMAGE_BASE:-${MONTANHA_TCP_IMAGE_BASE:-ghcr.io/paulocsanz/montanha-tcp:m12-n}}"
REV="${MONTANHA_REV:-m13}"
PLATFORM="${PLATFORM:-linux/amd64}"
PROJECT="${PROJECT:-pedradb-montanha}"
HOST_SSH="${HOST_SSH:-paulo@192.168.68.109}"
cmd="${1:-local3}"

build_base() {
  echo "==> docker build --platform $PLATFORM $IMAGE"
  docker build --platform "$PLATFORM" -f Dockerfile.montanha-tcp -t "$IMAGE" .
}

push_base() {
  echo "==> docker push $IMAGE"
  docker push "$IMAGE"
}

# Per-node images: bake CLUSTER/NODE_ID/membership (env inject race on caixote).
build_mesh_nodes() {
  build_base
  push_base
  local peers='1=127.0.0.1:9701 2=127.0.0.1:9701 3=127.0.0.1:9701'
  for id in 1 2 3; do
    local tag="${IMAGE_BASE}${id}"
    local df
    df=$(mktemp)
    cat >"$df" <<EOF
FROM ${IMAGE}
ENV CLUSTER=single
ENV NODE_ID=${id}
ENV BIND=0.0.0.0:9701
ENV HEALTH_BIND=0.0.0.0:9780
ENV RANGES=1
ENV DATA_DIR=/data
ENV PEERS="${peers}"
EOF
    echo "==> build $tag"
    docker build --platform "$PLATFORM" -t "$tag" -f "$df" "$ROOT" >/dev/null
    rm -f "$df"
    echo "==> push $tag"
    docker push "$tag"
  done
}

deploy_local3() {
  echo "==> iac apply MONTANHA_MODE=local3 image=$IMAGE"
  export MONTANHA_MODE=local3
  export MONTANHA_TCP_IMAGE="$IMAGE"
  export MONTANHA_REV="${REV}"
  local args=(iac apply --auto-approve -f caixote.config.ts)
  [[ -n "${ORG:-}" ]] && args+=(--org "$ORG")
  caixote "${args[@]}"
}

# Bake RoleAware proxy image with CLUSTER=proxy + MEMBERS DNS (env inject race).
build_proxy_image() {
  local tag_suffix="${MONTANHA_PROJECT_TAG_SUFFIX:-c8d3f0c9}"
  local proxy_tag="${MONTANHA_PROXY_IMAGE:-ghcr.io/paulocsanz/montanha-tcp:${REV}-proxy}"
  local members
  members=$(for id in 1 2 3; do
    echo -n "mtcp-${id}-${REV}-${tag_suffix}.internal:9701 "
  done | sed 's/ $//')
  local df
  df=$(mktemp)
  cat >"$df" <<EOF
FROM ${IMAGE}
ENV CLUSTER=proxy
ENV PROXY_LISTEN=0.0.0.0:9600
ENV PROXY_MODE=write
ENV MEMBERS=${members}
EOF
  echo "==> build proxy $proxy_tag MEMBERS=$members"
  docker build --platform "$PLATFORM" -t "$proxy_tag" -f "$df" "$ROOT" >/dev/null
  rm -f "$df"
  echo "==> push $proxy_tag"
  docker push "$proxy_tag"
  export MONTANHA_PROXY_IMAGE="$proxy_tag"
}

deploy_mesh() {
  echo "==> iac apply MONTANHA_MODE=mesh IMAGE_BASE=$IMAGE_BASE REV=$REV"
  export MONTANHA_MODE=mesh
  export MONTANHA_TCP_IMAGE_BASE="$IMAGE_BASE"
  export MONTANHA_REV="$REV"
  if [[ "${MONTANHA_MESH_PROXY:-0}" == "1" ]]; then
    export MONTANHA_MESH_PROXY=1
  fi
  local args=(iac apply --auto-approve --confirm-destructive -f caixote.config.ts)
  [[ -n "${ORG:-}" ]] && args+=(--org "$ORG")
  caixote "${args[@]}"
}

linux_bin() {
  # Host is x86_64; extract from image if no local amd64 binary.
  if [[ -x target/release/montanha-tcp ]] && file target/release/montanha-tcp | grep -q x86-64; then
    echo "$ROOT/target/release/montanha-tcp"
    return
  fi
  local out=/tmp/montanha-tcp-linux
  local cname="mtcp-extract-$$"
  docker create --name "$cname" --platform "$PLATFORM" "$IMAGE" true >/dev/null
  docker cp "$cname:/usr/local/bin/montanha-tcp" "$out"
  docker rm "$cname" >/dev/null
  chmod +x "$out"
  echo "$out"
}

wire_mesh() {
  local bin
  bin=$(linux_bin)
  echo "==> wire raft HOST_SSH=$HOST_SSH bin=$bin"
  MONTANHA_MESH_REV="$REV" LOCAL_BIN="$bin" HOST_SSH="$HOST_SSH" \
    "$ROOT/scripts/montanha_tcp_mesh_wire.sh" raft
}

case "$cmd" in
  build) build_base ;;
  push) push_base ;;
  build-mesh) build_mesh_nodes ;;
  deploy) deploy_local3 ;;
  local3)
    build_base
    push_base
    deploy_local3
    ;;
  mesh)
    build_mesh_nodes
    if [[ "${MONTANHA_MESH_PROXY:-0}" == "1" ]]; then
      build_proxy_image
    fi
    deploy_mesh
    echo "==> wait ~40s for boot/WG"
    sleep 40
    wire_mesh
    if [[ "${MONTANHA_MESH_PROXY:-0}" == "1" ]]; then
      MONTANHA_MESH_REV="$REV" LOCAL_BIN="$(linux_bin)" \
        "$ROOT/scripts/montanha_proxy_field.sh" || true
    fi
    ;;
  wire)
    wire_mesh
    ;;
  local-docker)
    build_base
    docker run --rm --platform "$PLATFORM" -e CLUSTER=smoke "$IMAGE"
    ;;
  *)
    echo "usage: $0 build|push|build-mesh|deploy|local3|mesh|wire|local-docker" >&2
    exit 2
    ;;
esac
