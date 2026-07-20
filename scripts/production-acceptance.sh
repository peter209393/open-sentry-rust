#!/usr/bin/env bash
set -euo pipefail

backend_url="${BACKEND_URL:-http://127.0.0.1:8080}"
frontend_url="${FRONTEND_URL:-http://localhost:3000}"
metrics_key="${METRICS_API_KEY:-metrics-secret}"

cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
(
  cd frontend
  npm test
  npm run lint
  npm audit --audit-level=high
)

curl --fail --silent "${backend_url}/health" >/dev/null
curl --fail --silent "${frontend_url}/" >/dev/null
curl --fail --silent --header "authorization: Bearer ${metrics_key}" "${backend_url}/metrics" \
  | grep --quiet '^open_sentry_ingest_requests_total '

LOAD_URL="${backend_url}/api/projects/00000000-0000-0000-0000-000000000001/store" \
  cargo run --quiet --bin ingest-load

if [[ "${BUILD_IMAGES:-1}" == "1" ]]; then
  docker build --tag open-sentry-backend:acceptance .
  docker build --tag open-sentry-frontend:acceptance ./frontend
fi

printf 'production acceptance passed\n'
