# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development Commands

### Backend
```bash
# Run the application (requires PostgreSQL and Docker Compose services)
docker compose up -d
set -a && source .env && set +a
cargo run

# Run all tests
cargo test --all-targets

# Run tests with output
cargo test -- --nocapture

# Run Sentry SDK smoke tests (integration tests)
cargo test --bin sentry-smoke -- --test-threads=1
cargo run --bin sentry-smoke -- all

# Format code
cargo fmt

# Lint with Clippy
cargo clippy -- -D warnings

# Build release
cargo build --release

# Run load test benchmark
LOAD_REQUESTS=200 LOAD_CONCURRENCY=20 LOAD_MAX_P95_MS=500 \
  cargo run --release --bin ingest-load
```

### Frontend
```bash
cd frontend
cp .env.example .env.local
npm install
npm run dev        # Development server at http://localhost:3000
npm run build      # Production build
npm run lint       # ESLint
npm test           # Run tests
```

### Production
```bash
# Full production acceptance test suite
scripts/production-acceptance.sh

# Backup/restore PostgreSQL
scripts/backup-postgres.sh
scripts/restore-postgres.sh backups/open_sentry_TIMESTAMP.dump
```

## Architecture

This is a **modular monolith** Sentry-compatible error monitoring platform. The API and notification worker run in the same process, with PostgreSQL serving as both persistence and reliable outbox queue.

### Data Flow
```
SDK / curl → Axum ingest API → PostgreSQL (single transaction)
                                  ├─ events (raw events)
                                  ├─ issues (aggregated by fingerprint)
                                  ├─ notification_outbox → Worker → SMTP/Telegram/Voice
                                  └─ envelope_items (attachments, logs, other SDK items)

Web UI → Query API → PostgreSQL
```

### Key Domain Models
- **Project**: Event isolation boundary, contains DSN keys and alert rules
- **Event**: Immutable error fact with level, message, stacktrace, tags, contexts
- **Issue**: Aggregation of events by fingerprint; tracks status, counts, first/last seen
- **AlertRule**: Matches events by level/environment/message with cooldown windows
- **NotificationOutbox**: Reliable delivery queue with exponential backoff retry

### Source Structure
- `src/main.rs` - Entry point: config load, migrations, worker spawn, Axum server
- `src/api.rs` - HTTP routes (ingest, management, auth, metrics)
- `src/envelope.rs` - Sentry Envelope parsing (gzip/zlib, size limits)
- `src/model.rs` - Request/response types and SQL view types
- `src/notification.rs` - Notification worker (email, Telegram, Twilio voice calls)
- `src/alert.rs` - Alert rule matching and outbox enqueuing
- `src/auth.rs` - Session authentication, RBAC (owner/admin/member), bootstrap admin
- `src/operations.rs` - Rate limiting, retention cleanup, audit logging, metrics
- `src/telemetry.rs` - Service extraction and log batch processing from SDK envelopes
- `src/config.rs` - Configuration via `APP__*` environment variables
- `src/state.rs` - Shared AppState (DB pool, settings, HTTP client, metrics)
- `src/error.rs` - Error types with AppError variants
- `migrations/` - SQL migrations (numbered, run via sqlx)
- `src/bin/sentry-smoke.rs` - Sentry SDK integration smoke tests
- `src/bin/ingest-load.rs` - Load test benchmark

## Core Concepts

### Fingerprinting & Issue Aggregation
Events are aggregated into Issues by SHA-256 fingerprint:
1. SDK-provided fingerprint (supports `{{ default }}` placeholder) takes priority
2. Default: `level | exception_type | stable_in_app_frames` (max 5 frames)
3. Fallback: `level | exception_type | normalized_message`

This prevents dynamic values (order IDs) from fragmenting issues.

### Consistency Guarantees
All event ingest operations use a **single database transaction**:
1. Upsert Issue (with event count, timestamps, regression detection)
2. Insert Event
3. Extract/upsert Service metadata
4. Match and enqueue AlertRules to `notification_outbox`

Worker processes outbox with `FOR UPDATE SKIP LOCKED` and exponential backoff (up to 7 attempts → failed).

### Authentication & Authorization
- **Ingest API**: Project key (SHA-256 hash only stored in DB) passed via `X-Sentry-Authorization` or `Bearer`
- **Web console**: HttpOnly session cookies (token SHA-256 stored), `SameSite=Strict`
- **RBAC**: owner (can delete), admin (full management), member (read/write issues)
- All management APIs verify organization membership and project access

### Rate Limiting & Protection
- Ingest API: 600 req/min per DSN by default (`APP__INGEST_RATE_LIMIT_PER_MINUTE`)
- Returns `429` with `Retry-After` header when exceeded
- Payload limits: 5 MiB compressed, 20 MiB decompressed, 100 items, 1 MiB per event

### Configuration Pattern
All settings use `APP__` prefix with `__` for nesting:
```bash
APP__DATABASE_URL=postgres://...
APP__SMTP__HOST=smtp.example.com
APP__TELEGRAM__BOT_TOKEN=...
```
See `src/config.rs` and `.env.example` for all options.

### Sensitive Field Scrubbing
Projects can configure custom `scrub_fields` list. Default fields: `password`, `passwd`, `authorization`, `cookie`, `token`, `secret`. Recursive scrubbing runs on tags, contexts, and exception before persist.

### Retention
Background worker periodically deletes expired events/logs/envelopes based on:
- Global `APP__RETENTION_DAYS` (default 30)
- Per-project override
- Configured via `APP__RETENTION_INTERVAL_SECONDS`

## Testing Strategy

1. **Unit tests**: `cargo test` - co-located with modules using `#[cfg(test)]`
2. **SDK smoke tests**: `sentry-smoke` binary uses `sentry 0.48.5` to test:
   - Message, Error, structured Event
   - Attachment, Transaction
   - Structured Logs
   - Caught panic
3. **Load test**: `ingest-load` binary benchmarks ingest performance
4. **Production acceptance**: `scripts/production-acceptance.sh` runs:
   - `cargo fmt --check`
   - `cargo test --all-targets`
   - `cargo clippy`
   - Frontend tests and lint
   - Dependency audit
   - Health check
   - Load benchmark (200 req, 20 concurrent, p95 < 500ms)

## Production Evolution Path

Current architecture supports ~100 events/s on PostgreSQL alone. Evolution path documented in `docs/architecture.md`:
1. Split worker into separate deployment
2. Add message queue (NATS/Kafka) for ingest buffering
3. Migrate raw events to ClickHouse for large-scale analytics
4. Keep PostgreSQL for control plane (tenants, rules, Issue aggregates)

## Important Constraints

- Project key raw values are **never stored**, only SHA-256 hashes
- Session tokens only stored as SHA-256 hashes
- Bootstrap admin password must be changed in production
- DSN uses UUID v4 for projects, sequential `external_id` for numeric Sentry compatibility
- Event IDs are UUID v4; if client-provided, advisory lock serializes duplicate handling
- Issue merge/split moves events between issues and updates aggregates
- Project deletion requires 24-hour cooling-off period with slug confirmation
