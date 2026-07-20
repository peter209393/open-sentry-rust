use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct IngestEvent {
    pub event_id: Option<Uuid>,
    #[serde(default = "default_level")]
    pub level: String,
    pub message: String,
    #[serde(default)]
    pub fingerprint: Value,
    #[serde(default, deserialize_with = "deserialize_timestamp")]
    pub timestamp: Option<DateTime<Utc>>,
    pub environment: Option<String>,
    pub release: Option<String>,
    #[serde(default)]
    pub sdk: Value,
    pub server_name: Option<String>,
    pub platform: Option<String>,
    #[serde(default)]
    pub tags: Value,
    #[serde(default)]
    pub contexts: Value,
    #[serde(default)]
    pub exception: Value,
}

fn default_level() -> String {
    "error".to_owned()
}

fn deserialize_timestamp<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    let Some(value) = value else { return Ok(None) };
    match value {
        Value::String(value) => value
            .parse::<DateTime<Utc>>()
            .map(Some)
            .map_err(serde::de::Error::custom),
        Value::Number(value) => {
            let seconds = value
                .as_f64()
                .ok_or_else(|| serde::de::Error::custom("invalid numeric timestamp"))?;
            DateTime::from_timestamp_micros((seconds * 1_000_000.0) as i64)
                .map(Some)
                .ok_or_else(|| serde::de::Error::custom("timestamp is out of range"))
        }
        _ => Err(serde::de::Error::custom(
            "timestamp must be RFC3339 text or Unix seconds",
        )),
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct IssueSummary {
    pub id: Uuid,
    pub project_id: Uuid,
    pub fingerprint: String,
    pub title: String,
    pub level: String,
    pub status: String,
    pub event_count: i64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct StoredEvent {
    pub id: Uuid,
    pub project_id: Uuid,
    pub issue_id: Uuid,
    pub level: String,
    pub message: String,
    pub environment: Option<String>,
    pub release: Option<String>,
    pub tags: Value,
    pub contexts: Value,
    pub exception: Value,
    pub symbolicated_exception: Option<Value>,
    pub symbolication_status: String,
    pub received_at: DateTime<Utc>,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub status: Option<String>,
    pub q: Option<String>,
    pub level: Option<String>,
    pub assigned_to: Option<String>,
    pub service: Option<String>,
    pub environment: Option<String>,
    pub release: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub before: Option<DateTime<Utc>>,
    pub sort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TelemetryQuery {
    pub limit: Option<i64>,
    pub level: Option<String>,
    pub service: Option<String>,
    pub environment: Option<String>,
    pub release: Option<String>,
    pub trace_id: Option<String>,
    pub q: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub before: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAlertRule {
    pub name: String,
    pub level: Option<String>,
    pub message_contains: Option<String>,
    pub environment: Option<String>,
    pub cooldown_seconds: Option<i32>,
    pub channel: String,
    pub target: String,
    pub escalation_policy_id: Option<Uuid>,
    pub threshold_count: Option<i32>,
    pub window_seconds: Option<i32>,
    #[serde(default)]
    pub notify_recovery: bool,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AlertRuleView {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub level: Option<String>,
    pub message_contains: Option<String>,
    pub environment: Option<String>,
    pub cooldown_seconds: i32,
    pub channel: String,
    pub target: String,
    pub enabled: bool,
    pub escalation_policy_id: Option<Uuid>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TrendPoint {
    pub bucket: DateTime<Utc>,
    pub count: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct LevelCount {
    pub level: String,
    pub count: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EnvelopeItemSummary {
    pub id: Uuid,
    pub item_type: String,
    pub size_bytes: i64,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ProjectOverview {
    pub total_events: i64,
    pub events_24h: i64,
    pub unresolved_issues: i64,
    pub resolved_issues: i64,
    pub trends: Vec<TrendPoint>,
    pub levels: Vec<LevelCount>,
    pub recent_items: Vec<EnvelopeItemSummary>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ProjectDetails {
    pub id: Uuid,
    pub external_id: i64,
    pub name: String,
    pub slug: String,
    pub created_at: DateTime<Utc>,
    pub last_seen: Option<DateTime<Utc>>,
    pub service_count: i64,
    pub issue_count: i64,
    pub event_count: i64,
    pub latest_release: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ServiceView {
    pub id: Uuid,
    pub name: String,
    pub environment: String,
    pub latest_release: Option<String>,
    pub sdk_name: Option<String>,
    pub sdk_version: Option<String>,
    pub runtime: Value,
    pub event_count: i64,
    pub log_count: i64,
    pub issue_count: i64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct LogView {
    pub id: Uuid,
    pub service: Option<String>,
    pub environment: Option<String>,
    pub occurred_at: DateTime<Utc>,
    pub level: String,
    pub body: String,
    pub trace_id: Option<String>,
    pub attributes: Value,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EventStreamView {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub service: Option<String>,
    pub level: String,
    pub message: String,
    pub environment: Option<String>,
    pub release: Option<String>,
    pub trace_id: Option<String>,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AuditLogView {
    pub id: Uuid,
    pub actor_email: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<Uuid>,
    pub metadata: Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeConfigView {
    pub environment: String,
    pub ingest_rate_limit_per_minute: u64,
    pub retention_days: i64,
    pub secure_cookies: bool,
}
