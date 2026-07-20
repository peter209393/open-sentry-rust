use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Settings {
    #[serde(default = "default_environment")]
    pub environment: String,
    pub bind_addr: String,
    pub database_url: String,
    #[allow(dead_code)] // retained for backwards-compatible configuration parsing
    pub ingest_api_key: String,
    pub public_base_url: String,
    #[serde(default = "default_admin_email")]
    pub bootstrap_admin_email: String,
    #[serde(default = "default_admin_password")]
    pub bootstrap_admin_password: String,
    #[serde(default)]
    pub secure_cookies: bool,
    #[serde(default = "default_ingest_rate_limit")]
    pub ingest_rate_limit_per_minute: u64,
    #[serde(default = "default_metrics_api_key")]
    pub metrics_api_key: String,
    #[serde(default = "default_retention_days")]
    pub retention_days: i64,
    #[serde(default = "default_retention_interval")]
    pub retention_interval_seconds: u64,
    #[serde(default = "default_poll_interval")]
    pub worker_poll_interval_ms: u64,
    #[serde(default)]
    pub smtp: SmtpSettings,
    #[serde(default)]
    pub telegram: TelegramSettings,
    #[serde(default)]
    pub twilio: TwilioSettings,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SmtpSettings {
    #[serde(default)]
    pub enabled: bool,
    pub host: Option<String>,
    #[serde(default = "default_smtp_port")]
    pub port: u16,
    #[serde(default = "default_true")]
    pub starttls: bool,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct TelegramSettings {
    pub bot_token: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct TwilioSettings {
    pub account_sid: Option<String>,
    pub auth_token: Option<String>,
    pub from_number: Option<String>,
}

fn default_poll_interval() -> u64 {
    1_000
}
fn default_smtp_port() -> u16 {
    587
}
fn default_true() -> bool {
    true
}
fn default_admin_email() -> String {
    "admin@example.com".into()
}
fn default_admin_password() -> String {
    "change-me".into()
}
fn default_environment() -> String {
    "development".into()
}
fn default_ingest_rate_limit() -> u64 {
    600
}
fn default_metrics_api_key() -> String {
    "metrics-secret".into()
}
fn default_retention_days() -> i64 {
    30
}
fn default_retention_interval() -> u64 {
    3_600
}

impl Settings {
    pub fn load() -> Result<Self, config::ConfigError> {
        config::Config::builder()
            .set_default("bind_addr", "0.0.0.0:8080")?
            .set_default("public_base_url", "http://localhost:8080")?
            .set_default("worker_poll_interval_ms", 1_000_u64)?
            .add_source(config::Environment::with_prefix("APP").separator("__"))
            .build()?
            .try_deserialize()
    }

    pub fn validate_production(&self) -> anyhow::Result<()> {
        if self.environment == "production" {
            if self.bootstrap_admin_password == "change-me"
                || self.bootstrap_admin_password.len() < 12
            {
                anyhow::bail!(
                    "production bootstrap admin password must be changed and contain at least 12 characters"
                );
            }
            if self.metrics_api_key == "metrics-secret" || self.metrics_api_key.len() < 16 {
                anyhow::bail!(
                    "production metrics API key must be changed and contain at least 16 characters"
                );
            }
            if !self.secure_cookies {
                anyhow::bail!("production requires APP__SECURE_COOKIES=true");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_rejects_development_secrets() {
        let settings = Settings {
            environment: "production".into(),
            bind_addr: "x".into(),
            database_url: "x".into(),
            ingest_api_key: "x".into(),
            public_base_url: "x".into(),
            bootstrap_admin_email: "a@b.c".into(),
            bootstrap_admin_password: "change-me".into(),
            secure_cookies: false,
            ingest_rate_limit_per_minute: 1,
            metrics_api_key: "metrics-secret".into(),
            retention_days: 30,
            retention_interval_seconds: 60,
            worker_poll_interval_ms: 1000,
            smtp: SmtpSettings::default(),
            telegram: TelegramSettings::default(),
            twilio: TwilioSettings::default(),
        };
        assert!(settings.validate_production().is_err());
    }
}
