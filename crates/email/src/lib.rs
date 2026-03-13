mod config;
mod deliver;

pub use config::{EmailConfig, SmtpConfig};
pub use deliver::{send_invoice, DeliveryError, DeliveryResult};
