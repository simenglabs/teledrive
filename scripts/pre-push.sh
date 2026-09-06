#!/usr/bin/env bash
# ============================================================
# Git PRE-PUSH hook pipeline for MengDrive
#
# Runs before every `git push`:
#   1. cargo check  (typecheck)
#   2. cargo test   (unit tests)
#   3. cargo build --release  -> local artifact (target/release/s3-telegram)
#   4. docker build using ONLY the prebuilt artifact
#
# Skip with: git push --no-verify
# ============================================================
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

IMAGE_NAME="${IMAGE_NAME:-simenglabs/teledrive}"
IMAGE_TAG="${IMAGE_TAG:-latest}"
BIN="target/release/s3-telegram"

echo "▸ [1/4] cargo check"
cargo check --quiet

echo "▸ [2/4] cargo test"
cargo test --quiet

echo "▸ [3/4] cargo build --release (local artifact for the image)"
cargo build --release

if [[ ! -f "$BIN" ]]; then
  echo "✗ Release binary not found at $BIN" >&2
  exit 1
fi

echo "▸ [4/4] docker build (from prebuilt binary + templates only)"
if command -v docker >/dev/null 2>&1; then
  docker build -t "$IMAGE_NAME:$IMAGE_TAG" .
  echo "✓ Image ready: $IMAGE_NAME:$IMAGE_TAG"
  echo "  Push manually with: docker push $IMAGE_NAME:$IMAGE_TAG"
  echo "  (or run: make docker-push)"
else
  echo "⚠ docker not found — skipping image build (binary is built and verified)"
fi

echo "✓ Pre-push checks passed."
