use serde::{Deserialize, Serialize};

/// SMTP connection settings for lettre.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// Use STARTTLS (port 587) vs implicit TLS (port 465).
    #[serde(default = "default_true")]
    pub starttls: bool,
}

fn default_true() -> bool {
    true
}

/// Email delivery configuration — either SMTP or Resend API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "backend")]
pub enum EmailConfig {
    /// Send via SMTP (lettre).
    #[serde(rename = "smtp")]
    Smtp {
        from_name: String,
        from_email: String,
        smtp: SmtpConfig,
    },
    /// Send via Resend HTTP API.
    #[serde(rename = "resend")]
    Resend {
        from_name: String,
        from_email: String,
        api_key: String,
    },
}

impl EmailConfig {
    pub fn from_address(&self) -> (&str, &str) {
        match self {
            EmailConfig::Smtp {
                from_name,
                from_email,
                ..
            } => (from_name, from_email),
            EmailConfig::Resend {
                from_name,
                from_email,
                ..
            } => (from_name, from_email),
        }
    }
}
