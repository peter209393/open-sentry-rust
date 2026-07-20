use serde_json::json;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{error::Result, model::IngestEvent};

#[derive(sqlx::FromRow)]
struct MatchingRule {
    id: Uuid,
    name: String,
    channel: String,
    target: String,
    escalation_policy_id: Option<Uuid>,
}

pub async fn enqueue_matching_alerts(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    issue_id: Uuid,
    event_id: Uuid,
    event: &IngestEvent,
) -> Result<()> {
    let rules = sqlx::query_as::<_, MatchingRule>(
        r#"SELECT id, name, channel, target, escalation_policy_id FROM alert_rules
           WHERE project_id = $1 AND enabled
             AND (level IS NULL OR level = $2)
             AND (message_contains IS NULL OR $3 ILIKE '%' || message_contains || '%')
             AND (environment IS NULL OR environment = $4)
             AND (threshold_count IS NULL OR
                  (SELECT count(*) FROM events e WHERE e.project_id=$1
                   AND e.occurred_at >= now() - COALESCE(window_seconds,300) * interval '1 second'
                   AND (alert_rules.level IS NULL OR e.level=alert_rules.level)
                   AND (alert_rules.environment IS NULL OR e.environment=alert_rules.environment)) >= threshold_count)
             AND (last_triggered_at IS NULL OR last_triggered_at < now() - cooldown_seconds * interval '1 second')
           FOR UPDATE"#,
    )
    .bind(project_id).bind(&event.level).bind(&event.message).bind(&event.environment)
    .fetch_all(&mut **tx).await?;

    for rule in rules {
        let payload = json!({
            "rule_name": rule.name, "project_id": project_id, "issue_id": issue_id,
            "event_id": event_id, "level": event.level, "message": event.message,
        });
        let (channel, target, delay) = if let Some(policy_id) = rule.escalation_policy_id {
            sqlx::query_as::<_,(String,String,i32)>(r#"SELECT s.channel,COALESCE(s.target,u.email),s.delay_seconds FROM escalation_steps s LEFT JOIN LATERAL (SELECT u.email FROM on_call_members m JOIN users u ON u.id=m.user_id WHERE m.schedule_id=s.schedule_id AND u.active ORDER BY m.rotation_order LIMIT 1) u ON true WHERE s.policy_id=$1 AND s.step_order=0"#).bind(policy_id).fetch_optional(&mut **tx).await?.ok_or_else(||crate::error::AppError::BadRequest("escalation policy has no resolvable first step".into()))?
        } else {
            (rule.channel, rule.target, 0)
        };
        sqlx::query("INSERT INTO notification_outbox (rule_id, channel, target, payload, escalation_policy_id, escalation_step, available_at) VALUES ($1, $2, $3, $4, $5, 0, now()+$6*interval '1 second')")
            .bind(rule.id).bind(channel).bind(target).bind(payload).bind(rule.escalation_policy_id).bind(delay)
            .execute(&mut **tx).await?;
        sqlx::query("UPDATE alert_rules SET last_triggered_at = now() WHERE id = $1")
            .bind(rule.id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

pub async fn mark_issue_status(db: &PgPool, issue_id: Uuid, status: &str) -> Result<()> {
    if !matches!(
        status,
        "unresolved" | "in_progress" | "resolved" | "ignored"
    ) {
        return Err(crate::error::AppError::BadRequest(
            "invalid issue status".into(),
        ));
    }
    let result = sqlx::query("UPDATE issues SET status = $2 WHERE id = $1")
        .bind(issue_id)
        .bind(status)
        .execute(db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(crate::error::AppError::NotFound);
    }
    Ok(())
}

pub async fn enqueue_recovery_alerts(db: &PgPool, issue_id: Uuid) -> Result<()> {
    sqlx::query(r#"INSERT INTO notification_outbox(rule_id,channel,target,payload,dedup_key)
      SELECT r.id,r.channel,r.target,jsonb_build_object('rule_name',r.name,'project_id',i.project_id,'issue_id',i.id,'recovery',true),
             'recovery:'||r.id||':'||i.id||':'||i.event_count
      FROM issues i JOIN alert_rules r ON r.project_id=i.project_id
      WHERE i.id=$1 AND r.enabled AND r.notify_recovery
      ON CONFLICT(dedup_key) WHERE dedup_key IS NOT NULL DO NOTHING"#).bind(issue_id).execute(db).await?;
    Ok(())
}
