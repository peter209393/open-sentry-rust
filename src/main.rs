use std::sync::Arc;

use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

use open_sentry::{
    api, auth, config::Settings, notification, operations, state::AppState, symbols,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "open_sentry=info,tower_http=info".into()),
        )
        .init();

    let settings = Settings::load().context("load configuration")?;
    settings
        .validate_production()
        .context("validate production configuration")?;
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&settings.database_url)
        .await
        .context("connect to PostgreSQL")?;
    sqlx::migrate!()
        .run(&pool)
        .await
        .context("run migrations")?;

    auth::ensure_bootstrap_admin(&pool, &settings).await?;

    let state = Arc::new(AppState::new(pool, settings.clone()));
    let worker_state = state.clone();
    tokio::spawn(async move { notification::run_worker(worker_state).await });
    let retention_state = state.clone();
    tokio::spawn(async move { operations::run_retention_worker(retention_state).await });
    let symbol_state = state.clone();
    tokio::spawn(async move { symbols::run_worker(symbol_state).await });

    let listener = TcpListener::bind(&settings.bind_addr).await?;
    info!(address = %settings.bind_addr, "open-sentry listening");
    axum::serve(listener, api::router(state)).await?;
    Ok(())
}
