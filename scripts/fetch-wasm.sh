#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
OUT="${1:-$ROOT/dist}"

TAG="${2:-${LIGHT_CLIENT_VERSION:-}}"
REPO="${3:-${LIGHT_CLIENT_REPOSITORY:-}}"
ARTIFACT="stellar_light_client.wasm"

if [ -z "$REPO" ] || [ -z "$TAG" ]; then
  echo "error: set LIGHT_CLIENT_REPOSITORY (owner/name) and LIGHT_CLIENT_VERSION (a release tag)," >&2
  echo "       or pass them: fetch-wasm.sh <out-dir> <tag> <owner/name>." >&2
  exit 1
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "error: gh not found. The light client is released from $REPO;" >&2
  echo "       install the GitHub CLI or set LIGHT_CLIENT_WASM_DIR to a local build." >&2
  exit 1
fi

mkdir -p "$OUT"

gh release download "$TAG" \
  --repo "$REPO" \
  --pattern "$ARTIFACT" \
  --pattern "$ARTIFACT.gz" \
  --pattern "checksums.txt" \
  --dir "$OUT" \
  --clobber

cd "$OUT"

if ! shasum -a 256 -c checksums.txt; then
  echo "error: downloaded artifact does not match its published checksums." >&2
  echo "       Do not submit this blob. Report it against $REPO@$TAG." >&2
  exit 1
fi

echo
echo "$REPO@$TAG verified in $OUT"
cat checksums.txt
echo
echo "on-chain checksum is the uncompressed one:"
echo "  gunzip -c $ARTIFACT.gz | shasum -a 256"
