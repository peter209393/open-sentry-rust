use std::sync::Arc;

use axum::{
    Json, Router,
    body::Bytes,
    extract::DefaultBodyLimit,
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::{
    alert, envelope,
    error::{AppError, Result},
    model::{
        AlertRuleView, AuditLogView, CreateAlertRule, EnvelopeItemSummary, EventStreamView,
        IngestEvent, IssueSummary, LevelCount, ListQuery, LogView, ProjectDetails, ProjectOverview,
        RuntimeConfigView, ServiceView, StoredEvent, TelemetryQuery, TrendPoint,
    },
    state::AppState,
    telemetry,
};

pub fn router(state: Arc<AppState>) -> Router {
    let ingest_routes = Router::new()
        .route("/api/{project_id}/envelope/", post(ingest_envelope))
        .route("/api/projects/{project_id}/store", post(ingest))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::operations::rate_limit_ingest,
        ));
    let management = Router::new()
        .route("/api/projects", get(list_projects).post(create_project))
        .route("/api/projects/{project_id}/issues", get(list_issues))
        .route(
            "/api/projects/{project_id}",
            get(project_details)
                .patch(update_project)
                .delete(archive_project),
        )
        .route(
            "/api/projects/{project_id}/keys",
            get(list_project_keys).post(create_project_key),
        )
        .route(
            "/api/projects/{project_id}/keys/{key_id}",
            axum::routing::delete(revoke_project_key),
        )
        .route("/api/users", get(list_users).post(create_user))
        .route(
            "/api/invitations",
            get(list_invitations).post(create_invitation),
        )
        .route(
            "/api/users/{user_id}",
            axum::routing::patch(update_user).delete(deactivate_user),
        )
        .route("/api/users/{user_id}/sessions", get(list_user_sessions))
        .route(
            "/api/sessions/{session_id}",
            axum::routing::delete(revoke_session),
        )
        .route("/api/projects/{project_id}/services", get(list_services))
        .route("/api/projects/{project_id}/logs", get(list_project_logs))
        .route(
            "/api/projects/{project_id}/events",
            get(list_project_events),
        )
        .route("/api/projects/{project_id}/overview", get(project_overview))
        .route(
            "/api/projects/{project_id}/debug-files",
            get(list_debug_files).post(upload_debug_file),
        )
        .route(
            "/api/debug-files/{file_id}",
            axum::routing::delete(delete_debug_file),
        )
        .route("/api/events/{event_id}/reprocess", post(reprocess_event))
        .route(
            "/api/projects/{project_id}/releases",
            get(list_releases).post(create_release),
        )
        .route(
            "/api/projects/{project_id}/releases/compare",
            get(compare_releases),
        )
        .route("/api/releases/{release_id}", get(get_release))
        .route(
            "/api/releases/{release_id}/deployments",
            post(create_deployment),
        )
        .route("/api/releases/{release_id}/health", get(release_health))
        .route(
            "/api/projects/{project_id}/webhooks",
            get(list_webhooks).post(create_webhook),
        )
        .route(
            "/api/webhooks/{endpoint_id}",
            axum::routing::delete(delete_webhook),
        )
        .route("/api/webhooks/{endpoint_id}/test", post(test_webhook))
        .route(
            "/api/projects/{project_id}/on-call-schedules",
            get(list_schedules).post(create_schedule),
        )
        .route(
            "/api/projects/{project_id}/escalation-policies",
            get(list_policies).post(create_policy),
        )
        .route(
            "/api/issues/{issue_id}",
            get(get_issue).patch(update_issue).post(fix_issue),
        )
        .route("/api/issues/{issue_id}/events", get(list_events))
        .route(
            "/api/issues/{issue_id}/comments",
            get(list_issue_comments).post(create_issue_comment),
        )
        .route(
            "/api/issues/batch",
            axum::routing::patch(batch_update_issues),
        )
        .route("/api/issues/merge", post(merge_issues))
        .route("/api/issues/{issue_id}/split", post(split_issue))
        .route(
            "/api/issues/{issue_id}/fingerprint",
            axum::routing::patch(change_fingerprint),
        )
        .route(
            "/api/envelope-items/{item_id}/download",
            get(download_envelope_item),
        )
        .route(
            "/api/projects/{project_id}/deletion-request",
            post(request_project_deletion).delete(cancel_project_deletion),
        )
        .route("/api/projects/{project_id}/purge", post(purge_project))
        .route(
            "/api/projects/{project_id}/alert-rules",
            get(list_rules).post(create_rule),
        )
        .route(
            "/api/alert-rules/{rule_id}",
            axum::routing::patch(update_rule).delete(delete_rule),
        )
        .route("/api/alert-rules/{rule_id}/test", post(test_rule))
        .route("/api/alert-rules/{rule_id}/check", get(check_rule_channel))
        .route(
            "/api/projects/{project_id}/notifications",
            get(list_notifications),
        )
        .route(
            "/api/notifications/{notification_id}/retry",
            post(retry_notification),
        )
        .route("/api/audit-logs", get(list_audit_logs))
        .route("/api/runtime-config", get(runtime_config))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::auth::require_session,
        ));

    Router::new()
        .route("/health", get(health))
        .route("/api/auth/login", post(crate::auth::login))
        .route("/api/auth/logout", post(crate::auth::logout))
        .route("/api/auth/me", get(crate::auth::me))
        .route("/api/invitations/accept", post(accept_invitation))
        .route("/metrics", get(crate::operations::metrics))
        .merge(ingest_routes)
        .merge(management)
        .layer(TraceLayer::new_for_http())
        .layer(DefaultBodyLimit::max(envelope::MAX_COMPRESSED_BYTES))
        .with_state(state)
}

async fn health(State(state): State<Arc<AppState>>) -> Result<Json<Value>> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.db)
        .await?;
    Ok(Json(json!({ "status": "ok" })))
}

async fn ingest(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Json(event): Json<IngestEvent>,
) -> Result<(StatusCode, Json<Value>)> {
    authorize_project_key(&state, project_id, &headers).await?;
    let response = persist_event(&state, project_id, event).await?;
    state.metrics.accepted(1);
    Ok(response)
}

async fn authorize_project_key(
    state: &AppState,
    project_id: Uuid,
    headers: &HeaderMap,
) -> Result<()> {
    let raw = headers
        .get("x-sentry-auth")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| sentry_key(headers).or(Some(value)))
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
        })
        .ok_or(AppError::Unauthorized)?;
    let key_hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
    let key_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT k.id FROM project_keys k JOIN projects p ON p.id=k.project_id WHERE k.project_id=$1 AND k.key_hash=$2 AND k.revoked_at IS NULL AND p.archived_at IS NULL",
    )
    .bind(project_id).bind(key_hash).fetch_optional(&state.db).await?
    .ok_or(AppError::Unauthorized)?;
    sqlx::query("UPDATE project_keys SET last_used_at=now() WHERE id=$1")
        .bind(key_id)
        .execute(&state.db)
        .await?;
    Ok(())
}

async fn ingest_envelope(
    State(state): State<Arc<AppState>>,
    Path(external_project_id): Path<i64>,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>)> {
    // 浏览器 Sentry SDK 把公钥放在 URL query(?sentry_key=)；server-to-server
    // 放在 X-Sentry-Auth 头。两者都接受。
    let public_key = sentry_key(&headers)
        .map(str::to_owned)
        .or_else(|| query_sentry_key(raw_query.as_deref()))
        .ok_or(AppError::Unauthorized)?;
    let key_hash = format!("{:x}", Sha256::digest(public_key.as_bytes()));
    let (project_id, key_id) = sqlx::query_as::<_, (Uuid, Uuid)>(
        r#"SELECT p.id, k.id FROM projects p
           JOIN project_keys k ON k.project_id = p.id
           WHERE p.external_id = $1 AND k.key_hash = $2 AND k.revoked_at IS NULL
             AND p.archived_at IS NULL"#,
    )
    .bind(external_project_id)
    .bind(key_hash)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::Unauthorized)?;
    sqlx::query("UPDATE project_keys SET last_used_at=now() WHERE id=$1")
        .bind(key_id)
        .execute(&state.db)
        .await?;

    let encoding = headers
        .get("content-encoding")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let decoded = tokio::task::spawn_blocking(move || envelope::decode(encoding.as_deref(), &body))
        .await
        .map_err(|error| AppError::Internal(error.into()))??;
    let items = envelope::parse(&decoded)?;
    let mut accepted = Vec::new();
    for item in items {
        if matches!(item.item_type.as_str(), "event" | "transaction") {
            let event = event_from_payload(&item.payload)?;
            let (_, response) = persist_event(&state, project_id, event).await?;
            accepted.push(response.0);
        } else {
            let item_id = sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO envelope_items (project_id, item_type, payload) VALUES ($1, $2, $3) RETURNING id",
            )
            .bind(project_id)
            .bind(&item.item_type)
            .bind(&item.payload)
            .fetch_one(&state.db)
            .await?;
            let records = if item.item_type == "log" {
                telemetry::persist_log_batch(&state.db, project_id, item_id, &item.payload).await?
            } else {
                1
            };
            accepted.push(json!({ "id": item_id, "type": item.item_type, "records": records }));
        }
    }
    state.metrics.accepted(accepted.len() as u64);
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "accepted": accepted.len(), "events": accepted })),
    ))
}

fn sentry_key(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get("x-sentry-auth")?.to_str().ok()?;
    value
        .strip_prefix("Sentry ")?
        .split(',')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("sentry_key="))
}

fn query_sentry_key(query: Option<&str>) -> Option<String> {
    let query = query?;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("sentry_key="))
        .map(str::to_owned)
}

fn event_from_payload(payload: &[u8]) -> Result<IngestEvent> {
    let mut value: Value = serde_json::from_slice(payload)
        .map_err(|error| AppError::BadRequest(format!("invalid event payload: {error}")))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| AppError::BadRequest("event payload must be an object".into()))?;
    if !object.get("message").is_some_and(Value::is_string) {
        let message = object
            .get("message")
            .and_then(|message| message.get("formatted"))
            .and_then(Value::as_str)
            .or_else(|| object.get("transaction").and_then(Value::as_str))
            .unwrap_or("event")
            .to_owned();
        object.insert("message".into(), Value::String(message));
    }
    serde_json::from_value(value)
        .map_err(|error| AppError::BadRequest(format!("invalid event payload: {error}")))
}

async fn persist_event(
    state: &AppState,
    project_id: Uuid,
    mut event: IngestEvent,
) -> Result<(StatusCode, Json<Value>)> {
    if event.message.trim().is_empty() {
        return Err(AppError::BadRequest("message is required".into()));
    }
    let scrub_fields = sqlx::query_scalar::<_, Value>(
        "SELECT scrub_fields FROM projects WHERE id=$1 AND archived_at IS NULL",
    )
    .bind(project_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let fields = scrub_fields
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    scrub_sensitive_with(&mut event.tags, &fields);
    scrub_sensitive_with(&mut event.contexts, &fields);
    scrub_sensitive_with(&mut event.exception, &fields);

    let event_id = event.event_id.unwrap_or_else(Uuid::new_v4);
    let occurred_at = event.timestamp.unwrap_or_else(chrono::Utc::now);
    let fingerprint = fingerprint(&event);
    let mut tx = state.db.begin().await?;
    // Serialize requests for the same client-provided event id so only one of
    // them can mutate the aggregate and enqueue notifications.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(event_id)
        .execute(&mut *tx)
        .await?;
    if let Some((existing_project_id, existing_issue_id)) =
        sqlx::query_as::<_, (Uuid, Uuid)>("SELECT project_id, issue_id FROM events WHERE id = $1")
            .bind(event_id)
            .fetch_optional(&mut *tx)
            .await?
    {
        if existing_project_id != project_id {
            return Err(AppError::Conflict(
                "event id is already used by another project".into(),
            ));
        }
        tx.commit().await?;
        return Ok((
            StatusCode::ACCEPTED,
            Json(json!({ "id": event_id, "issue_id": existing_issue_id })),
        ));
    }
    let issue_id = sqlx::query_scalar::<_, Uuid>(
        r#"INSERT INTO issues (project_id, fingerprint, title, level, first_seen, last_seen)
           VALUES ($1, $2, $3, $4, $5, $5)
           ON CONFLICT (project_id, fingerprint) DO UPDATE SET
             first_seen = LEAST(issues.first_seen, EXCLUDED.first_seen),
             last_seen = GREATEST(issues.last_seen, EXCLUDED.last_seen),
             event_count = issues.event_count + 1,
             level = EXCLUDED.level, title = EXCLUDED.title,
             regressed_at = CASE WHEN issues.status='resolved' THEN EXCLUDED.last_seen ELSE issues.regressed_at END,
             status = CASE WHEN issues.status='resolved' THEN 'unresolved' ELSE issues.status END
           RETURNING id"#,
    )
    .bind(project_id)
    .bind(fingerprint)
    .bind(&event.message)
    .bind(&event.level)
    .bind(occurred_at)
    .fetch_one(&mut *tx)
    .await?;

    let service = telemetry::upsert_event_service(&mut tx, project_id, &event, occurred_at).await?;

    sqlx::query(
        r#"INSERT INTO events
           (id, project_id, issue_id, service_id, trace_id, level, message, environment, release, tags, contexts, exception, occurred_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(event_id).bind(project_id).bind(issue_id).bind(service.id).bind(service.trace_id)
    .bind(&event.level).bind(&event.message).bind(&event.environment).bind(&event.release)
    .bind(&event.tags).bind(&event.contexts).bind(&event.exception).bind(occurred_at)
    .execute(&mut *tx).await?;

    if let Some(release) = &event.release {
        sqlx::query("INSERT INTO releases(project_id,version,first_event_at,last_event_at,event_count) VALUES($1,$2,$3,$3,1) ON CONFLICT(project_id,version) DO UPDATE SET first_event_at=LEAST(releases.first_event_at,EXCLUDED.first_event_at),last_event_at=GREATEST(releases.last_event_at,EXCLUDED.last_event_at),event_count=releases.event_count+1")
            .bind(project_id).bind(release).bind(occurred_at).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO symbolication_jobs(event_id) VALUES($1) ON CONFLICT(event_id) DO NOTHING",
        )
        .bind(event_id)
        .execute(&mut *tx)
        .await?;
    }

    alert::enqueue_matching_alerts(&mut tx, project_id, issue_id, event_id, &event).await?;
    tx.commit().await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "id": event_id, "issue_id": issue_id })),
    ))
}

#[cfg(test)]
fn authorize_ingest(headers: &HeaderMap, expected: &str) -> Result<()> {
    let actual = headers
        .get("x-sentry-auth")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
        });
    if actual == Some(expected) {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

fn fingerprint(event: &IngestEvent) -> String {
    let exception_type = event
        .exception
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| {
            event.exception["values"]
                .as_array()
                .and_then(|values| values.first())
                .and_then(|exception| exception.get("type"))
                .and_then(Value::as_str)
        })
        .unwrap_or("");
    let normalized = event
        .message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let frames = event
        .exception
        .get("values")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
        .and_then(|exception| exception.get("stacktrace"))
        .or_else(|| event.exception.get("stacktrace"))
        .and_then(|stacktrace| stacktrace.get("frames"))
        .and_then(Value::as_array)
        .map(|frames| {
            frames
                .iter()
                .rev()
                .filter(|frame| frame.get("in_app").and_then(Value::as_bool) != Some(false))
                .take(5)
                .map(|frame| {
                    let module = frame.get("module").and_then(Value::as_str).unwrap_or("");
                    let function = frame.get("function").and_then(Value::as_str).unwrap_or("");
                    let filename = frame.get("filename").and_then(Value::as_str).unwrap_or("");
                    format!("{module}:{function}:{filename}")
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let default_component = if frames.is_empty() {
        format!("{}|{}|{}", event.level, exception_type, normalized)
    } else {
        format!("{}|{}|{}", event.level, exception_type, frames.join("|"))
    };
    let custom = match &event.fingerprint {
        Value::Array(values) => values.iter().filter_map(Value::as_str).collect::<Vec<_>>(),
        Value::String(value) if !value.is_empty() => vec![value.as_str()],
        _ => Vec::new(),
    };
    let input = if custom.is_empty() {
        default_component.clone()
    } else {
        custom
            .iter()
            .map(|component| {
                if *component == "{{ default }}" {
                    default_component.as_str()
                } else {
                    component
                }
            })
            .collect::<Vec<_>>()
            .join("|")
    };
    format!("{:x}", Sha256::digest(input))
}

#[cfg(test)]
fn scrub_sensitive(value: &mut Value) {
    let fields = [
        "password",
        "passwd",
        "authorization",
        "cookie",
        "token",
        "secret",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    scrub_sensitive_with(value, &fields);
}

fn scrub_sensitive_with(value: &mut Value, fields: &[String]) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if fields
                    .iter()
                    .any(|field| field == &key.to_ascii_lowercase())
                {
                    *value = Value::String("[Filtered]".into());
                } else {
                    scrub_sensitive_with(value, fields);
                }
            }
        }
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| scrub_sensitive_with(value, fields)),
        _ => {}
    }
}

#[derive(Deserialize)]
struct CreateProject {
    name: String,
    slug: String,
    retention_days: Option<i32>,
}
#[derive(Deserialize)]
struct UpdateProject {
    name: Option<String>,
    retention_days: Option<i32>,
    scrub_fields: Option<Vec<String>>,
}
#[derive(Deserialize)]
struct CreateKey {
    name: String,
}

async fn current_admin(state: &AppState, headers: &HeaderMap) -> Result<crate::auth::UserView> {
    let user = crate::auth::authenticate(&state.db, headers).await?;
    crate::auth::require_admin(&user)?;
    Ok(user)
}

async fn list_projects(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let user = crate::auth::authenticate(&state.db, &headers).await?;
    let rows = sqlx::query_as::<_, (Uuid, i64, String, String, Option<i32>, Option<chrono::DateTime<chrono::Utc>>, chrono::DateTime<chrono::Utc>)>(
        "SELECT p.id,p.external_id,p.name,p.slug,p.retention_days,p.archived_at,p.created_at FROM projects p JOIN users u ON u.organization_id=p.organization_id WHERE u.id=$1 ORDER BY p.created_at",
    ).bind(user.id).fetch_all(&state.db).await?;
    Ok(Json(json!(rows.into_iter().map(|r| json!({"id":r.0,"external_id":r.1,"name":r.2,"slug":r.3,"retention_days":r.4,"archived_at":r.5,"created_at":r.6})).collect::<Vec<_>>())))
}

async fn create_project(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateProject>,
) -> Result<(StatusCode, Json<Value>)> {
    let user = current_admin(&state, &headers).await?;
    let slug = body.slug.trim().to_ascii_lowercase();
    if body.name.trim().is_empty()
        || slug.len() < 2
        || !slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(AppError::BadRequest(
            "name and a valid slug are required".into(),
        ));
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(7265351)")
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query_as::<_, (Uuid,i64,String,String)>(
        "INSERT INTO projects (organization_id,external_id,name,slug,retention_days) SELECT organization_id,(SELECT COALESCE(max(external_id),0)+1 FROM projects),$2,$3,$4 FROM users WHERE id=$1 RETURNING id,external_id,name,slug",
    ).bind(user.id).bind(body.name.trim()).bind(slug).bind(body.retention_days).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    crate::operations::audit(
        &state.db,
        &user,
        "project.created",
        "project",
        Some(row.0),
        json!({"slug":row.3}),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id":row.0,"external_id":row.1,"name":row.2,"slug":row.3})),
    ))
}

async fn update_project(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<UpdateProject>,
) -> Result<Json<Value>> {
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    if body.retention_days.is_some_and(|days| days < 1) {
        return Err(AppError::BadRequest(
            "retention_days must be positive".into(),
        ));
    }
    let fields = body.scrub_fields.map(|items| {
        json!(
            items
                .into_iter()
                .map(|v| v.to_ascii_lowercase())
                .collect::<Vec<_>>()
        )
    });
    let row = sqlx::query_as::<_,(String,Option<i32>,Value)>("UPDATE projects SET name=COALESCE($2,name),retention_days=COALESCE($3,retention_days),scrub_fields=COALESCE($4,scrub_fields) WHERE id=$1 RETURNING name,retention_days,scrub_fields")
        .bind(project_id).bind(body.name).bind(body.retention_days).bind(fields).fetch_optional(&state.db).await?.ok_or(AppError::NotFound)?;
    crate::operations::audit(
        &state.db,
        &user,
        "project.updated",
        "project",
        Some(project_id),
        json!({}),
    )
    .await?;
    Ok(Json(
        json!({"id":project_id,"name":row.0,"retention_days":row.1,"scrub_fields":row.2}),
    ))
}

async fn archive_project(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    sqlx::query("UPDATE projects SET archived_at=now() WHERE id=$1")
        .bind(project_id)
        .execute(&state.db)
        .await?;
    sqlx::query(
        "UPDATE project_keys SET revoked_at=COALESCE(revoked_at,now()) WHERE project_id=$1",
    )
    .bind(project_id)
    .execute(&state.db)
    .await?;
    crate::operations::audit(
        &state.db,
        &user,
        "project.archived",
        "project",
        Some(project_id),
        json!({}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_project_keys(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let rows=sqlx::query_as::<_,(Uuid,String,chrono::DateTime<chrono::Utc>,Option<chrono::DateTime<chrono::Utc>>,Option<chrono::DateTime<chrono::Utc>>)>("SELECT id,name,created_at,last_used_at,revoked_at FROM project_keys WHERE project_id=$1 ORDER BY created_at DESC").bind(project_id).fetch_all(&state.db).await?;
    Ok(Json(json!(rows.into_iter().map(|r|json!({"id":r.0,"name":r.1,"created_at":r.2,"last_used_at":r.3,"revoked_at":r.4})).collect::<Vec<_>>())))
}

async fn create_project_key(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<CreateKey>,
) -> Result<(StatusCode, Json<Value>)> {
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("key name is required".into()));
    }
    let raw = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
    let (id,external_id)=sqlx::query_as::<_,(Uuid,i64)>("INSERT INTO project_keys(project_id,name,key_hash) VALUES($1,$2,$3) RETURNING id,(SELECT external_id FROM projects WHERE id=$1)").bind(project_id).bind(body.name.trim()).bind(hash).fetch_one(&state.db).await?;
    let base = state.settings.public_base_url.trim_end_matches('/');
    let dsn = format!(
        "{base_scheme}{raw}@{host}/{external_id}",
        base_scheme = if base.starts_with("https://") {
            "https://"
        } else {
            "http://"
        },
        host = base
            .trim_start_matches("https://")
            .trim_start_matches("http://")
    );
    crate::operations::audit(
        &state.db,
        &user,
        "project_key.created",
        "project_key",
        Some(id),
        json!({"project_id":project_id}),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id":id,"name":body.name,"key":raw,"dsn":dsn,"shown_once":true})),
    ))
}

async fn revoke_project_key(
    State(state): State<Arc<AppState>>,
    Path((project_id, key_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    let changed=sqlx::query("UPDATE project_keys SET revoked_at=now() WHERE id=$1 AND project_id=$2 AND revoked_at IS NULL").bind(key_id).bind(project_id).execute(&state.db).await?.rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    crate::operations::audit(
        &state.db,
        &user,
        "project_key.revoked",
        "project_key",
        Some(key_id),
        json!({"project_id":project_id}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct CreateUser {
    email: String,
    display_name: String,
    password: String,
    role: String,
}
#[derive(Deserialize)]
struct UpdateUser {
    display_name: Option<String>,
    role: Option<String>,
    active: Option<bool>,
}

async fn list_users(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::auth::ManagedUserView>>> {
    let user = current_admin(&state, &headers).await?;
    let rows=sqlx::query_as::<_,crate::auth::ManagedUserView>("SELECT id,email,display_name,role,active,last_login_at,created_at FROM users WHERE organization_id=(SELECT organization_id FROM users WHERE id=$1) ORDER BY created_at").bind(user.id).fetch_all(&state.db).await?;
    Ok(Json(rows))
}
async fn create_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateUser>,
) -> Result<(StatusCode, Json<crate::auth::ManagedUserView>)> {
    let actor = current_admin(&state, &headers).await?;
    if !matches!(body.role.as_str(), "owner" | "admin" | "member") {
        return Err(AppError::BadRequest("invalid role".into()));
    }
    let hash = crate::auth::hash_password(&body.password)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let row=sqlx::query_as::<_,crate::auth::ManagedUserView>("INSERT INTO users(organization_id,email,display_name,password_hash,role) SELECT organization_id,lower($2),$3,$4,$5 FROM users WHERE id=$1 RETURNING id,email,display_name,role,active,last_login_at,created_at").bind(actor.id).bind(body.email.trim()).bind(body.display_name.trim()).bind(hash).bind(body.role).fetch_one(&state.db).await?;
    crate::operations::audit(
        &state.db,
        &actor,
        "user.created",
        "user",
        Some(row.id),
        json!({"role":row.role}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(row)))
}
async fn update_user(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<UpdateUser>,
) -> Result<Json<crate::auth::ManagedUserView>> {
    let actor = current_admin(&state, &headers).await?;
    if body
        .role
        .as_deref()
        .is_some_and(|r| !matches!(r, "owner" | "admin" | "member"))
    {
        return Err(AppError::BadRequest("invalid role".into()));
    }
    if actor.id == user_id && (body.active == Some(false) || body.role.as_deref() == Some("member"))
    {
        return Err(AppError::Conflict(
            "cannot remove your own administrative access".into(),
        ));
    }
    let row=sqlx::query_as::<_,crate::auth::ManagedUserView>("UPDATE users SET display_name=COALESCE($3,display_name),role=COALESCE($4,role),active=COALESCE($5,active) WHERE id=$2 AND organization_id=(SELECT organization_id FROM users WHERE id=$1) RETURNING id,email,display_name,role,active,last_login_at,created_at").bind(actor.id).bind(user_id).bind(body.display_name).bind(body.role).bind(body.active).fetch_optional(&state.db).await?.ok_or(AppError::NotFound)?;
    if !row.active {
        sqlx::query("DELETE FROM user_sessions WHERE user_id=$1")
            .bind(user_id)
            .execute(&state.db)
            .await?;
    }
    crate::operations::audit(
        &state.db,
        &actor,
        "user.updated",
        "user",
        Some(user_id),
        json!({"role":row.role,"active":row.active}),
    )
    .await?;
    Ok(Json(row))
}
async fn deactivate_user(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let _ = update_user(
        State(state),
        Path(user_id),
        headers,
        Json(UpdateUser {
            display_name: None,
            role: None,
            active: Some(false),
        }),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn list_user_sessions(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let actor = current_admin(&state, &headers).await?;
    let rows=sqlx::query_as::<_,(Uuid,chrono::DateTime<chrono::Utc>,chrono::DateTime<chrono::Utc>,chrono::DateTime<chrono::Utc>)>("SELECT s.id,s.created_at,s.last_seen_at,s.expires_at FROM user_sessions s JOIN users u ON u.id=s.user_id WHERE s.user_id=$2 AND u.organization_id=(SELECT organization_id FROM users WHERE id=$1) ORDER BY s.last_seen_at DESC").bind(actor.id).bind(user_id).fetch_all(&state.db).await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|r| json!({"id":r.0,"created_at":r.1,"last_seen_at":r.2,"expires_at":r.3}))
            .collect::<Vec<_>>()
    )))
}
async fn revoke_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let actor = current_admin(&state, &headers).await?;
    let n=sqlx::query("DELETE FROM user_sessions s USING users u WHERE s.id=$2 AND s.user_id=u.id AND u.organization_id=(SELECT organization_id FROM users WHERE id=$1)").bind(actor.id).bind(session_id).execute(&state.db).await?.rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct InvitationInput {
    email: String,
    display_name: String,
    role: String,
}
#[derive(Deserialize)]
struct AcceptInvitationInput {
    token: String,
    password: String,
}
async fn list_invitations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let user = current_admin(&state, &headers).await?;
    let rows=sqlx::query_as::<_,(Uuid,String,String,String,chrono::DateTime<chrono::Utc>,Option<chrono::DateTime<chrono::Utc>>)>("SELECT id,email,display_name,role,expires_at,accepted_at FROM user_invitations WHERE organization_id=(SELECT organization_id FROM users WHERE id=$1) ORDER BY created_at DESC").bind(user.id).fetch_all(&state.db).await?;
    Ok(Json(json!(rows.into_iter().map(|r|json!({"id":r.0,"email":r.1,"display_name":r.2,"role":r.3,"expires_at":r.4,"accepted_at":r.5})).collect::<Vec<_>>())))
}
async fn create_invitation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<InvitationInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let user = current_admin(&state, &headers).await?;
    if !matches!(body.role.as_str(), "admin" | "member") {
        return Err(AppError::BadRequest("invalid invitation role".into()));
    }
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let hash = format!("{:x}", Sha256::digest(token.as_bytes()));
    let id=sqlx::query_scalar::<_,Uuid>("INSERT INTO user_invitations(organization_id,email,display_name,role,token_hash,invited_by,expires_at) SELECT organization_id,lower($2),$3,$4,$5,id,now()+interval '48 hours' FROM users WHERE id=$1 RETURNING id").bind(user.id).bind(&body.email).bind(&body.display_name).bind(&body.role).bind(hash).fetch_one(&state.db).await?;
    let link = format!(
        "{}/invite?token={}",
        state.settings.public_base_url.trim_end_matches('/'),
        token
    );
    let delivery = crate::notification::send_invitation_email(&state, &body.email, &link).await;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"id":id,"invite_url":link,"shown_once":true,"email_sent":delivery.is_ok(),"email_error":delivery.err().map(|e|e.to_string())}),
        ),
    ))
}
async fn accept_invitation(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AcceptInvitationInput>,
) -> Result<(StatusCode, Json<crate::auth::ManagedUserView>)> {
    let hash = format!("{:x}", Sha256::digest(body.token.as_bytes()));
    let password_hash = crate::auth::hash_password(&body.password)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let mut tx = state.db.begin().await?;
    let invitation=sqlx::query_as::<_,(Uuid,Uuid,String,String,String)>("SELECT id,organization_id,email,display_name,role FROM user_invitations WHERE token_hash=$1 AND accepted_at IS NULL AND expires_at>now() FOR UPDATE").bind(hash).fetch_optional(&mut*tx).await?.ok_or(AppError::Unauthorized)?;
    let row=sqlx::query_as::<_,crate::auth::ManagedUserView>("INSERT INTO users(organization_id,email,display_name,password_hash,role) VALUES($1,$2,$3,$4,$5) RETURNING id,email,display_name,role,active,last_login_at,created_at").bind(invitation.1).bind(invitation.2).bind(invitation.3).bind(password_hash).bind(invitation.4).fetch_one(&mut*tx).await?;
    sqlx::query("UPDATE user_invitations SET accepted_at=now() WHERE id=$1")
        .bind(invitation.0)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}

#[derive(Deserialize)]
struct DeleteRequest {
    confirmation: String,
}
async fn request_project_deletion(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<DeleteRequest>,
) -> Result<(StatusCode, Json<Value>)> {
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    if user.role != "owner" {
        return Err(AppError::Forbidden);
    }
    let slug = sqlx::query_scalar::<_, String>("SELECT slug FROM projects WHERE id=$1")
        .bind(project_id)
        .fetch_one(&state.db)
        .await?;
    if body.confirmation != slug {
        return Err(AppError::BadRequest(
            "confirmation must equal project slug".into(),
        ));
    }
    let id=sqlx::query_scalar::<_,Uuid>("INSERT INTO project_deletion_requests(project_id,requested_by,confirmation,execute_after) VALUES($1,$2,$3,now()+interval '24 hours') ON CONFLICT(project_id) DO UPDATE SET requested_by=$2,confirmation=$3,execute_after=now()+interval '24 hours' RETURNING id").bind(project_id).bind(user.id).bind(body.confirmation).fetch_one(&state.db).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"id":id,"execute_after_hours":24})),
    ))
}
async fn cancel_project_deletion(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    if user.role != "owner" {
        return Err(AppError::Forbidden);
    }
    sqlx::query("DELETE FROM project_deletion_requests WHERE project_id=$1")
        .bind(project_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn purge_project(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<DeleteRequest>,
) -> Result<StatusCode> {
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    if user.role != "owner" {
        return Err(AppError::Forbidden);
    }
    let allowed=sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM project_deletion_requests d JOIN projects p ON p.id=d.project_id WHERE d.project_id=$1 AND d.execute_after<=now() AND d.confirmation=$2)").bind(project_id).bind(body.confirmation).fetch_one(&state.db).await?;
    if !allowed {
        return Err(AppError::Conflict(
            "deletion cooling-off period has not elapsed".into(),
        ));
    }
    sqlx::query("DELETE FROM projects WHERE id=$1")
        .bind(project_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct IssueCommentInput {
    body: String,
}
#[derive(Deserialize)]
struct BatchIssueUpdate {
    issue_ids: Vec<Uuid>,
    status: String,
    assigned_to: Option<String>,
}
async fn list_issue_comments(
    State(state): State<Arc<AppState>>,
    Path(issue_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    crate::auth::authorize_issue(&state.db, &headers, issue_id).await?;
    let rows=sqlx::query_as::<_,(Uuid,String,String,chrono::DateTime<chrono::Utc>)>("SELECT c.id,u.display_name,c.body,c.created_at FROM issue_comments c JOIN users u ON u.id=c.author_user_id WHERE c.issue_id=$1 ORDER BY c.created_at").bind(issue_id).fetch_all(&state.db).await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|r| json!({"id":r.0,"author":r.1,"body":r.2,"created_at":r.3}))
            .collect::<Vec<_>>()
    )))
}
async fn create_issue_comment(
    State(state): State<Arc<AppState>>,
    Path(issue_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<IssueCommentInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let user = crate::auth::authorize_issue(&state.db, &headers, issue_id).await?;
    let text = body.body.trim();
    if text.is_empty() || text.len() > 5000 {
        return Err(AppError::BadRequest(
            "comment must contain 1-5000 characters".into(),
        ));
    }
    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO issue_comments(issue_id,author_user_id,body) VALUES($1,$2,$3) RETURNING id",
    )
    .bind(issue_id)
    .bind(user.id)
    .bind(text)
    .fetch_one(&state.db)
    .await?;
    crate::operations::audit(
        &state.db,
        &user,
        "issue.comment_created",
        "issue",
        Some(issue_id),
        json!({"comment_id":id}),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id":id,"body":text,"author":user.display_name})),
    ))
}
async fn batch_update_issues(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<BatchIssueUpdate>,
) -> Result<Json<Value>> {
    let user = current_admin(&state, &headers).await?;
    if body.issue_ids.is_empty() || body.issue_ids.len() > 200 {
        return Err(AppError::BadRequest(
            "issue_ids must contain 1-200 items".into(),
        ));
    }
    if !matches!(
        body.status.as_str(),
        "unresolved" | "in_progress" | "resolved" | "ignored"
    ) {
        return Err(AppError::BadRequest("invalid issue status".into()));
    }
    let n=sqlx::query("UPDATE issues i SET status=$3,assigned_to=COALESCE($4,assigned_to) FROM projects p WHERE i.project_id=p.id AND i.id=ANY($2) AND p.organization_id=(SELECT organization_id FROM users WHERE id=$1)").bind(user.id).bind(&body.issue_ids).bind(&body.status).bind(body.assigned_to).execute(&state.db).await?.rows_affected();
    crate::operations::audit(
        &state.db,
        &user,
        "issue.batch_updated",
        "issue",
        None,
        json!({"count":n,"status":body.status}),
    )
    .await?;
    Ok(Json(json!({"updated":n})))
}

#[derive(Deserialize)]
struct MergeInput {
    target_issue_id: Uuid,
    source_issue_ids: Vec<Uuid>,
}
#[derive(Deserialize)]
struct SplitInput {
    event_ids: Vec<Uuid>,
    fingerprint: String,
    title: Option<String>,
}
#[derive(Deserialize)]
struct FingerprintInput {
    fingerprint: String,
}
async fn merge_issues(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<MergeInput>,
) -> Result<Json<Value>> {
    let user = crate::auth::authorize_issue(&state.db, &headers, body.target_issue_id).await?;
    crate::auth::require_admin(&user)?;
    if body.source_issue_ids.is_empty() {
        return Err(AppError::BadRequest("source_issue_ids required".into()));
    }
    let mut tx = state.db.begin().await?;
    let project_id = sqlx::query_scalar::<_, Uuid>("SELECT project_id FROM issues WHERE id=$1")
        .bind(body.target_issue_id)
        .fetch_one(&mut *tx)
        .await?;
    let moved =
        sqlx::query("UPDATE events SET issue_id=$1 WHERE issue_id=ANY($2) AND project_id=$3")
            .bind(body.target_issue_id)
            .bind(&body.source_issue_ids)
            .bind(project_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    sqlx::query("UPDATE issues SET event_count=(SELECT count(*) FROM events WHERE issue_id=$1),first_seen=(SELECT min(occurred_at) FROM events WHERE issue_id=$1),last_seen=(SELECT max(occurred_at) FROM events WHERE issue_id=$1) WHERE id=$1").bind(body.target_issue_id).execute(&mut*tx).await?;
    sqlx::query("DELETE FROM issues WHERE id=ANY($1) AND project_id=$2")
        .bind(&body.source_issue_ids)
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    crate::operations::audit(
        &state.db,
        &user,
        "issue.merged",
        "issue",
        Some(body.target_issue_id),
        json!({"moved":moved}),
    )
    .await?;
    Ok(Json(json!({"moved":moved})))
}
async fn split_issue(
    State(state): State<Arc<AppState>>,
    Path(issue_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<SplitInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let user = crate::auth::authorize_issue(&state.db, &headers, issue_id).await?;
    crate::auth::require_admin(&user)?;
    if body.event_ids.is_empty() || body.fingerprint.trim().is_empty() {
        return Err(AppError::BadRequest(
            "event_ids and fingerprint required".into(),
        ));
    }
    let mut tx = state.db.begin().await?;
    let (project_id, level, title) = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT project_id,level,title FROM issues WHERE id=$1",
    )
    .bind(issue_id)
    .fetch_one(&mut *tx)
    .await?;
    let new_id=sqlx::query_scalar::<_,Uuid>("INSERT INTO issues(project_id,fingerprint,title,level,event_count,first_seen,last_seen) SELECT $1,$2,$3,$4,count(*),min(occurred_at),max(occurred_at) FROM events WHERE issue_id=$5 AND id=ANY($6) HAVING count(*)>0 RETURNING id").bind(project_id).bind(body.fingerprint).bind(body.title.unwrap_or(title)).bind(level).bind(issue_id).bind(&body.event_ids).fetch_optional(&mut*tx).await?.ok_or(AppError::BadRequest("no matching events".into()))?;
    sqlx::query("UPDATE events SET issue_id=$1 WHERE issue_id=$2 AND id=ANY($3)")
        .bind(new_id)
        .bind(issue_id)
        .bind(&body.event_ids)
        .execute(&mut *tx)
        .await?;
    let remaining = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM events WHERE issue_id=$1")
        .bind(issue_id)
        .fetch_one(&mut *tx)
        .await?;
    if remaining == 0 {
        sqlx::query("DELETE FROM issues WHERE id=$1")
            .bind(issue_id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("UPDATE issues SET event_count=$2,first_seen=(SELECT min(occurred_at) FROM events WHERE issue_id=$1),last_seen=(SELECT max(occurred_at) FROM events WHERE issue_id=$1) WHERE id=$1")
            .bind(issue_id).bind(remaining).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"issue_id":new_id}))))
}
async fn change_fingerprint(
    State(state): State<Arc<AppState>>,
    Path(issue_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<FingerprintInput>,
) -> Result<StatusCode> {
    let user = crate::auth::authorize_issue(&state.db, &headers, issue_id).await?;
    crate::auth::require_admin(&user)?;
    if body.fingerprint.trim().is_empty() {
        return Err(AppError::BadRequest("fingerprint required".into()));
    }
    sqlx::query("UPDATE issues SET fingerprint=$2 WHERE id=$1")
        .bind(issue_id)
        .bind(body.fingerprint)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn download_envelope_item(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<axum::response::Response> {
    let row = sqlx::query_as::<_, (Uuid, String, Vec<u8>)>(
        "SELECT project_id,item_type,payload FROM envelope_items WHERE id=$1",
    )
    .bind(item_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    crate::auth::authorize_project(&state.db, &headers, row.0).await?;
    if row.1 != "attachment" {
        return Err(AppError::BadRequest("item is not an attachment".into()));
    }
    Ok((
        [
            ("content-type", "application/octet-stream"),
            ("content-disposition", "attachment; filename=attachment.bin"),
            ("cache-control", "private, no-store"),
        ],
        row.2,
    )
        .into_response())
}

async fn list_issues(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<IssueSummary>>> {
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let rows = sqlx::query_as::<_, IssueSummary>(
        r#"SELECT id, project_id, fingerprint, title, level, status, event_count, first_seen, last_seen
           FROM issues WHERE project_id = $1 AND ($2::text IS NULL OR status = $2)
             AND ($3::text IS NULL OR title ILIKE '%' || $3 || '%' OR fingerprint ILIKE '%' || $3 || '%')
             AND ($4::text IS NULL OR level=$4)
             AND ($5::text IS NULL OR assigned_to=$5)
             AND ($6::timestamptz IS NULL OR last_seen >= $6)
             AND ($7::timestamptz IS NULL OR last_seen < $7)
             AND EXISTS (SELECT 1 FROM events e LEFT JOIN services s ON s.id=e.service_id
                         WHERE e.issue_id=issues.id
                           AND ($8::text IS NULL OR s.name=$8)
                           AND ($9::text IS NULL OR e.environment=$9)
                           AND ($10::text IS NULL OR e.release=$10))
           ORDER BY CASE WHEN $11='events' THEN event_count END DESC, last_seen DESC
           LIMIT $12 OFFSET $13"#,
    ).bind(project_id).bind(q.status).bind(q.q).bind(q.level).bind(q.assigned_to)
    .bind(q.since).bind(q.before).bind(q.service).bind(q.environment).bind(q.release)
    .bind(q.sort.unwrap_or_else(||"recent".into()))
    .bind(q.limit.unwrap_or(50).clamp(1, 200)).bind(q.offset.unwrap_or(0).max(0))
    .fetch_all(&state.db).await?;
    Ok(Json(rows))
}

async fn project_details(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<ProjectDetails>> {
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let row = sqlx::query_as::<_, ProjectDetails>(
        r#"SELECT p.id, p.external_id, p.name, p.slug, p.created_at,
                  (SELECT max(e.occurred_at) FROM events e WHERE e.project_id=p.id) AS last_seen,
                  (SELECT count(*)::bigint FROM services s WHERE s.project_id=p.id) AS service_count,
                  (SELECT count(*)::bigint FROM issues i WHERE i.project_id=p.id) AS issue_count,
                  (SELECT count(*)::bigint FROM events e WHERE e.project_id=p.id) AS event_count,
                  (SELECT e.release FROM events e WHERE e.project_id=p.id AND e.release IS NOT NULL
                   ORDER BY e.occurred_at DESC LIMIT 1) AS latest_release
           FROM projects p WHERE p.id=$1"#,
    )
    .bind(project_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(row))
}

async fn list_services(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Vec<ServiceView>>> {
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let rows = sqlx::query_as::<_, ServiceView>(
        r#"SELECT s.id, s.name, s.environment, s.latest_release, s.sdk_name, s.sdk_version,
                  s.runtime, s.event_count, s.log_count,
                  (SELECT count(DISTINCT e.issue_id)::bigint FROM events e WHERE e.service_id=s.id) AS issue_count,
                  s.first_seen, s.last_seen
           FROM services s WHERE s.project_id=$1 ORDER BY s.last_seen DESC"#,
    )
    .bind(project_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

async fn list_project_logs(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Query(q): Query<TelemetryQuery>,
) -> Result<Json<Vec<LogView>>> {
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let rows = sqlx::query_as::<_, LogView>(
        r#"SELECT l.id, s.name AS service, s.environment, l.occurred_at, l.level,
                  l.body, l.trace_id, l.attributes
           FROM logs l LEFT JOIN services s ON s.id=l.service_id
           WHERE l.project_id=$1
             AND ($2::text IS NULL OR l.level=$2)
             AND ($3::text IS NULL OR s.name=$3)
             AND ($4::text IS NULL OR s.environment=$4)
             AND ($5::text IS NULL OR l.trace_id=$5)
             AND ($6::text IS NULL OR l.body ILIKE '%' || $6 || '%')
             AND ($7::timestamptz IS NULL OR l.occurred_at >= $7)
             AND ($8::timestamptz IS NULL OR l.occurred_at < $8)
           ORDER BY l.occurred_at DESC LIMIT $9"#,
    )
    .bind(project_id)
    .bind(q.level)
    .bind(q.service)
    .bind(q.environment)
    .bind(q.trace_id)
    .bind(q.q)
    .bind(q.since)
    .bind(q.before)
    .bind(q.limit.unwrap_or(100).clamp(1, 500))
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

async fn list_project_events(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Query(q): Query<TelemetryQuery>,
) -> Result<Json<Vec<EventStreamView>>> {
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let rows = sqlx::query_as::<_, EventStreamView>(
        r#"SELECT e.id, e.issue_id, s.name AS service, e.level, e.message, e.environment,
                  e.release, e.trace_id, e.occurred_at
           FROM events e LEFT JOIN services s ON s.id=e.service_id
           WHERE e.project_id=$1
             AND ($2::text IS NULL OR e.level=$2)
             AND ($3::text IS NULL OR s.name=$3)
             AND ($4::text IS NULL OR e.environment=$4)
             AND ($5::text IS NULL OR e.release=$5)
             AND ($6::text IS NULL OR e.trace_id=$6)
             AND ($7::text IS NULL OR e.message ILIKE '%' || $7 || '%')
             AND ($8::timestamptz IS NULL OR e.occurred_at >= $8)
             AND ($9::timestamptz IS NULL OR e.occurred_at < $9)
           ORDER BY e.occurred_at DESC LIMIT $10"#,
    )
    .bind(project_id)
    .bind(q.level)
    .bind(q.service)
    .bind(q.environment)
    .bind(q.release)
    .bind(q.trace_id)
    .bind(q.q)
    .bind(q.since)
    .bind(q.before)
    .bind(q.limit.unwrap_or(100).clamp(1, 500))
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

async fn list_debug_files(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let rows=sqlx::query_as::<_,(Uuid,String,String,Option<String>,String,String,Option<String>,chrono::DateTime<chrono::Utc>)>("SELECT d.id,d.kind,d.name,r.version,d.checksum,d.status,d.error,d.created_at FROM debug_files d LEFT JOIN releases r ON r.id=d.release_id WHERE d.project_id=$1 ORDER BY d.created_at DESC").bind(project_id).fetch_all(&state.db).await?;
    Ok(Json(json!(rows.into_iter().map(|r|json!({"id":r.0,"kind":r.1,"name":r.2,"release":r.3,"checksum":r.4,"status":r.5,"error":r.6,"created_at":r.7})).collect::<Vec<_>>())))
}
async fn upload_debug_file(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>)> {
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    let value = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let kind = value("x-debug-file-kind")
        .ok_or_else(|| AppError::BadRequest("x-debug-file-kind is required".into()))?;
    let name = value("x-debug-file-name")
        .ok_or_else(|| AppError::BadRequest("x-debug-file-name is required".into()))?;
    let release =
        value("x-release").ok_or_else(|| AppError::BadRequest("x-release is required".into()))?;
    crate::symbols::validate_debug_file(&kind, &body)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let checksum = format!("{:x}", Sha256::digest(&body));
    let release_id=sqlx::query_scalar::<_,Uuid>("INSERT INTO releases(project_id,version) VALUES($1,$2) ON CONFLICT(project_id,version) DO UPDATE SET version=EXCLUDED.version RETURNING id").bind(project_id).bind(&release).fetch_one(&state.db).await?;
    let id=sqlx::query_scalar::<_,Uuid>("INSERT INTO debug_files(project_id,release_id,kind,name,debug_id,checksum,payload) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(project_id,checksum) DO UPDATE SET name=EXCLUDED.name RETURNING id").bind(project_id).bind(release_id).bind(&kind).bind(&name).bind(value("x-debug-id")).bind(&checksum).bind(body.as_ref()).fetch_one(&state.db).await?;
    sqlx::query("UPDATE symbolication_jobs j SET status='pending',available_at=now(),last_error=NULL FROM events e WHERE j.event_id=e.id AND e.project_id=$1 AND e.release=$2").bind(project_id).bind(&release).execute(&state.db).await?;
    crate::operations::audit(
        &state.db,
        &user,
        "debug_file.uploaded",
        "debug_file",
        Some(id),
        json!({"kind":kind,"release":release}),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id":id,"checksum":checksum,"status":"ready"})),
    ))
}
async fn delete_debug_file(
    State(state): State<Arc<AppState>>,
    Path(file_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let project_id =
        sqlx::query_scalar::<_, Uuid>("SELECT project_id FROM debug_files WHERE id=$1")
            .bind(file_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NotFound)?;
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    sqlx::query("DELETE FROM debug_files WHERE id=$1")
        .bind(file_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn reprocess_event(
    State(state): State<Arc<AppState>>,
    Path(event_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let project_id = sqlx::query_scalar::<_, Uuid>("SELECT project_id FROM events WHERE id=$1")
        .bind(event_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    sqlx::query("INSERT INTO symbolication_jobs(event_id,status,available_at) VALUES($1,'pending',now()) ON CONFLICT(event_id) DO UPDATE SET status='pending',available_at=now(),last_error=NULL").bind(event_id).execute(&state.db).await?;
    sqlx::query("UPDATE events SET symbolication_status='pending' WHERE id=$1")
        .bind(event_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
struct CreateReleaseInput {
    version: String,
    description: Option<String>,
}
#[derive(Deserialize)]
struct DeploymentInput {
    environment: String,
    name: Option<String>,
    url: Option<String>,
}
#[derive(Deserialize)]
struct CompareReleaseQuery {
    base: String,
    head: String,
}
async fn list_releases(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let rows=sqlx::query_as::<_,(Uuid,String,Option<String>,i64,Option<chrono::DateTime<chrono::Utc>>,Option<chrono::DateTime<chrono::Utc>>,chrono::DateTime<chrono::Utc>)>("SELECT id,version,description,event_count,first_event_at,last_event_at,created_at FROM releases WHERE project_id=$1 ORDER BY COALESCE(last_event_at,created_at) DESC").bind(project_id).fetch_all(&state.db).await?;
    Ok(Json(json!(rows.into_iter().map(|r|json!({"id":r.0,"version":r.1,"description":r.2,"event_count":r.3,"first_event_at":r.4,"last_event_at":r.5,"created_at":r.6})).collect::<Vec<_>>())))
}
async fn create_release(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<CreateReleaseInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    if body.version.trim().is_empty() {
        return Err(AppError::BadRequest("version required".into()));
    }
    let id=sqlx::query_scalar::<_,Uuid>("INSERT INTO releases(project_id,version,description) VALUES($1,$2,$3) ON CONFLICT(project_id,version) DO UPDATE SET description=COALESCE(EXCLUDED.description,releases.description) RETURNING id").bind(project_id).bind(body.version.trim()).bind(body.description).fetch_one(&state.db).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id":id,"version":body.version})),
    ))
}
async fn get_release(
    State(state): State<Arc<AppState>>,
    Path(release_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            String,
            Option<String>,
            i64,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        "SELECT id,project_id,version,description,event_count,created_at FROM releases WHERE id=$1",
    )
    .bind(release_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    crate::auth::authorize_project(&state.db, &headers, row.1).await?;
    let deployments=sqlx::query_as::<_,(Uuid,String,Option<String>,Option<String>,chrono::DateTime<chrono::Utc>)>("SELECT id,environment,name,url,deployed_at FROM deployments WHERE release_id=$1 ORDER BY deployed_at DESC").bind(release_id).fetch_all(&state.db).await?;
    Ok(Json(
        json!({"id":row.0,"project_id":row.1,"version":row.2,"description":row.3,"event_count":row.4,"created_at":row.5,"deployments":deployments.into_iter().map(|d|json!({"id":d.0,"environment":d.1,"name":d.2,"url":d.3,"deployed_at":d.4})).collect::<Vec<_>>()}),
    ))
}
async fn create_deployment(
    State(state): State<Arc<AppState>>,
    Path(release_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<DeploymentInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let project_id = sqlx::query_scalar::<_, Uuid>("SELECT project_id FROM releases WHERE id=$1")
        .bind(release_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    let id=sqlx::query_scalar::<_,Uuid>("INSERT INTO deployments(release_id,environment,name,url,deployed_by) VALUES($1,$2,$3,$4,$5) RETURNING id").bind(release_id).bind(body.environment).bind(body.name).bind(body.url).bind(user.id).fetch_one(&state.db).await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id}))))
}
async fn release_health(
    State(state): State<Arc<AppState>>,
    Path(release_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let (project_id, version) =
        sqlx::query_as::<_, (Uuid, String)>("SELECT project_id,version FROM releases WHERE id=$1")
            .bind(release_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NotFound)?;
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let points=sqlx::query_as::<_,(chrono::DateTime<chrono::Utc>,String,i64,i64)>("SELECT date_trunc('hour',occurred_at),COALESCE(environment,'default'),count(*)::bigint,count(DISTINCT issue_id)::bigint FROM events WHERE project_id=$1 AND release=$2 GROUP BY 1,2 ORDER BY 1").bind(project_id).bind(version).fetch_all(&state.db).await?;
    Ok(Json(json!(
        points
            .into_iter()
            .map(|p| json!({"bucket":p.0,"environment":p.1,"events":p.2,"issues":p.3}))
            .collect::<Vec<_>>()
    )))
}
async fn compare_releases(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Query(q): Query<CompareReleaseQuery>,
) -> Result<Json<Value>> {
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let rows=sqlx::query_as::<_,(Uuid,String,String,bool,bool)>(r#"SELECT i.id,i.title,i.status,
EXISTS(SELECT 1 FROM events e WHERE e.issue_id=i.id AND e.release=$2) base,
EXISTS(SELECT 1 FROM events e WHERE e.issue_id=i.id AND e.release=$3) head
FROM issues i WHERE i.project_id=$1 AND (EXISTS(SELECT 1 FROM events e WHERE e.issue_id=i.id AND e.release=$2) OR EXISTS(SELECT 1 FROM events e WHERE e.issue_id=i.id AND e.release=$3))"#).bind(project_id).bind(&q.base).bind(&q.head).fetch_all(&state.db).await?;
    Ok(Json(
        json!({"base":q.base,"head":q.head,"issues":rows.into_iter().map(|r|json!({"id":r.0,"title":r.1,"status":r.2,"classification":match(r.3,r.4){(false,true)=>"new",(true,true)=>"ongoing",(true,false)=>"fixed",_=>"unknown"}})).collect::<Vec<_>>()}),
    ))
}

#[derive(Deserialize)]
struct CreateWebhookInput {
    name: String,
    url: String,
}
#[derive(Deserialize)]
struct CreateScheduleInput {
    name: String,
    timezone: Option<String>,
    members: Vec<Uuid>,
}
#[derive(Deserialize)]
struct CreatePolicyInput {
    name: String,
    steps: Vec<CreateEscalationStepInput>,
}
#[derive(Deserialize)]
struct CreateEscalationStepInput {
    delay_seconds: i32,
    channel: String,
    target: Option<String>,
    schedule_id: Option<Uuid>,
}

async fn list_webhooks(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let rows=sqlx::query_as::<_,(Uuid,String,String,bool,chrono::DateTime<chrono::Utc>,i64)>(r#"SELECT w.id,w.name,w.url,w.enabled,w.created_at,count(d.id)::bigint FROM webhook_endpoints w LEFT JOIN webhook_deliveries d ON d.endpoint_id=w.id WHERE w.project_id=$1 GROUP BY w.id ORDER BY w.created_at DESC"#).bind(project_id).fetch_all(&state.db).await?;
    Ok(Json(json!(rows.into_iter().map(|r|json!({"id":r.0,"name":r.1,"url":r.2,"enabled":r.3,"created_at":r.4,"delivery_count":r.5})).collect::<Vec<_>>())))
}
async fn create_webhook(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<CreateWebhookInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    let parsed = reqwest::Url::parse(body.url.trim())
        .map_err(|_| AppError::BadRequest("valid webhook URL required".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") || body.name.trim().is_empty() {
        return Err(AppError::BadRequest(
            "name and an HTTP(S) URL are required".into(),
        ));
    }
    let secret = format!(
        "whsec_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    let id=sqlx::query_scalar::<_,Uuid>("INSERT INTO webhook_endpoints(project_id,name,url,signing_secret) VALUES($1,$2,$3,$4) RETURNING id").bind(project_id).bind(body.name.trim()).bind(parsed.as_str()).bind(&secret).fetch_one(&state.db).await?;
    crate::operations::audit(
        &state.db,
        &user,
        "webhook.created",
        "webhook",
        Some(id),
        json!({"project_id":project_id}),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id":id,"signing_secret":secret,"shown_once":true})),
    ))
}
async fn delete_webhook(
    State(state): State<Arc<AppState>>,
    Path(endpoint_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let project_id =
        sqlx::query_scalar::<_, Uuid>("SELECT project_id FROM webhook_endpoints WHERE id=$1")
            .bind(endpoint_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NotFound)?;
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    sqlx::query("DELETE FROM webhook_endpoints WHERE id=$1")
        .bind(endpoint_id)
        .execute(&state.db)
        .await?;
    crate::operations::audit(
        &state.db,
        &user,
        "webhook.deleted",
        "webhook",
        Some(endpoint_id),
        json!({}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn test_webhook(
    State(state): State<Arc<AppState>>,
    Path(endpoint_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<Value>)> {
    let row=sqlx::query_as::<_,(Uuid,Uuid)>("SELECT w.project_id,r.id FROM webhook_endpoints w JOIN alert_rules r ON r.project_id=w.project_id WHERE w.id=$1 ORDER BY r.created_at LIMIT 1").bind(endpoint_id).fetch_optional(&state.db).await?.ok_or_else(||AppError::BadRequest("create an alert rule before testing a webhook".into()))?;
    let user = crate::auth::authorize_project(&state.db, &headers, row.0).await?;
    crate::auth::require_admin(&user)?;
    let id=sqlx::query_scalar::<_,Uuid>("INSERT INTO notification_outbox(rule_id,channel,target,payload) VALUES($1,'webhook',$2,$3) RETURNING id").bind(row.1).bind(endpoint_id.to_string()).bind(json!({"rule_name":"Webhook test","project_id":row.0,"test":true})).fetch_one(&state.db).await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"notification_id":id}))))
}
async fn list_schedules(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let rows=sqlx::query_as::<_,(Uuid,String,String,chrono::DateTime<chrono::Utc>,Value)>(r#"SELECT s.id,s.name,s.timezone,s.created_at,COALESCE(jsonb_agg(jsonb_build_object('user_id',u.id,'email',u.email,'display_name',u.display_name,'rotation_order',m.rotation_order) ORDER BY m.rotation_order) FILTER(WHERE u.id IS NOT NULL),'[]') FROM on_call_schedules s LEFT JOIN on_call_members m ON m.schedule_id=s.id LEFT JOIN users u ON u.id=m.user_id WHERE s.project_id=$1 GROUP BY s.id ORDER BY s.created_at DESC"#).bind(project_id).fetch_all(&state.db).await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|r| json!({"id":r.0,"name":r.1,"timezone":r.2,"created_at":r.3,"members":r.4}))
            .collect::<Vec<_>>()
    )))
}
async fn create_schedule(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<CreateScheduleInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    if body.name.trim().is_empty() || body.members.is_empty() {
        return Err(AppError::BadRequest(
            "schedule name and at least one member are required".into(),
        ));
    }
    let mut tx = state.db.begin().await?;
    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO on_call_schedules(project_id,name,timezone) VALUES($1,$2,$3) RETURNING id",
    )
    .bind(project_id)
    .bind(body.name.trim())
    .bind(body.timezone.unwrap_or_else(|| "UTC".into()))
    .fetch_one(&mut *tx)
    .await?;
    for (order, member) in body.members.into_iter().enumerate() {
        let valid=sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM users u JOIN projects p ON p.organization_id=u.organization_id WHERE u.id=$1 AND p.id=$2 AND u.active)").bind(member).bind(project_id).fetch_one(&mut *tx).await?;
        if !valid {
            return Err(AppError::BadRequest(
                "schedule member is not an active project organization user".into(),
            ));
        }
        sqlx::query(
            "INSERT INTO on_call_members(schedule_id,user_id,rotation_order) VALUES($1,$2,$3)",
        )
        .bind(id)
        .bind(member)
        .bind(order as i32)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id}))))
}
async fn list_policies(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let rows=sqlx::query_as::<_,(Uuid,String,chrono::DateTime<chrono::Utc>,Value)>(r#"SELECT p.id,p.name,p.created_at,COALESCE(jsonb_agg(jsonb_build_object('order',s.step_order,'delay_seconds',s.delay_seconds,'channel',s.channel,'target',s.target,'schedule_id',s.schedule_id) ORDER BY s.step_order) FILTER(WHERE s.id IS NOT NULL),'[]') FROM escalation_policies p LEFT JOIN escalation_steps s ON s.policy_id=p.id WHERE p.project_id=$1 GROUP BY p.id ORDER BY p.created_at DESC"#).bind(project_id).fetch_all(&state.db).await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|r| json!({"id":r.0,"name":r.1,"created_at":r.2,"steps":r.3}))
            .collect::<Vec<_>>()
    )))
}
async fn create_policy(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<CreatePolicyInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    if body.name.trim().is_empty() || body.steps.is_empty() {
        return Err(AppError::BadRequest(
            "policy name and steps are required".into(),
        ));
    }
    let mut tx = state.db.begin().await?;
    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO escalation_policies(project_id,name) VALUES($1,$2) RETURNING id",
    )
    .bind(project_id)
    .bind(body.name.trim())
    .fetch_one(&mut *tx)
    .await?;
    for (order, step) in body.steps.into_iter().enumerate() {
        if !matches!(
            step.channel.as_str(),
            "email" | "telegram" | "voice_call" | "webhook"
        ) || (step.target.as_deref().unwrap_or("").is_empty() && step.schedule_id.is_none())
        {
            return Err(AppError::BadRequest(
                "each policy step needs a supported channel and target or schedule".into(),
            ));
        }
        if let Some(schedule) = step.schedule_id {
            let valid = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM on_call_schedules WHERE id=$1 AND project_id=$2)",
            )
            .bind(schedule)
            .bind(project_id)
            .fetch_one(&mut *tx)
            .await?;
            if !valid {
                return Err(AppError::BadRequest(
                    "schedule does not belong to project".into(),
                ));
            }
        }
        sqlx::query("INSERT INTO escalation_steps(policy_id,step_order,delay_seconds,channel,target,schedule_id) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(order as i32).bind(step.delay_seconds.max(0)).bind(step.channel).bind(step.target).bind(step.schedule_id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id}))))
}

async fn project_overview(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<ProjectOverview>> {
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let (total_events, events_24h) = sqlx::query_as::<_, (i64, i64)>(
        r#"SELECT count(*)::bigint,
                  count(*) FILTER (WHERE occurred_at >= now() - interval '24 hours')::bigint
           FROM events WHERE project_id = $1"#,
    )
    .bind(project_id)
    .fetch_one(&state.db)
    .await?;
    let (unresolved_issues, resolved_issues) = sqlx::query_as::<_, (i64, i64)>(
        r#"SELECT count(*) FILTER (WHERE status = 'unresolved')::bigint,
                  count(*) FILTER (WHERE status = 'resolved')::bigint
           FROM issues WHERE project_id = $1"#,
    )
    .bind(project_id)
    .fetch_one(&state.db)
    .await?;
    let trends = sqlx::query_as::<_, TrendPoint>(
        r#"WITH buckets AS (
             SELECT generate_series(
               date_trunc('hour', now() - interval '23 hours'),
               date_trunc('hour', now()), interval '1 hour'
             ) AS bucket
           )
           SELECT b.bucket, count(e.id)::bigint AS count
           FROM buckets b LEFT JOIN events e
             ON e.project_id = $1 AND e.occurred_at >= b.bucket
            AND e.occurred_at < b.bucket + interval '1 hour'
           GROUP BY b.bucket ORDER BY b.bucket"#,
    )
    .bind(project_id)
    .fetch_all(&state.db)
    .await?;
    let levels = sqlx::query_as::<_, LevelCount>(
        "SELECT level, count(*)::bigint AS count FROM events WHERE project_id = $1 GROUP BY level ORDER BY count DESC",
    )
    .bind(project_id)
    .fetch_all(&state.db)
    .await?;
    let recent_items = sqlx::query_as::<_, EnvelopeItemSummary>(
        r#"SELECT id, item_type, octet_length(payload)::bigint AS size_bytes, received_at
           FROM envelope_items WHERE project_id = $1 ORDER BY received_at DESC LIMIT 8"#,
    )
    .bind(project_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(ProjectOverview {
        total_events,
        events_24h,
        unresolved_issues,
        resolved_issues,
        trends,
        levels,
        recent_items,
    }))
}

async fn get_issue(
    State(state): State<Arc<AppState>>,
    Path(issue_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<IssueSummary>> {
    crate::auth::authorize_issue(&state.db, &headers, issue_id).await?;
    let row = sqlx::query_as::<_, IssueSummary>("SELECT id, project_id, fingerprint, title, level, status, event_count, first_seen, last_seen FROM issues WHERE id = $1")
        .bind(issue_id).fetch_optional(&state.db).await?.ok_or(AppError::NotFound)?;
    Ok(Json(row))
}

#[derive(Deserialize)]
struct UpdateIssue {
    status: String,
    #[serde(default)]
    assigned_to: Option<String>,
}

async fn update_issue(
    State(state): State<Arc<AppState>>,
    Path(issue_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<UpdateIssue>,
) -> Result<StatusCode> {
    let user = crate::auth::authorize_issue(&state.db, &headers, issue_id).await?;
    crate::auth::require_admin(&user)?;
    alert::mark_issue_status(&state.db, issue_id, &body.status).await?;
    if body.status == "resolved" {
        alert::enqueue_recovery_alerts(&state.db, issue_id).await?;
    }
    if let Some(assigned_to) = body.assigned_to {
        sqlx::query("UPDATE issues SET assigned_to=$2 WHERE id=$1")
            .bind(issue_id)
            .bind(assigned_to)
            .execute(&state.db)
            .await?;
    }
    crate::operations::audit(
        &state.db,
        &user,
        "issue.status_changed",
        "issue",
        Some(issue_id),
        json!({"status":body.status}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn fix_issue(
    State(state): State<Arc<AppState>>,
    Path(issue_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let user = crate::auth::authorize_issue(&state.db, &headers, issue_id).await?;
    crate::auth::require_admin(&user)?;
    let issue = sqlx::query_as::<_, IssueSummary>(
        "SELECT id, project_id, fingerprint, title, level, status, event_count, first_seen, last_seen FROM issues WHERE id=$1",
    )
    .bind(issue_id).fetch_optional(&state.db).await?.ok_or(AppError::NotFound)?;
    let events = sqlx::query_as::<_, StoredEvent>(
        r#"SELECT id, project_id, issue_id, level, message, environment, release, tags,
                  contexts, exception, symbolicated_exception, symbolication_status, received_at, occurred_at
           FROM events WHERE issue_id=$1 ORDER BY occurred_at DESC LIMIT 5"#,
    )
    .bind(issue_id)
    .fetch_all(&state.db)
    .await?;
    let context = json!({
        "issue": issue,
        "recent_events": events,
        "suggested_action": "Inspect the exception and trace context, reproduce on the latest release, then deploy a fix and monitor for recurrence."
    });
    sqlx::query(
        "UPDATE issues SET status='in_progress', fix_context=$2, fixed_at=NULL WHERE id=$1",
    )
    .bind(issue_id)
    .bind(&context)
    .execute(&state.db)
    .await?;
    crate::operations::audit(
        &state.db,
        &user,
        "issue.fix_started",
        "issue",
        Some(issue_id),
        json!({}),
    )
    .await?;
    Ok(Json(context))
}

async fn list_events(
    State(state): State<Arc<AppState>>,
    Path(issue_id): Path<Uuid>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<StoredEvent>>> {
    crate::auth::authorize_issue(&state.db, &headers, issue_id).await?;
    let rows = sqlx::query_as::<_, StoredEvent>(
        r#"SELECT id, project_id, issue_id, level, message, environment, release, tags, contexts, exception, symbolicated_exception, symbolication_status, received_at, occurred_at
           FROM events WHERE issue_id = $1 ORDER BY occurred_at DESC LIMIT $2 OFFSET $3"#,
    ).bind(issue_id).bind(q.limit.unwrap_or(50).clamp(1, 200)).bind(q.offset.unwrap_or(0).max(0))
    .fetch_all(&state.db).await?;
    Ok(Json(rows))
}

async fn create_rule(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Json(rule): Json<CreateAlertRule>,
) -> Result<(StatusCode, Json<AlertRuleView>)> {
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    if !matches!(
        rule.channel.as_str(),
        "email" | "telegram" | "voice_call" | "webhook"
    ) {
        return Err(AppError::BadRequest(
            "channel must be email, telegram, voice_call, or webhook".into(),
        ));
    }
    if let Some(policy_id) = rule.escalation_policy_id {
        let valid = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM escalation_policies WHERE id=$1 AND project_id=$2)",
        )
        .bind(policy_id)
        .bind(project_id)
        .fetch_one(&state.db)
        .await?;
        if !valid {
            return Err(AppError::BadRequest(
                "escalation policy does not belong to project".into(),
            ));
        }
    }
    let row = sqlx::query_as::<_, AlertRuleView>(
        r#"INSERT INTO alert_rules (project_id, name, level, message_contains, environment, cooldown_seconds, channel, target,threshold_count,window_seconds,notify_recovery,escalation_policy_id)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
           RETURNING id, project_id, name, level, message_contains, environment, cooldown_seconds, channel, target, enabled, escalation_policy_id"#,
    ).bind(project_id).bind(rule.name).bind(rule.level).bind(rule.message_contains).bind(rule.environment)
    .bind(rule.cooldown_seconds.unwrap_or(300).max(0)).bind(rule.channel).bind(rule.target)
    .bind(rule.threshold_count).bind(rule.window_seconds).bind(rule.notify_recovery).bind(rule.escalation_policy_id)
    .fetch_one(&state.db).await?;
    crate::operations::audit(
        &state.db,
        &user,
        "alert_rule.created",
        "alert_rule",
        Some(row.id),
        json!({"project_id":project_id}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(row)))
}

async fn list_rules(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Vec<AlertRuleView>>> {
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let rows = sqlx::query_as::<_, AlertRuleView>("SELECT id, project_id, name, level, message_contains, environment, cooldown_seconds, channel, target, enabled, escalation_policy_id FROM alert_rules WHERE project_id = $1 ORDER BY created_at DESC")
        .bind(project_id).fetch_all(&state.db).await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
struct UpdateAlertRule {
    name: Option<String>,
    level: Option<String>,
    message_contains: Option<String>,
    environment: Option<String>,
    cooldown_seconds: Option<i32>,
    channel: Option<String>,
    target: Option<String>,
    enabled: Option<bool>,
    threshold_count: Option<i32>,
    window_seconds: Option<i32>,
    notify_recovery: Option<bool>,
    escalation_policy_id: Option<Uuid>,
}

async fn update_rule(
    State(state): State<Arc<AppState>>,
    Path(rule_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<UpdateAlertRule>,
) -> Result<Json<AlertRuleView>> {
    let project_id =
        sqlx::query_scalar::<_, Uuid>("SELECT project_id FROM alert_rules WHERE id=$1")
            .bind(rule_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NotFound)?;
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    if body
        .channel
        .as_deref()
        .is_some_and(|c| !matches!(c, "email" | "telegram" | "voice_call" | "webhook"))
    {
        return Err(AppError::BadRequest(
            "channel must be email, telegram, voice_call, or webhook".into(),
        ));
    }
    if let Some(policy_id) = body.escalation_policy_id {
        let valid = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM escalation_policies WHERE id=$1 AND project_id=$2)",
        )
        .bind(policy_id)
        .bind(project_id)
        .fetch_one(&state.db)
        .await?;
        if !valid {
            return Err(AppError::BadRequest(
                "escalation policy does not belong to project".into(),
            ));
        }
    }
    let row=sqlx::query_as::<_,AlertRuleView>("UPDATE alert_rules SET name=COALESCE($2,name),level=COALESCE($3,level),message_contains=COALESCE($4,message_contains),environment=COALESCE($5,environment),cooldown_seconds=COALESCE($6,cooldown_seconds),channel=COALESCE($7,channel),target=COALESCE($8,target),enabled=COALESCE($9,enabled),threshold_count=COALESCE($10,threshold_count),window_seconds=COALESCE($11,window_seconds),notify_recovery=COALESCE($12,notify_recovery),escalation_policy_id=COALESCE($13,escalation_policy_id) WHERE id=$1 RETURNING id,project_id,name,level,message_contains,environment,cooldown_seconds,channel,target,enabled,escalation_policy_id").bind(rule_id).bind(body.name).bind(body.level).bind(body.message_contains).bind(body.environment).bind(body.cooldown_seconds).bind(body.channel).bind(body.target).bind(body.enabled).bind(body.threshold_count).bind(body.window_seconds).bind(body.notify_recovery).bind(body.escalation_policy_id).fetch_one(&state.db).await?;
    crate::operations::audit(
        &state.db,
        &user,
        "alert_rule.updated",
        "alert_rule",
        Some(rule_id),
        json!({"enabled":row.enabled}),
    )
    .await?;
    Ok(Json(row))
}
async fn delete_rule(
    State(state): State<Arc<AppState>>,
    Path(rule_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let project_id =
        sqlx::query_scalar::<_, Uuid>("SELECT project_id FROM alert_rules WHERE id=$1")
            .bind(rule_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NotFound)?;
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    sqlx::query("DELETE FROM alert_rules WHERE id=$1")
        .bind(rule_id)
        .execute(&state.db)
        .await?;
    crate::operations::audit(
        &state.db,
        &user,
        "alert_rule.deleted",
        "alert_rule",
        Some(rule_id),
        json!({}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn test_rule(
    State(state): State<Arc<AppState>>,
    Path(rule_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<Value>)> {
    let row = sqlx::query_as::<_, (Uuid, String, String, String)>(
        "SELECT project_id,name,channel,target FROM alert_rules WHERE id=$1",
    )
    .bind(rule_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let user = crate::auth::authorize_project(&state.db, &headers, row.0).await?;
    crate::auth::require_admin(&user)?;
    let id=sqlx::query_scalar::<_,Uuid>("INSERT INTO notification_outbox(rule_id,channel,target,payload) VALUES($1,$2,$3,$4) RETURNING id").bind(rule_id).bind(row.2).bind(row.3).bind(json!({"rule_name":row.1,"project_id":row.0,"issue_id":null,"test":true})).fetch_one(&state.db).await?;
    crate::operations::audit(
        &state.db,
        &user,
        "alert_rule.test_sent",
        "alert_rule",
        Some(rule_id),
        json!({"notification_id":id}),
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"notification_id":id}))))
}
async fn check_rule_channel(
    State(state): State<Arc<AppState>>,
    Path(rule_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let row = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT project_id,channel,target FROM alert_rules WHERE id=$1",
    )
    .bind(rule_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    crate::auth::authorize_project(&state.db, &headers, row.0).await?;
    let error = crate::notification::channel_configuration_error(&state, &row.1, &row.2);
    Ok(Json(json!({"ok":error.is_none(),"error":error})))
}
async fn list_notifications(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>> {
    crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    let rows=sqlx::query_as::<_,(Uuid,Uuid,String,String,String,i32,Option<String>,chrono::DateTime<chrono::Utc>,Option<chrono::DateTime<chrono::Utc>>)>("SELECT n.id,n.rule_id,r.name,n.channel,n.status,n.attempts,n.last_error,n.created_at,n.sent_at FROM notification_outbox n JOIN alert_rules r ON r.id=n.rule_id WHERE r.project_id=$1 ORDER BY n.created_at DESC LIMIT $2").bind(project_id).bind(q.limit.unwrap_or(100).clamp(1,500)).fetch_all(&state.db).await?;
    Ok(Json(json!(rows.into_iter().map(|r|json!({"id":r.0,"rule_id":r.1,"rule_name":r.2,"channel":r.3,"status":r.4,"attempts":r.5,"last_error":r.6,"created_at":r.7,"sent_at":r.8})).collect::<Vec<_>>())))
}
async fn retry_notification(
    State(state): State<Arc<AppState>>,
    Path(notification_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let project_id=sqlx::query_scalar::<_,Uuid>("SELECT r.project_id FROM notification_outbox n JOIN alert_rules r ON r.id=n.rule_id WHERE n.id=$1").bind(notification_id).fetch_optional(&state.db).await?.ok_or(AppError::NotFound)?;
    let user = crate::auth::authorize_project(&state.db, &headers, project_id).await?;
    crate::auth::require_admin(&user)?;
    sqlx::query("UPDATE notification_outbox SET status='pending',attempts=0,available_at=now(),claimed_at=NULL,last_error=NULL,sent_at=NULL WHERE id=$1").bind(notification_id).execute(&state.db).await?;
    crate::operations::audit(
        &state.db,
        &user,
        "notification.retried",
        "notification",
        Some(notification_id),
        json!({}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_audit_logs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<AuditLogView>>> {
    let user = crate::auth::authenticate(&state.db, &headers).await?;
    let rows = sqlx::query_as::<_, AuditLogView>(
        r#"SELECT a.id, u.email AS actor_email, a.action, a.resource_type,
                  a.resource_id, a.metadata, a.occurred_at
           FROM audit_logs a
           LEFT JOIN users u ON u.id=a.actor_user_id
           WHERE a.organization_id=(SELECT organization_id FROM users WHERE id=$1)
           ORDER BY a.occurred_at DESC LIMIT $2 OFFSET $3"#,
    )
    .bind(user.id)
    .bind(q.limit.unwrap_or(50).clamp(1, 200))
    .bind(q.offset.unwrap_or(0).max(0))
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

async fn runtime_config(State(state): State<Arc<AppState>>) -> Json<RuntimeConfigView> {
    Json(RuntimeConfigView {
        environment: state.settings.environment.clone(),
        ingest_rate_limit_per_minute: state.settings.ingest_rate_limit_per_minute,
        retention_days: state.settings.retention_days,
        secure_cookies: state.settings.secure_cookies,
    })
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};
    use serde_json::json;

    use super::*;

    #[test]
    fn fingerprint_normalizes_case_and_whitespace() {
        let make = |message: &str| IngestEvent {
            event_id: None,
            level: "error".into(),
            message: message.into(),
            fingerprint: json!(null),
            timestamp: None,
            environment: None,
            release: None,
            sdk: json!({}),
            server_name: None,
            platform: None,
            tags: json!({}),
            contexts: json!({}),
            exception: json!({ "type": "Timeout" }),
        };
        assert_eq!(
            fingerprint(&make("Payment  TIMEOUT")),
            fingerprint(&make("payment timeout"))
        );
    }

    #[test]
    fn sdk_fingerprint_overrides_dynamic_message() {
        let make = |message: &str| IngestEvent {
            event_id: None,
            level: "error".into(),
            message: message.into(),
            fingerprint: json!(["payment-timeout"]),
            timestamp: None,
            environment: None,
            release: None,
            sdk: json!({}),
            server_name: None,
            platform: None,
            tags: json!({}),
            contexts: json!({}),
            exception: json!({"type":"Timeout"}),
        };
        assert_eq!(fingerprint(&make("order 1")), fingerprint(&make("order 2")));
    }

    #[test]
    fn stable_stack_frames_prevent_dynamic_message_fragmentation() {
        let make = |message: &str| IngestEvent {
            event_id: None,
            level: "error".into(),
            message: message.into(),
            fingerprint: json!(null),
            timestamp: None,
            environment: None,
            release: None,
            sdk: json!({}),
            server_name: None,
            platform: None,
            tags: json!({}),
            contexts: json!({}),
            exception: json!({"values":[{"type":"Timeout","stacktrace":{"frames":[{"module":"checkout","function":"charge","filename":"src/payment.rs","lineno":42,"in_app":true}]}}]}),
        };
        assert_eq!(fingerprint(&make("order 1")), fingerprint(&make("order 2")));
    }

    #[test]
    fn ingest_auth_accepts_header_and_rejects_wrong_key() {
        let mut headers = HeaderMap::new();
        headers.insert("x-sentry-auth", HeaderValue::from_static("secret"));
        assert!(authorize_ingest(&headers, "secret").is_ok());
        assert!(matches!(
            authorize_ingest(&headers, "wrong"),
            Err(AppError::Unauthorized)
        ));
    }

    #[test]
    fn extracts_sentry_key() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-sentry-auth",
            HeaderValue::from_static("Sentry sentry_version=7, sentry_key=public-key"),
        );
        assert_eq!(sentry_key(&headers), Some("public-key"));
    }

    #[test]
    fn recursively_scrubs_sensitive_fields() {
        let mut value =
            json!({"request": {"headers": {"authorization": "Bearer x"}}, "token": "x"});
        scrub_sensitive(&mut value);
        assert_eq!(value["token"], "[Filtered]");
        assert_eq!(value["request"]["headers"]["authorization"], "[Filtered]");
    }
}
