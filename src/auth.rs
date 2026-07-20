use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, HeaderValue, Request, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    config::Settings,
    error::{AppError, Result},
    state::AppState,
};

const SESSION_COOKIE: &str = "open_sentry_session";

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct UserView {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct ManagedUserView {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub active: bool,
    pub last_login_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

pub async fn ensure_bootstrap_admin(db: &PgPool, settings: &Settings) -> anyhow::Result<()> {
    if settings.bootstrap_admin_password.len() < 8 {
        anyhow::bail!("APP__BOOTSTRAP_ADMIN_PASSWORD must contain at least 8 characters");
    }
    let hash = hash_password(&settings.bootstrap_admin_password)?;
    sqlx::query("INSERT INTO users (organization_id,email,display_name,password_hash,role) VALUES ('00000000-0000-0000-0000-000000000001',$1,'Administrator',$2,'owner') ON CONFLICT (organization_id,email) DO NOTHING")
        .bind(settings.bootstrap_admin_email.trim().to_lowercase()).bind(hash).execute(db).await?;
    Ok(())
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginRequest>,
) -> Result<Response> {
    let row = sqlx::query_as::<_, (Uuid, String, String, String, String)>(
        "SELECT id,email,display_name,role,password_hash FROM users WHERE lower(email)=lower($1) AND active",
    )
    .bind(body.email.trim())
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| {
        state.metrics.auth_failure();
        AppError::Unauthorized
    })?;
    let parsed = PasswordHash::new(&row.4)
        .map_err(|error| AppError::Internal(anyhow::anyhow!(error.to_string())))?;
    Argon2::default()
        .verify_password(body.password.as_bytes(), &parsed)
        .map_err(|_| {
            state.metrics.auth_failure();
            AppError::Unauthorized
        })?;
    sqlx::query("UPDATE users SET last_login_at=now() WHERE id=$1")
        .bind(row.0)
        .execute(&state.db)
        .await?;
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let token_hash = hash_token(&token);
    sqlx::query("DELETE FROM user_sessions WHERE expires_at <= now()")
        .execute(&state.db)
        .await?;
    sqlx::query("INSERT INTO user_sessions (user_id,token_hash,expires_at) VALUES ($1,$2,now()+interval '7 days')")
        .bind(row.0).bind(token_hash).execute(&state.db).await?;
    let user = UserView {
        id: row.0,
        email: row.1,
        display_name: row.2,
        role: row.3,
    };
    crate::operations::audit(
        &state.db,
        &user,
        "auth.login",
        "user",
        Some(user.id),
        json!({}),
    )
    .await?;
    let mut response = Json(user).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=604800{}",
            if state.settings.secure_cookies {
                "; Secure"
            } else {
                ""
            }
        ))
        .unwrap(),
    );
    Ok(response)
}

pub async fn me(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Result<Json<UserView>> {
    Ok(Json(authenticate(&state.db, &headers).await?))
}

pub async fn logout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Result<Response> {
    let user = authenticate(&state.db, &headers).await.ok();
    if let Some(token) = cookie_token(&headers) {
        sqlx::query("DELETE FROM user_sessions WHERE token_hash=$1")
            .bind(hash_token(token))
            .execute(&state.db)
            .await?;
    }
    if let Some(user) = user {
        crate::operations::audit(
            &state.db,
            &user,
            "auth.logout",
            "user",
            Some(user.id),
            json!({}),
        )
        .await?;
    }
    let mut response = Json(json!({"ok":true})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "open_sentry_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}",
            if state.settings.secure_cookies {
                "; Secure"
            } else {
                ""
            }
        ))
        .unwrap(),
    );
    Ok(response)
}

pub async fn authenticate(db: &PgPool, headers: &HeaderMap) -> Result<UserView> {
    let token = cookie_token(headers).ok_or(AppError::Unauthorized)?;
    let user = sqlx::query_as::<_, UserView>("SELECT u.id,u.email,u.display_name,u.role FROM user_sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=$1 AND s.expires_at>now() AND u.active")
        .bind(hash_token(token)).fetch_optional(db).await?.ok_or(AppError::Unauthorized)?;
    Ok(user)
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    if password.len() < 8 {
        anyhow::bail!("password must contain at least 8 characters");
    }
    let salt = SaltString::encode_b64(Uuid::new_v4().as_bytes())
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?
        .to_string())
}

pub async fn authorize_project(
    db: &PgPool,
    headers: &HeaderMap,
    project_id: Uuid,
) -> Result<UserView> {
    let user = authenticate(db, headers).await?;
    let allowed = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM projects p JOIN users u ON u.organization_id=p.organization_id WHERE p.id=$1 AND u.id=$2)")
        .bind(project_id).bind(user.id).fetch_one(db).await?;
    if !allowed {
        return Err(AppError::Unauthorized);
    }
    Ok(user)
}

pub async fn authorize_issue(db: &PgPool, headers: &HeaderMap, issue_id: Uuid) -> Result<UserView> {
    let user = authenticate(db, headers).await?;
    let allowed = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM issues i JOIN projects p ON p.id=i.project_id JOIN users u ON u.organization_id=p.organization_id WHERE i.id=$1 AND u.id=$2)")
        .bind(issue_id).bind(user.id).fetch_one(db).await?;
    if !allowed {
        return Err(AppError::Unauthorized);
    }
    Ok(user)
}

pub async fn require_session(
    State(state): State<Arc<AppState>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response> {
    authenticate(&state.db, request.headers()).await?;
    Ok(next.run(request).await)
}

pub fn require_admin(user: &UserView) -> Result<()> {
    if matches!(user.role.as_str(), "owner" | "admin") {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

fn cookie_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|v| v.strip_prefix(&format!("{SESSION_COOKIE}=")))
}
fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_only_named_cookie() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("x=1; open_sentry_session=abc; y=2"),
        );
        assert_eq!(cookie_token(&h), Some("abc"));
    }
    #[test]
    fn token_hash_is_not_plaintext() {
        assert_ne!(hash_token("secret"), "secret");
        assert_eq!(hash_token("secret"), hash_token("secret"));
    }
    #[test]
    fn role_enforces_write_permissions() {
        let member = UserView {
            id: Uuid::nil(),
            email: "member@example.com".into(),
            display_name: "Member".into(),
            role: "member".into(),
        };
        let admin = UserView {
            role: "admin".into(),
            ..member.clone()
        };
        assert!(matches!(require_admin(&member), Err(AppError::Forbidden)));
        assert!(require_admin(&admin).is_ok());
    }
}
