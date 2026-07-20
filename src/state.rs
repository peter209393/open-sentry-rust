use sqlx::PgPool;

use crate::{
    config::Settings,
    operations::{IngestRateLimiter, RuntimeMetrics},
};

pub struct AppState {
    pub db: PgPool,
    pub settings: Settings,
    pub http: reqwest::Client,
    pub ingest_limiter: IngestRateLimiter,
    pub metrics: RuntimeMetrics,
}

impl AppState {
    pub fn new(db: PgPool, settings: Settings) -> Self {
        let ingest_rate_limit = settings.ingest_rate_limit_per_minute;
        Self {
            db,
            settings,
            http: reqwest::Client::new(),
            ingest_limiter: IngestRateLimiter::new(ingest_rate_limit),
            metrics: RuntimeMetrics::default(),
        }
    }
}
