use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::Value;
use sqlx::PgPool;
use tracing::{error, info};
use uuid::Uuid;

use crate::{
    auth::UserView,
    error::{AppError, Result},
    state::AppState,
};

#[derive(Clone)]
pub struct IngestRateLimiter {
    limit: u64,
    windows: Arc<Mutex<HashMap<String, Window>>>,
}
#[derive(Clone, Copy)]
struct Window {
    started: Instant,
    count: u64,
}

impl IngestRateLimiter {
    pub fn new(limit: u64) -> Self {
        Self {
            limit: limit.max(1),
            windows: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    pub fn check(&self, key: &str, now: Instant) -> bool {
        let mut windows = self.windows.lock().expect("rate limiter lock poisoned");
        let window = windows.entry(key.to_owned()).or_insert(Window {
            started: now,
            count: 0,
        });
        if now.duration_since(window.started) >= Duration::from_secs(60) {
            *window = Window {
                started: now,
                count: 0,
            };
        }
        if window.count >= self.limit {
            return false;
        }
        window.count += 1;
        true
    }
}

#[derive(Clone, Default)]
pub struct RuntimeMetrics {
    inner: Arc<MetricsInner>,
}
#[derive(Default)]
struct MetricsInner {
    requests: AtomicU64,
    accepted: AtomicU64,
    rate_limited: AtomicU64,
    auth_failures: AtomicU64,
    retention_deleted: AtomicU64,
}
impl RuntimeMetrics {
    pub fn request(&self) {
        self.inner.requests.fetch_add(1, Ordering::Relaxed);
    }
    pub fn accepted(&self, n: u64) {
        self.inner.accepted.fetch_add(n, Ordering::Relaxed);
    }
    pub fn rate_limited(&self) {
        self.inner.rate_limited.fetch_add(1, Ordering::Relaxed);
    }
    pub fn auth_failure(&self) {
        self.inner.auth_failures.fetch_add(1, Ordering::Relaxed);
    }
    pub fn retention_deleted(&self, n: u64) {
        self.inner.retention_deleted.fetch_add(n, Ordering::Relaxed);
    }
    pub fn render(&self) -> String {
        format!(
            concat!(
                "# HELP open_sentry_ingest_requests_total Ingest HTTP requests.\n# TYPE open_sentry_ingest_requests_total counter\nopen_sentry_ingest_requests_total {}\n",
                "# HELP open_sentry_events_accepted_total Accepted envelope items and events.\n# TYPE open_sentry_events_accepted_total counter\nopen_sentry_events_accepted_total {}\n",
                "# HELP open_sentry_ingest_rate_limited_total Rate-limited ingest requests.\n# TYPE open_sentry_ingest_rate_limited_total counter\nopen_sentry_ingest_rate_limited_total {}\n",
                "# HELP open_sentry_auth_failures_total Failed authentication attempts.\n# TYPE open_sentry_auth_failures_total counter\nopen_sentry_auth_failures_total {}\n",
                "# HELP open_sentry_retention_deleted_rows_total Rows deleted by retention.\n# TYPE open_sentry_retention_deleted_rows_total counter\nopen_sentry_retention_deleted_rows_total {}\n"
            ),
            self.inner.requests.load(Ordering::Relaxed),
            self.inner.accepted.load(Ordering::Relaxed),
            self.inner.rate_limited.load(Ordering::Relaxed),
            self.inner.auth_failures.load(Ordering::Relaxed),
            self.inner.retention_deleted.load(Ordering::Relaxed)
        )
    }
}

pub async fn rate_limit_ingest(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> Result<Response> {
    state.metrics.request();
    let credential = request
        .headers()
        .get("x-sentry-auth")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("anonymous");
    let mut hasher = DefaultHasher::new();
    credential.hash(&mut hasher);
    let key = hasher.finish().to_string();
    if !state.ingest_limiter.check(&key, Instant::now()) {
        state.metrics.rate_limited();
        return Err(AppError::RateLimited);
    }
    Ok(next.run(request).await)
}

pub async fn metrics(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Result<Response> {
    let actual = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if actual != Some(&state.settings.metrics_api_key) {
        return Err(AppError::Unauthorized);
    }
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        state.metrics.render(),
    )
        .into_response())
}

pub async fn audit(
    db: &PgPool,
    user: &UserView,
    action: &str,
    resource_type: &str,
    resource_id: Option<Uuid>,
    metadata: Value,
) -> Result<()> {
    sqlx::query("INSERT INTO audit_logs (organization_id,actor_user_id,action,resource_type,resource_id,metadata) SELECT organization_id,$1,$2,$3,$4,$5 FROM users WHERE id=$1")
        .bind(user.id).bind(action).bind(resource_type).bind(resource_id).bind(metadata).execute(db).await?;
    Ok(())
}

pub async fn run_retention_once(db: &PgPool, retention_days: i64) -> Result<u64> {
    let days = retention_days.max(1);
    let logs = sqlx::query("DELETE FROM logs l USING projects p WHERE l.project_id=p.id AND l.occurred_at < now() - make_interval(days => COALESCE(p.retention_days,$1)::int)")
            .bind(days as i32)
            .execute(db)
            .await?
            .rows_affected();
    let events = sqlx::query(
        "DELETE FROM events e USING projects p WHERE e.project_id=p.id AND e.occurred_at < now() - make_interval(days => COALESCE(p.retention_days,$1)::int)",
    )
    .bind(days as i32)
    .execute(db)
    .await?
    .rows_affected();
    let items = sqlx::query(
        "DELETE FROM envelope_items e USING projects p WHERE e.project_id=p.id AND e.received_at < now() - make_interval(days => COALESCE(p.retention_days,$1)::int)",
    )
    .bind(days as i32)
    .execute(db)
    .await?
    .rows_affected();
    let sessions = sqlx::query("DELETE FROM user_sessions WHERE expires_at <= now()")
        .execute(db)
        .await?
        .rows_affected();
    Ok(logs + events + items + sessions)
}

pub async fn run_retention_worker(state: Arc<AppState>) {
    let mut timer = tokio::time::interval(Duration::from_secs(
        state.settings.retention_interval_seconds.max(60),
    ));
    loop {
        timer.tick().await;
        match run_retention_once(&state.db, state.settings.retention_days).await {
            Ok(n) => {
                state.metrics.retention_deleted(n);
                if n > 0 {
                    info!(deleted = n, "retention cleanup complete")
                }
            }
            Err(e) => error!(error=%e,"retention cleanup failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limiter_rejects_and_resets() {
        let limiter = IngestRateLimiter::new(2);
        let now = Instant::now();
        assert!(limiter.check("dsn", now));
        assert!(limiter.check("dsn", now));
        assert!(!limiter.check("dsn", now));
        assert!(limiter.check("dsn", now + Duration::from_secs(61)));
    }
    #[test]
    fn metrics_are_prometheus_text() {
        let m = RuntimeMetrics::default();
        m.request();
        m.accepted(2);
        m.rate_limited();
        let text = m.render();
        assert!(text.contains("open_sentry_ingest_requests_total 1"));
        assert!(text.contains("open_sentry_events_accepted_total 2"));
        assert!(text.contains("open_sentry_ingest_rate_limited_total 1"));
    }
}
