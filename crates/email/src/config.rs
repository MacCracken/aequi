use serde::{Deserialize, Serialize};
use std::fmt;

/// SMTP connection settings for lettre.
#[derive(Clone, Serialize, Deserialize)]
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

// Redact password in Debug output
impl fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("starttls", &self.starttls)
            .finish()
    }
}

/// Email delivery configuration — either SMTP or Resend API.
#[derive(Clone, Deserialize)]
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

// Redact secrets in Debug output
impl fmt::Debug for EmailConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmailConfig::Smtp {
                from_name,
                from_email,
                smtp,
            } => f
                .debug_struct("EmailConfig::Smtp")
                .field("from_name", from_name)
                .field("from_email", from_email)
                .field("smtp", smtp)
                .finish(),
            EmailConfig::Resend {
                from_name,
                from_email,
                ..
            } => f
                .debug_struct("EmailConfig::Resend")
                .field("from_name", from_name)
                .field("from_email", from_email)
                .field("api_key", &"[REDACTED]")
                .finish(),
        }
    }
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
