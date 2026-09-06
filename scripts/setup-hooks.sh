#!/usr/bin/env bash
# One-time setup: activate repo hooks (pre-push build gate)
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
chmod +x scripts/pre-push.sh .hooks/pre-push
git config core.hooksPath .hooks
echo "✓ Hooks activated (core.hooksPath=.hooks)"
echo "  Pre-push now runs: check → test → release build → docker build"
echo "  Skip a push with: git push --no-verify"
