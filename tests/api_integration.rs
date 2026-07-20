use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use open_sentry::{
    api, auth,
    config::{Settings, SmtpSettings, TelegramSettings, TwilioSettings},
    state::AppState,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

fn settings(database_url: String) -> Settings {
    Settings {
        environment: "test".into(),
        bind_addr: "127.0.0.1:0".into(),
        database_url,
        ingest_api_key: "dev-secret".into(),
        public_base_url: "http://localhost".into(),
        bootstrap_admin_email: "integration@example.com".into(),
        bootstrap_admin_password: "integration-password".into(),
        secure_cookies: false,
        ingest_rate_limit_per_minute: 1000,
        metrics_api_key: "integration-metrics".into(),
        retention_days: 30,
        retention_interval_seconds: 3600,
        worker_poll_interval_ms: 1000,
        smtp: SmtpSettings::default(),
        telegram: TelegramSettings::default(),
        twilio: TwilioSettings::default(),
    }
}

async fn response_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL and creates an isolated SQLx test database"]
async fn login_ingest_and_rbac_are_database_backed(pool: PgPool) {
    let config = settings(std::env::var("DATABASE_URL").unwrap());
    auth::ensure_bootstrap_admin(&pool, &config).await.unwrap();
    let app = api::router(Arc::new(AppState::new(pool.clone(), config)));

    let login = app
        .clone()
        .oneshot(
            Request::post("/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"email":"integration@example.com","password":"integration-password"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();

    let ingest = app
        .clone()
        .oneshot(
            Request::post("/api/projects/00000000-0000-0000-0000-000000000001/store")
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-sentry-auth", "dev-secret")
                .body(Body::from(
                    json!({"message":"integration event","tags":{"password":"secret"}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ingest.status(), StatusCode::ACCEPTED);
    let event = response_json(ingest).await;
    let stored: Value = sqlx::query_scalar("SELECT tags FROM events WHERE id=$1")
        .bind(event["id"].as_str().unwrap().parse::<uuid::Uuid>().unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored["password"], "[Filtered]");

    let projects = app
        .clone()
        .oneshot(
            Request::get("/api/projects")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(projects.status(), StatusCode::OK);
    assert!(!response_json(projects).await.as_array().unwrap().is_empty());

    let release = app
        .clone()
        .oneshot(
            Request::post("/api/projects/00000000-0000-0000-0000-000000000001/releases")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"version":"integration@1.0.0","description":"CI release"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(release.status(), StatusCode::CREATED);

    let source_map = app
        .clone()
        .oneshot(
            Request::post("/api/projects/00000000-0000-0000-0000-000000000001/debug-files")
                .header(header::COOKIE, &cookie)
                .header("x-debug-file-kind", "source_map")
                .header("x-debug-file-name", "app.js.map")
                .header("x-release", "integration@1.0.0")
                .body(Body::from(
                    r#"{"version":3,"sources":["src.ts"],"names":[],"mappings":"AAAA"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(source_map.status(), StatusCode::CREATED);

    let webhook = app
        .clone()
        .oneshot(
            Request::post("/api/projects/00000000-0000-0000-0000-000000000001/webhooks")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"name":"CI webhook","url":"https://example.invalid/hook"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(webhook.status(), StatusCode::CREATED);
    let webhook_body = response_json(webhook).await;
    assert!(
        webhook_body["signing_secret"]
            .as_str()
            .unwrap()
            .starts_with("whsec_")
    );

    let policy = app.oneshot(Request::post("/api/projects/00000000-0000-0000-0000-000000000001/escalation-policies").header(header::COOKIE,cookie).header(header::CONTENT_TYPE,"application/json").body(Body::from(json!({"name":"CI escalation","steps":[{"delay_seconds":0,"channel":"email","target":"oncall@example.com"},{"delay_seconds":60,"channel":"webhook","target":webhook_body["id"]}]}).to_string())).unwrap()).await.unwrap();
    assert_eq!(policy.status(), StatusCode::CREATED);
}
