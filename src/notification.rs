use std::{sync::Arc, time::Duration};

use anyhow::{Context, anyhow};
use hmac::{Hmac, Mac};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::Mailbox};
use serde_json::Value;
use sha2::Sha256;
use uuid::Uuid;

use crate::state::AppState;

#[derive(sqlx::FromRow)]
struct Job {
    id: Uuid,
    channel: String,
    target: String,
    payload: Value,
    attempts: i32,
    rule_id: Uuid,
    escalation_policy_id: Option<Uuid>,
    escalation_step: i32,
}

pub fn channel_configuration_error(
    state: &AppState,
    channel: &str,
    target: &str,
) -> Option<String> {
    if target.trim().is_empty() {
        return Some("notification target is empty".into());
    }
    match channel {
        "email" if !state.settings.smtp.enabled => Some("SMTP is disabled".into()),
        "email" if state.settings.smtp.host.is_none() || state.settings.smtp.from.is_none() => {
            Some("SMTP host/from is incomplete".into())
        }
        "telegram" if state.settings.telegram.bot_token.is_none() => {
            Some("Telegram bot token is not configured".into())
        }
        "voice_call" if !is_e164(target) => {
            Some("voice target must use E.164 format, for example +60123456789".into())
        }
        "voice_call"
            if state.settings.twilio.account_sid.is_none()
                || state.settings.twilio.auth_token.is_none()
                || state.settings.twilio.from_number.is_none() =>
        {
            Some("Twilio account SID/auth token/from number is incomplete".into())
        }
        "webhook" => target
            .parse::<Uuid>()
            .err()
            .map(|_| "webhook target must be an endpoint id".into()),
        "email" | "telegram" | "voice_call" => None,
        _ => Some("unsupported notification channel".into()),
    }
}

pub async fn send_invitation_email(
    state: &AppState,
    target: &str,
    invite_url: &str,
) -> anyhow::Result<()> {
    send_email(
        state,
        target,
        "[Open Sentry] Workspace invitation",
        &format!("This single-use invitation expires in 48 hours:\n\n{invite_url}"),
    )
    .await
}

pub async fn run_worker(state: Arc<AppState>) {
    let interval = Duration::from_millis(state.settings.worker_poll_interval_ms);
    loop {
        if let Err(error) = process_batch(&state).await {
            tracing::error!(%error, "notification worker failed");
        }
        tokio::time::sleep(interval).await;
    }
}

async fn process_batch(state: &AppState) -> anyhow::Result<()> {
    let mut tx = state.db.begin().await?;
    let jobs = sqlx::query_as::<_, Job>(
        r#"WITH claimed AS (
             SELECT id FROM notification_outbox
             WHERE (status = 'pending' AND available_at <= now())
                OR (status = 'processing' AND claimed_at < now() - interval '5 minutes')
             ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 20
           )
           UPDATE notification_outbox AS n SET status = 'processing', claimed_at = now()
           FROM claimed WHERE n.id = claimed.id
           RETURNING n.id, n.channel, n.target, n.payload, n.attempts, n.rule_id, n.escalation_policy_id, n.escalation_step"#,
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    for job in jobs {
        let result = send(state, &job).await;
        match result {
            Ok(()) => {
                sqlx::query(
                    "UPDATE notification_outbox SET status='sent', sent_at=now() WHERE id=$1",
                )
                .bind(job.id)
                .execute(&state.db)
                .await?;
            }
            Err(error) => {
                let delay = 2_i64.pow((job.attempts as u32).min(8));
                sqlx::query("UPDATE notification_outbox SET status = CASE WHEN attempts >= 7 THEN 'failed' ELSE 'pending' END, attempts=attempts+1, last_error=$2, available_at=now()+$3*interval '1 second' WHERE id=$1")
                    .bind(job.id).bind(error.to_string()).bind(delay).execute(&state.db).await?;
                if job.attempts >= 7 {
                    enqueue_next_escalation(state, &job).await?;
                }
            }
        }
    }
    Ok(())
}

async fn enqueue_next_escalation(state: &AppState, job: &Job) -> anyhow::Result<()> {
    let Some(policy_id) = job.escalation_policy_id else {
        return Ok(());
    };
    let next = job.escalation_step + 1;
    let row=sqlx::query_as::<_,(String,String,i32)>(r#"SELECT s.channel,COALESCE(s.target,u.email),s.delay_seconds FROM escalation_steps s LEFT JOIN LATERAL (SELECT u.email FROM on_call_members m JOIN users u ON u.id=m.user_id WHERE m.schedule_id=s.schedule_id AND u.active ORDER BY m.rotation_order LIMIT 1) u ON true WHERE s.policy_id=$1 AND s.step_order=$2"#).bind(policy_id).bind(next).fetch_optional(&state.db).await?;
    if let Some((channel, target, delay)) = row {
        let dedup = format!("escalation:{}:{}", job.id, next);
        sqlx::query("INSERT INTO notification_outbox(rule_id,channel,target,payload,escalation_policy_id,escalation_step,available_at,dedup_key) VALUES($1,$2,$3,$4,$5,$6,now()+$7*interval '1 second',$8) ON CONFLICT(dedup_key) WHERE dedup_key IS NOT NULL DO NOTHING").bind(job.rule_id).bind(channel).bind(target).bind(&job.payload).bind(policy_id).bind(next).bind(delay).bind(dedup).execute(&state.db).await?;
    }
    Ok(())
}

async fn send(state: &AppState, job: &Job) -> anyhow::Result<()> {
    let subject = format!(
        "[Open Sentry] {}",
        job.payload["rule_name"].as_str().unwrap_or("Alert")
    );
    let body = format!(
        "{}\n\nIssue: {}/api/issues/{}",
        job.payload,
        state.settings.public_base_url,
        job.payload["issue_id"].as_str().unwrap_or("")
    );
    match job.channel.as_str() {
        "telegram" => send_telegram(state, &job.target, &body).await,
        "email" => send_email(state, &job.target, &subject, &body).await,
        "voice_call" => send_voice_call(state, &job.target, &job.payload).await,
        "webhook" => send_webhook(state, job).await,
        other => Err(anyhow!("unsupported notification channel: {other}")),
    }
}

async fn send_webhook(state: &AppState, job: &Job) -> anyhow::Result<()> {
    let endpoint_id = job
        .target
        .parse::<Uuid>()
        .context("webhook target must be an endpoint id")?;
    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT url,signing_secret FROM webhook_endpoints WHERE id=$1 AND enabled",
    )
    .bind(endpoint_id)
    .fetch_optional(&state.db)
    .await?
    .context("webhook endpoint is disabled or missing")?;
    let body = serde_json::to_vec(&job.payload)?;
    let signature = webhook_signature(&row.1, &body)?;
    let result = state
        .http
        .post(&row.0)
        .header("content-type", "application/json")
        .header("x-open-sentry-signature", signature)
        .header("x-open-sentry-delivery", job.id.to_string())
        .body(body)
        .send()
        .await;
    match result {
        Ok(response) => {
            let code = response.status().as_u16() as i32;
            sqlx::query("INSERT INTO webhook_deliveries(endpoint_id,notification_id,status_code) VALUES($1,$2,$3)").bind(endpoint_id).bind(job.id).bind(code).execute(&state.db).await?;
            response.error_for_status()?;
            Ok(())
        }
        Err(error) => {
            sqlx::query("INSERT INTO webhook_deliveries(endpoint_id,notification_id,error) VALUES($1,$2,$3)").bind(endpoint_id).bind(job.id).bind(error.to_string()).execute(&state.db).await?;
            Err(error.into())
        }
    }
}

fn webhook_signature(secret: &str, body: &[u8]) -> anyhow::Result<String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())?;
    mac.update(body);
    Ok(format!("sha256={:x}", mac.finalize().into_bytes()))
}

fn is_e164(number: &str) -> bool {
    let digits = number.strip_prefix('+').unwrap_or("");
    (8..=15).contains(&digits.len()) && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

async fn send_voice_call(state: &AppState, target: &str, payload: &Value) -> anyhow::Result<()> {
    if !is_e164(target) {
        return Err(anyhow!("voice target must use E.164 format"));
    }
    let twilio = &state.settings.twilio;
    let account_sid = twilio
        .account_sid
        .as_deref()
        .context("Twilio account SID is not configured")?;
    let auth_token = twilio
        .auth_token
        .as_deref()
        .context("Twilio auth token is not configured")?;
    let from = twilio
        .from_number
        .as_deref()
        .context("Twilio from number is not configured")?;
    let rule = xml_escape(payload["rule_name"].as_str().unwrap_or("Open Sentry alert"));
    let level = xml_escape(payload["level"].as_str().unwrap_or("critical"));
    let message = xml_escape(
        payload["message"]
            .as_str()
            .unwrap_or("Please check the incident console"),
    );
    let twiml = format!(
        "<Response><Say language=\"en-US\">Open Sentry urgent alert. Rule {rule}. Level {level}. {message}. Please check the incident console.</Say><Pause length=\"1\"/><Say language=\"en-US\">Repeating. Open Sentry urgent alert. Rule {rule}. Level {level}.</Say></Response>"
    );
    state
        .http
        .post(format!(
            "https://api.twilio.com/2010-04-01/Accounts/{account_sid}/Calls.json"
        ))
        .basic_auth(account_sid, Some(auth_token))
        .form(&[("To", target), ("From", from), ("Twiml", twiml.as_str())])
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn send_telegram(state: &AppState, chat_id: &str, text: &str) -> anyhow::Result<()> {
    let token = state
        .settings
        .telegram
        .bot_token
        .as_deref()
        .context("telegram bot token is not configured")?;
    state
        .http
        .post(format!("https://api.telegram.org/bot{token}/sendMessage"))
        .json(&serde_json::json!({ "chat_id": chat_id, "text": text }))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn send_email(
    state: &AppState,
    target: &str,
    subject: &str,
    body: &str,
) -> anyhow::Result<()> {
    let smtp = &state.settings.smtp;
    if !smtp.enabled {
        return Err(anyhow!("SMTP is disabled"));
    }
    let from: Mailbox = smtp
        .from
        .as_deref()
        .context("SMTP from is not configured")?
        .parse()?;
    let message = Message::builder()
        .from(from)
        .to(target.parse()?)
        .subject(subject)
        .body(body.to_owned())?;
    let host = smtp
        .host
        .as_deref()
        .context("SMTP host is not configured")?;
    let mut builder = if smtp.starttls {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)?.port(smtp.port)
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host).port(smtp.port)
    };
    if let (Some(user), Some(password)) = (&smtp.username, &smtp.password) {
        builder = builder.credentials(lettre::transport::smtp::authentication::Credentials::new(
            user.clone(),
            password.clone(),
        ));
    }
    builder.build().send(message).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_e164_phone_numbers() {
        assert!(is_e164("+60123456789"));
        assert!(!is_e164("012-3456789"));
        assert!(!is_e164("+60abc"));
    }

    #[test]
    fn webhook_signatures_are_deterministic_and_body_bound() {
        let first = webhook_signature("secret", br#"{"event":"alert"}"#).unwrap();
        assert_eq!(
            first,
            webhook_signature("secret", br#"{"event":"alert"}"#).unwrap()
        );
        assert_ne!(
            first,
            webhook_signature("secret", br#"{"event":"resolved"}"#).unwrap()
        );
        assert!(first.starts_with("sha256="));
    }

    #[test]
    fn escapes_alert_text_for_twiml() {
        assert_eq!(xml_escape("a < b & c"), "a &lt; b &amp; c");
    }
}
