pub struct ServerState {
    pub db: aequi_storage::DbPool,
    pub api_key: Option<String>,
    pub email_config: Option<aequi_email::EmailConfig>,
    pub oidc: Option<crate::oidc::JwksCache>,
    pub stripe_webhook_secret: Option<String>,
    /// Read-only Stripe API access for looking up processing fees.
    pub stripe_api: Option<crate::stripe_api::StripeApi>,
    pub plaid: Option<aequi_import::plaid::PlaidClient>,
    /// Serializes Plaid syncs so two concurrent requests can't queue the same
    /// bank transaction twice.
    pub plaid_sync_lock: tokio::sync::Mutex<()>,
}
