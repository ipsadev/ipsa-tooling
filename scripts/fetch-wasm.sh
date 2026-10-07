#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT/dist}"

REPO="${LIGHT_CLIENT_REPO:-ipsadev/stellar-light-client}"
TAG="${2:-${LIGHT_CLIENT_VERSION:-v0.1.0}}"
ARTIFACT="stellar_light_client.wasm"

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
