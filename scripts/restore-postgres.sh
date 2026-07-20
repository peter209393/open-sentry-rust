#!/usr/bin/env bash
set -euo pipefail

backup_path="${1:?usage: restore-postgres.sh BACKUP.dump}"
restore_database="${RESTORE_DATABASE:-open_sentry_restore}"
compose_file="${COMPOSE_FILE:-docker-compose.production.yml}"

if [[ "${CONFIRM_RESTORE:-}" != "$restore_database" ]]; then
  printf 'Refusing restore. Set CONFIRM_RESTORE=%s to restore into that database.\n' "$restore_database" >&2
  exit 2
fi
if [[ ! "$restore_database" =~ ^[a-zA-Z0-9_]+$ ]]; then
  printf 'RESTORE_DATABASE contains invalid characters.\n' >&2
  exit 2
fi

test -s "$backup_path"
if [[ -f "${backup_path}.sha256" ]]; then sha256sum --check "${backup_path}.sha256"; fi
docker compose -f "$compose_file" exec -T postgres pg_restore --list < "$backup_path" >/dev/null

docker compose -f "$compose_file" exec -T postgres \
  psql --username sentry --dbname postgres --set ON_ERROR_STOP=1 \
  --command "DROP DATABASE IF EXISTS \"${restore_database}\" WITH (FORCE)"
docker compose -f "$compose_file" exec -T postgres \
  psql --username sentry --dbname postgres --set ON_ERROR_STOP=1 \
  --command "CREATE DATABASE \"${restore_database}\" OWNER sentry"
docker compose -f "$compose_file" exec -T postgres \
  pg_restore --username sentry --dbname "$restore_database" --exit-on-error < "$backup_path"

docker compose -f "$compose_file" exec -T postgres \
  psql --username sentry --dbname "$restore_database" --tuples-only --command \
  "SELECT 'projects=' || count(*) FROM projects UNION ALL SELECT 'events=' || count(*) FROM events UNION ALL SELECT 'users=' || count(*) FROM users"
