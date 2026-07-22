# Open Sentry Rust

[中文文档](README.zh-CN.md) · **English**

> A production-oriented, self-hosted, Sentry-compatible error monitoring platform built with Rust.

`Open Sentry Rust` accepts Sentry SDK events and Envelopes, groups errors into
actionable issues, and provides a web console for investigating events, logs,
releases, symbols, alerting, and on-call escalation.

**Tags:** `rust` · `axum` · `postgresql` · `sentry` · `error-monitoring` ·
`observability` · `self-hosted` · `webhooks` · `prometheus` · `docker`

## Highlights

- **Sentry-compatible ingestion** — JSON store endpoint and Envelope ingestion
  for modern Sentry SDKs, including `identity`, `gzip`, and `deflate/zlib`.
- **Incident workflow** — issue grouping, status changes, comments, merge/split,
  regression detection, custom fingerprints, and server-side search.
- **Production-ready controls** — organization isolation, sessions, RBAC,
  audit log, rate limiting, payload limits, retention jobs, and Prometheus metrics.
- **Alerting and escalation** — email, Telegram, Twilio Voice, signed webhooks,
  on-call schedules, retries, recovery notifications, and escalation policies.
- **Release and symbol support** — release health, deployments, JavaScript source
  maps, ELF/Mach-O symbols, validated PDB uploads, and event reprocessing.
- **Web console** — projects, services, structured logs, events, issues,
  releases, symbols, alert delivery history, and escalation management.

## Architecture

The project is a modular monolith: the API and notification worker run in one
process, while PostgreSQL provides durable storage and the reliable outbox.
That keeps the first deployment simple and preserves a clear path to separate
workers, Kafka/NATS ingestion, and ClickHouse event storage as traffic grows.

```text
SDK / curl -> Axum ingest API -> PostgreSQL
                                  |-- events (raw event data)
                                  |-- issues (grouped incidents)
                                  |-- releases / debug_files -> Symbol worker
                                  `-- notification_outbox -> Worker -> Email / Telegram / Voice / Webhook

Web UI -> Query API -> PostgreSQL
```

Events, issue aggregation, and alert-outbox records are written in one database
transaction. Notification failures retry with exponential backoff without
blocking event ingestion.

## Quick start

```bash
cp .env.example .env
docker compose up -d
set -a && source .env && set +a
cargo run
```

The web console uses database-backed sessions. On the first start, create an
administrator through environment variables:

```text
APP__BOOTSTRAP_ADMIN_EMAIL=admin@example.com
APP__BOOTSTRAP_ADMIN_PASSWORD=change-me
```

Use `change-me` only for local development. In production, inject a unique
strong password through a secrets manager. Passwords use Argon2 hashes and
session tokens are stored only as SHA-256 digests.

The default demo project UUID is `00000000-0000-0000-0000-000000000001`; its
Sentry-compatible numeric project ID is `1`. The local development DSN is:

```text
http://dev-secret@localhost:8080/1
```

Send an event:

```bash
curl -X POST http://localhost:8080/api/projects/00000000-0000-0000-0000-000000000001/store \
  -H 'content-type: application/json' \
  -H 'x-sentry-auth: dev-secret' \
  -d '{"level":"error","message":"payment timeout","environment":"production","tags":{"service":"checkout"}}'
```

For modern Sentry SDKs, send Envelopes to:

```text
POST /api/1/envelope/
X-Sentry-Auth: Sentry sentry_version=7, sentry_key=dev-secret
Content-Type: application/x-sentry-envelope
```

Run the SDK smoke client (built with `sentry 0.48.5`):

```bash
cargo test --bin sentry-smoke -- --test-threads=1
cargo run --bin sentry-smoke -- all
```

`all` sends a message, standard error, custom event, attachment, transaction,
structured logs, and captured panic. Override its default DSN with `SENTRY_DSN`.

## API overview

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/health` | Liveness and database check |
| `GET` | `/metrics` | Bearer-protected Prometheus metrics |
| `POST` | `/api/auth/login` | Create an HttpOnly console session |
| `POST` | `/api/{numeric_id}/envelope/` | Sentry Envelope ingestion |
| `POST` | `/api/projects/{id}/store` | JSON event ingestion |
| `GET/PATCH` | `/api/issues/{id}` | Read or update an issue |
| `GET/POST` | `/api/projects/{id}/alert-rules` | Manage alert rules |
| `GET/POST` | `/api/projects/{id}/releases` | Manage releases and health |
| `GET/POST` | `/api/projects/{id}/debug-files` | Manage source maps and symbols |
| `GET/POST` | `/api/projects/{id}/webhooks` | Manage signed webhooks |
| `GET/POST` | `/api/projects/{id}/on-call-schedules` | Manage on-call rotations |
| `GET/POST` | `/api/projects/{id}/escalation-policies` | Manage multi-stage escalation |

For the full API surface and design boundaries, see
[architecture notes](docs/architecture.md). Release gates, capacity thresholds,
backup, and recovery procedures are in
[production acceptance](docs/production-acceptance.md).

## Production deployment

```bash
cp .env.production.example .env.production
docker compose --env-file .env.production -f docker-compose.production.yml up -d --build
scripts/production-acceptance.sh
```

Production images run as non-root users. Database, backend, and frontend health
checks control the startup order.

## Web console

```bash
cd frontend
cp .env.example .env.local
npm install
npm run dev
```

Open `http://localhost:3000`; it connects to `http://127.0.0.1:8080` by default.

## Technology

- Backend: Rust, Axum, Tokio, SQLx, PostgreSQL
- Frontend: Next.js, React, TypeScript
- Operations: Docker Compose, Prometheus-compatible metrics

## Attribution

This project was developed with **OpenAI ChatGPT 5.6**. Approximate AI development
cost: **US$20**.

## Chinese documentation

For a Chinese overview, setup instructions, API list, and production notes, read
[README.zh-CN.md](README.zh-CN.md).
