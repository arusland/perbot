#!/usr/bin/env bash
# Build the perbot Docker image (two-stage, static musl binary on Alpine).
set -euo pipefail
cd "$(dirname "$0")"

IMAGE="${IMAGE:-perbot}"

docker build -t "$IMAGE" .
