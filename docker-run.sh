#!/usr/bin/env bash
# Run the perbot container with ./data and ./logs bind-mounted from the host.
# Host /tmp is mounted too so the /tmp/perbot.lock instance lock is host-wide.
# Requires TG_BOT_TOKEN and TG_ADMIN_ID in the environment (or in ./perbot.env).
# Restart policy is on-failure: crashes restart, a clean exit (/exit yes) stays
# down. Boot-time start comes from the cron @reboot entry installed by spot.yml.
set -euo pipefail
cd "$(dirname "$0")"

# Pick up TG_BOT_TOKEN / TG_ADMIN_ID / TZ / RUST_LOG from perbot.env if present
if [ -f perbot.env ]; then
    set -a
    . ./perbot.env
    set +a
fi

: "${TG_BOT_TOKEN:?TG_BOT_TOKEN is required}"
: "${TG_ADMIN_ID:?TG_ADMIN_ID is required}"

# When launched from cron @reboot the docker daemon may not be up yet
for _ in $(seq 1 30); do
    docker info >/dev/null 2>&1 && break
    sleep 2
done

IMAGE="${IMAGE:-perbot}"
NAME="${NAME:-perbot}"
DATA_DIR="$PWD/data"   # perbot.db lives here (bot opens data/perbot.db from WORKDIR /)
LOGS_DIR="$PWD/logs"

mkdir -p "$DATA_DIR" "$LOGS_DIR"

docker rm -f "$NAME" >/dev/null 2>&1 || true

# --user matches the host owner of the bind mounts so the bot can write to them
docker run -d \
    --name "$NAME" \
    --restart on-failure \
    --user "$(id -u):$(id -g)" \
    -v "$DATA_DIR":/data \
    -v "$LOGS_DIR":/logs \
    -v /tmp:/tmp \
    -e LOG_DIR=/logs \
    -e TG_BOT_TOKEN \
    -e TG_ADMIN_ID \
    -e RUST_LOG="${RUST_LOG:-info}" \
    "$IMAGE"

echo "Started container '$NAME' (db: $DATA_DIR/perbot.db, logs: $LOGS_DIR)"
