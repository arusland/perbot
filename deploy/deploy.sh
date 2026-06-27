#!/usr/bin/env bash
# Usage: deploy/deploy.sh <dev|prod>
set -euo pipefail
target="${1:?usage: deploy.sh <dev|prod>}"
spot -p deploy/spot.yml -t "$target" -n deploy
