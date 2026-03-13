pub struct ServerState {
    pub db: aequi_storage::DbPool,
    pub api_key: Option<String>,
    pub email_config: Option<aequi_email::EmailConfig>,
    pub oidc: Option<crate::oidc::JwksCache>,
    pub stripe_webhook_secret: Option<String>,
}
