#!/usr/bin/env bash
set -euo pipefail

backup_dir="${BACKUP_DIR:-./backups}"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
backup_path="${backup_dir}/open_sentry_${timestamp}.dump"
compose_file="${COMPOSE_FILE:-docker-compose.production.yml}"
mkdir -p "$backup_dir"

docker compose -f "$compose_file" exec -T postgres \
  pg_dump --username sentry --dbname open_sentry --format custom --compress 9 \
  > "$backup_path"

test -s "$backup_path"
sha256sum "$backup_path" > "${backup_path}.sha256"
printf 'backup=%s\nchecksum=%s\n' "$backup_path" "${backup_path}.sha256"
