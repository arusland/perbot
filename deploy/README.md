# Deploying Perbot with Spot

Reproducible deploys to a **dev** and a **prod** server over SSH using
[umputun/spot](https://github.com/umputun/spot).

The binary is built **locally** (`cargo build --release`, native glibc) and copied to the
server, where it runs as a `systemd` service from `/opt/perbot`.

## Prerequisites

- [Spot](https://github.com/umputun/spot) installed on your machine
  (`go install github.com/umputun/spot/cmd/spot@latest`, or grab a release binary).
- SSH access to the hosts via key/agent (the `user` and `ssh_key` in `spot.yml`).
- The SSH user has **passwordless sudo** (or add `sudo_password`/`secrets` to the privileged
  commands in `spot.yml`).
- Servers are **x86_64 Linux with glibc ≥ your build machine's** (native-glibc copy strategy).
  If they diverge, switch to a static-musl or build-on-server strategy.

## First, edit `spot.yml`

Replace the placeholders: `DEV_HOST`, `PROD_HOST`, the SSH `user`, and the `ssh_key` path.

## One-time setup (per environment)

Provisions the service user, directories, the systemd unit, and writes the per-host env file
from secrets you pass on the CLI. **Secrets are never stored in the repo.**

```bash
spot -p deploy/spot.yml -t dev  -n setup -e TG_BOT_TOKEN:<dev-token>  -e TG_ADMIN_ID:<dev-admin-id>
spot -p deploy/spot.yml -t prod -n setup -e TG_BOT_TOKEN:<prod-token> -e TG_ADMIN_ID:<prod-admin-id>
```

To rotate a token or change the admin id later, re-run the `setup` task and restart:
`spot -p deploy/spot.yml -t <env> -n deploy`.

## Routine deploy (build + ship + restart)

```bash
spot -p deploy/spot.yml -t dev  -n deploy   # or: deploy/deploy.sh dev
spot -p deploy/spot.yml -t prod -n deploy   # or: deploy/deploy.sh prod
```

Each deploy rebuilds the release binary locally, uploads it, installs it into
`/opt/perbot/perbot`, and restarts the service. `perbot.db` and `logs/` live in
`/opt/perbot` and survive redeploys (only the binary is replaced).

## On the server

- Status / logs: `systemctl status perbot`, `journalctl -u perbot -f`,
  and file logs under `/opt/perbot/logs/`.
- State: SQLite DB at `/opt/perbot/perbot.db`; env file at `/etc/perbot/perbot.env`.

## Dry run

Preview the rendered commands without touching a host:

```bash
spot --dry -p deploy/spot.yml -t dev -n deploy
```
