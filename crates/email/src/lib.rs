mod config;
mod deliver;

pub use config::{EmailConfig, SmtpConfig};
pub use deliver::{DeliveryError, DeliveryResult, send_invoice};
