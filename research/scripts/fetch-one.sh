#!/usr/bin/env bash
# Fetch one open PDF into research/fontes/.
# Usage: ./research/scripts/fetch-one.sh R010 URL [Autor_Ano_Slug]
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: $0 Rxxx URL [Autor_Ano_Slug]" >&2
  exit 2
fi

id="$1"
url="$2"
slug="${3:-}"

if [[ ! "$id" =~ ^R[0-9]{3}$ ]]; then
  echo "id must look like R010" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/../.." && pwd)"
dest_dir="$root/research/fontes"
mkdir -p "$dest_dir"

if [[ -z "$slug" ]]; then
  slug="$(basename "${url%%\?*}")"
  slug="${slug%.pdf}"
  slug="$(echo "$slug" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//')"
  if [[ -z "$slug" ]]; then
    slug="paper"
  fi
fi

dest="$dest_dir/${id}_${slug}.pdf"
if [[ -e "$dest" ]]; then
  echo "refusing to overwrite $dest" >&2
  exit 1
fi

tmp="$(mktemp)"
cleanup() { rm -f "$tmp"; }
trap cleanup EXIT

curl -fsSL --retry 3 --retry-delay 1 -A "pedradb-research/0.1 (local library)" -o "$tmp" "$url"

if ! dd if="$tmp" bs=5 count=1 2>/dev/null | grep -q '%PDF-'; then
  echo "download does not look like a PDF (paywall HTML?)" >&2
  exit 1
fi

mv "$tmp" "$dest"
trap - EXIT
echo "wrote $dest"
echo "next: catalog.tsv status=have-pdf local=research/fontes/${id}_${slug}.pdf"
if command -v pdftotext >/dev/null 2>&1; then
  pdftotext -layout "$dest" "${dest%.pdf}.txt" && echo "wrote ${dest%.pdf}.txt"
fi
