use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Map, Value};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{error::Result, model::IngestEvent};

#[derive(Debug)]
pub struct EventService {
    pub id: Uuid,
    pub trace_id: Option<String>,
}

pub async fn upsert_event_service(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    event: &IngestEvent,
    occurred_at: DateTime<Utc>,
) -> Result<EventService> {
    let name = event
        .tags
        .get("service")
        .and_then(Value::as_str)
        .or_else(|| {
            event
                .contexts
                .pointer("/service/name")
                .and_then(Value::as_str)
        })
        .unwrap_or("unknown-service");
    let environment = event.environment.as_deref().unwrap_or("default");
    let sdk_name = event.sdk.get("name").and_then(Value::as_str);
    let sdk_version = event.sdk.get("version").and_then(Value::as_str);
    let mut runtime = event
        .contexts
        .get("runtime")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    if let Value::Object(runtime) = &mut runtime {
        if let Some(server_name) = &event.server_name {
            runtime.insert("server_name".into(), Value::String(server_name.clone()));
        }
        if let Some(platform) = &event.platform {
            runtime.insert("platform".into(), Value::String(platform.clone()));
        }
    }
    let trace_id = event
        .contexts
        .pointer("/trace/trace_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let id = sqlx::query_scalar::<_, Uuid>(
        r#"INSERT INTO services
           (project_id, name, environment, latest_release, sdk_name, sdk_version, runtime,
            event_count, first_seen, last_seen)
           VALUES ($1,$2,$3,$4,$5,$6,$7,1,$8,$8)
           ON CONFLICT (project_id, name, environment) DO UPDATE SET
             latest_release = COALESCE(EXCLUDED.latest_release, services.latest_release),
             sdk_name = COALESCE(EXCLUDED.sdk_name, services.sdk_name),
             sdk_version = COALESCE(EXCLUDED.sdk_version, services.sdk_version),
             runtime = CASE WHEN EXCLUDED.runtime = '{}'::jsonb THEN services.runtime ELSE EXCLUDED.runtime END,
             event_count = services.event_count + 1,
             first_seen = LEAST(services.first_seen, EXCLUDED.first_seen),
             last_seen = GREATEST(services.last_seen, EXCLUDED.last_seen)
           RETURNING id"#,
    )
    .bind(project_id)
    .bind(name)
    .bind(environment)
    .bind(&event.release)
    .bind(sdk_name)
    .bind(sdk_version)
    .bind(runtime)
    .bind(occurred_at)
    .fetch_one(&mut **tx)
    .await?;
    Ok(EventService { id, trace_id })
}

#[derive(Deserialize)]
struct LogBatch {
    items: Vec<SdkLog>,
}

#[derive(Deserialize)]
struct SdkLog {
    level: String,
    body: String,
    timestamp: f64,
    trace_id: Option<String>,
    #[serde(default)]
    attributes: Map<String, Value>,
}

fn attribute<'a>(attributes: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    attributes.get(name)?.get("value")
}

pub async fn persist_log_batch(
    db: &PgPool,
    project_id: Uuid,
    source_item_id: Uuid,
    payload: &[u8],
) -> Result<usize> {
    let batch: LogBatch = serde_json::from_slice(payload).map_err(|error| {
        crate::error::AppError::BadRequest(format!("invalid SDK log payload: {error}"))
    })?;
    let mut tx = db.begin().await?;
    for log in &batch.items {
        let occurred_at = DateTime::from_timestamp_micros((log.timestamp * 1_000_000.0) as i64)
            .unwrap_or_else(Utc::now);
        let service = attribute(&log.attributes, "service.name")
            .and_then(Value::as_str)
            .or_else(|| attribute(&log.attributes, "service").and_then(Value::as_str))
            .unwrap_or("unknown-service");
        let environment = attribute(&log.attributes, "sentry.environment")
            .and_then(Value::as_str)
            .unwrap_or("default");
        let release = attribute(&log.attributes, "sentry.release").and_then(Value::as_str);
        let sdk_name = attribute(&log.attributes, "sentry.sdk.name").and_then(Value::as_str);
        let sdk_version = attribute(&log.attributes, "sentry.sdk.version").and_then(Value::as_str);
        let service_id = sqlx::query_scalar::<_, Uuid>(
            r#"INSERT INTO services
               (project_id, name, environment, latest_release, sdk_name, sdk_version,
                log_count, first_seen, last_seen)
               VALUES ($1,$2,$3,$4,$5,$6,1,$7,$7)
               ON CONFLICT (project_id, name, environment) DO UPDATE SET
                 latest_release = COALESCE(EXCLUDED.latest_release, services.latest_release),
                 sdk_name = COALESCE(EXCLUDED.sdk_name, services.sdk_name),
                 sdk_version = COALESCE(EXCLUDED.sdk_version, services.sdk_version),
                 log_count = services.log_count + 1,
                 first_seen = LEAST(services.first_seen, EXCLUDED.first_seen),
                 last_seen = GREATEST(services.last_seen, EXCLUDED.last_seen)
               RETURNING id"#,
        )
        .bind(project_id)
        .bind(service)
        .bind(environment)
        .bind(release)
        .bind(sdk_name)
        .bind(sdk_version)
        .bind(occurred_at)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO logs
               (source_item_id, project_id, service_id, occurred_at, level, body, trace_id, attributes)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(source_item_id)
        .bind(project_id)
        .bind(service_id)
        .bind(occurred_at)
        .bind(&log.level)
        .bind(&log.body)
        .bind(&log.trace_id)
        .bind(Value::Object(log.attributes.clone()))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(batch.items.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_typed_log_attribute_value() {
        let attributes = serde_json::from_value(serde_json::json!({
            "service.name": {"value": "checkout", "type": "string"}
        }))
        .unwrap();
        assert_eq!(
            attribute(&attributes, "service.name").and_then(Value::as_str),
            Some("checkout")
        );
    }
}
