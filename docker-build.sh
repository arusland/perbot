#!/usr/bin/env bash
# Build the perbot Docker image (two-stage, static musl binary on Alpine).
set -euo pipefail
cd "$(dirname "$0")"

IMAGE="${IMAGE:-perbot}"

# Embed the current git revision so the bot can report which build is running
GIT_HASH="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
GIT_COMMIT_MSG="$(git log -1 --pretty=%s 2>/dev/null || echo unknown)"

docker build -t "$IMAGE" \
    --build-arg GIT_HASH="$GIT_HASH" \
    --build-arg GIT_COMMIT_MSG="$GIT_COMMIT_MSG" .
