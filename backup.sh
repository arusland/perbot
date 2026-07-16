#!/usr/bin/env bash
set -euo pipefail

SRC="/home/perbot"
DEST_DIR="/home/backup/dropbox/perbot"
STAMP="$(date +%Y%m%d_%H%M%S)"
TMP_DIR="/tmp/perbot_${STAMP}"
ARCHIVE="${TMP_DIR}.tar.gz"

cp -a "$SRC" "$TMP_DIR"
tar -czf "$ARCHIVE" -C /tmp "$(basename "$TMP_DIR")"
rm -rf "$TMP_DIR"

mkdir -p "$DEST_DIR"
cp "$ARCHIVE" "$DEST_DIR/"
rm -f "$ARCHIVE"

echo "Backup complete: ${DEST_DIR}/$(basename "$ARCHIVE")"
