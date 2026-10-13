use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod daimon;
mod error;
pub(crate) mod oidc;
mod routes;
mod state;

use state::ServerState;

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    // AEQUI_LOG_FORMAT=json enables structured Bunyan-compatible JSON logging
    if std::env::var("AEQUI_LOG_FORMAT").as_deref() == Ok("json") {
        let formatting_layer = tracing_bunyan_formatter::BunyanFormattingLayer::new(
            "aequi-server".into(),
            std::io::stdout,
        );
        let json_fields_layer = tracing_bunyan_formatter::JsonStorageLayer;

        tracing_subscriber::registry()
            .with(filter)
            .with(json_fields_layer)
            .with(formatting_layer)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }

    let db_path = env_opt("AEQUI_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("aequi.db"));
    let port: u16 = match env_opt("AEQUI_PORT") {
        Some(p) => p
            .parse()
            .with_context(|| format!("AEQUI_PORT is not a valid port: {p:?}"))?,
        None => 8060,
    };
    let bind_ip: IpAddr = match env_opt("AEQUI_BIND_ADDR") {
        Some(a) => a
            .parse()
            .with_context(|| format!("AEQUI_BIND_ADDR is not an IP address: {a:?}"))?,
        None => IpAddr::from([0, 0, 0, 0]),
    };

    // Every optional integration that is configured but unparsable is a
    // startup error: silently ignoring a typo could e.g. disable auth.
    let api_key = env_opt("AEQUI_API_KEY");
    if api_key.as_ref().is_some_and(|k| k.len() < 16) {
        tracing::warn!("AEQUI_API_KEY is shorter than 16 characters; use a long random key");
    }

    let email_config: Option<aequi_email::EmailConfig> = parse_json_env("AEQUI_EMAIL_CONFIG")?;
    if email_config.is_some() {
        tracing::info!("Email delivery configured");
    }

    let oidc_config: Option<oidc::OidcConfig> = parse_json_env("AEQUI_OIDC_CONFIG")?;
    if let Some(cfg) = &oidc_config {
        tracing::info!(issuer = %cfg.issuer, "OIDC authentication configured");
        if cfg.allows_any_account() {
            tracing::warn!(
                "OIDC has no allowed_subjects/allowed_emails: ANY account the issuer \
                 authenticates for this audience gets full API access"
            );
        }
    }
    let oidc = oidc_config.map(oidc::JwksCache::new);

    let stripe_webhook_secret = env_opt("STRIPE_WEBHOOK_SECRET");
    if stripe_webhook_secret.is_some() {
        tracing::info!("Stripe webhook listener configured");
    }

    let plaid_config: Option<aequi_import::plaid::PlaidConfig> =
        parse_json_env("AEQUI_PLAID_CONFIG")?;
    if plaid_config.is_some() {
        tracing::info!("Plaid bank sync configured");
    }

    let insecure_no_auth = env_opt("AEQUI_INSECURE_NO_AUTH")
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    match check_auth_policy(api_key.is_some(), oidc.is_some(), bind_ip, insecure_no_auth) {
        Ok(AuthPolicy::Enforced) => {}
        Ok(AuthPolicy::DisabledOnLoopback) => tracing::warn!(
            "No AEQUI_API_KEY or AEQUI_OIDC_CONFIG: API authentication is DISABLED \
             (allowed only because the server is bound to loopback {bind_ip})"
        ),
        Ok(AuthPolicy::DisabledInsecure) => tracing::warn!(
            "AEQUI_INSECURE_NO_AUTH is set: API authentication is DISABLED on {bind_ip}. \
             Anyone who can reach this port can read and modify the ledger."
        ),
        Err(msg) => bail!(msg),
    }

    let cors =
        routes::cors_layer(env_opt("AEQUI_CORS_ORIGINS").as_deref()).map_err(anyhow::Error::msg)?;

    let db = aequi_storage::create_db(&db_path).await?;
    aequi_storage::seed_default_accounts(&db).await?;

    let state = Arc::new(ServerState {
        db,
        api_key,
        email_config,
        oidc,
        stripe_webhook_secret,
        plaid_config,
        plaid_sync_lock: Default::default(),
    });

    // Shutdown signal for background tasks
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Spawn daimon integration (non-blocking — server starts regardless)
    let daimon_handle = daimon::spawn_daimon_task(Arc::clone(&state), shutdown_rx);

    let app = routes::router(Arc::clone(&state), cors);

    let addr = SocketAddr::new(bind_ip, port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("aequi-server listening on {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // Server stopped — shut down background tasks
    let _ = shutdown_tx.send(true);
    let _ = daimon_handle.await;
    state.db.close().await;
    tracing::info!("aequi-server stopped");

    Ok(())
}

/// Read an environment variable, treating unset, empty and whitespace-only
/// the same (compose files often pass `VAR=` for "not configured").
fn env_opt(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn parse_json_env<T: DeserializeOwned>(name: &str) -> Result<Option<T>> {
    parse_json_config(name, env_opt(name).as_deref())
}

/// Parse an optional JSON config value. Absent is `None`; present but invalid
/// is an error naming the variable and the parse problem.
fn parse_json_config<T: DeserializeOwned>(name: &str, value: Option<&str>) -> Result<Option<T>> {
    match value {
        None => Ok(None),
        Some(json) => serde_json::from_str(json)
            .map(Some)
            .with_context(|| format!("{name} is set but is not a valid configuration")),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AuthPolicy {
    Enforced,
    DisabledOnLoopback,
    DisabledInsecure,
}

/// Refuse to serve an unauthenticated API on a non-loopback address unless
/// the operator explicitly opted in with `AEQUI_INSECURE_NO_AUTH=1`.
fn check_auth_policy(
    has_api_key: bool,
    has_oidc: bool,
    bind_ip: IpAddr,
    insecure_no_auth: bool,
) -> Result<AuthPolicy, String> {
    if has_api_key || has_oidc {
        Ok(AuthPolicy::Enforced)
    } else if bind_ip.is_loopback() {
        Ok(AuthPolicy::DisabledOnLoopback)
    } else if insecure_no_auth {
        Ok(AuthPolicy::DisabledInsecure)
    } else {
        Err(format!(
            "refusing to start: no authentication configured while listening on {bind_ip}. \
             Set AEQUI_API_KEY or AEQUI_OIDC_CONFIG, bind to loopback with \
             AEQUI_BIND_ADDR=127.0.0.1, or set AEQUI_INSECURE_NO_AUTH=1 to run without auth"
        ))
    }
}

/// Resolves on Ctrl-C or SIGTERM (what `docker stop` sends).
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::warn!("failed to listen for Ctrl-C: {e}");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                tracing::warn!("failed to listen for SIGTERM: {e}");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("shutdown signal received, draining connections");
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANY: IpAddr = IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED);
    const LOOPBACK: IpAddr = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

    #[test]
    fn auth_required_on_public_bind() {
        assert!(check_auth_policy(false, false, ANY, false).is_err());
        assert_eq!(
            check_auth_policy(true, false, ANY, false),
            Ok(AuthPolicy::Enforced)
        );
        assert_eq!(
            check_auth_policy(false, true, ANY, false),
            Ok(AuthPolicy::Enforced)
        );
    }

    #[test]
    fn no_auth_allowed_on_loopback_or_explicit_opt_in() {
        assert_eq!(
            check_auth_policy(false, false, LOOPBACK, false),
            Ok(AuthPolicy::DisabledOnLoopback)
        );
        assert_eq!(
            check_auth_policy(false, false, "::1".parse().unwrap(), false),
            Ok(AuthPolicy::DisabledOnLoopback)
        );
        assert_eq!(
            check_auth_policy(false, false, ANY, true),
            Ok(AuthPolicy::DisabledInsecure)
        );
    }

    #[test]
    fn invalid_json_config_is_an_error() {
        let r: Result<Option<oidc::OidcConfig>> = parse_json_config(
            "AEQUI_OIDC_CONFIG",
            Some(r#"{"issuer_url":"https://x","audience":"a"}"#),
        );
        let msg = format!("{:#}", r.unwrap_err());
        assert!(msg.contains("AEQUI_OIDC_CONFIG"), "{msg}");

        let r: Result<Option<oidc::OidcConfig>> = parse_json_config("X", None);
        assert!(r.unwrap().is_none());
    }

    /// Every JSON example in .env.example must parse into its config type.
    #[test]
    fn env_example_configs_parse() {
        let example = include_str!("../../../.env.example");
        let mut seen = 0;
        for line in example.lines() {
            let Some(rest) = line.trim_start_matches('#').trim().split_once('=') else {
                continue;
            };
            let (key, value) = (rest.0.trim(), rest.1.trim());
            if !value.starts_with('{') {
                continue;
            }
            let ok = match key {
                "AEQUI_OIDC_CONFIG" => serde_json::from_str::<oidc::OidcConfig>(value).map(drop),
                "AEQUI_EMAIL_CONFIG" => {
                    serde_json::from_str::<aequi_email::EmailConfig>(value).map(drop)
                }
                "AEQUI_PLAID_CONFIG" => {
                    serde_json::from_str::<aequi_import::plaid::PlaidConfig>(value).map(drop)
                }
                other => panic!("unexpected JSON example for {other}"),
            };
            ok.unwrap_or_else(|e| panic!("{key} example does not parse: {e}\n{value}"));
            seen += 1;
        }
        assert!(
            seen >= 4,
            "expected JSON examples in .env.example, found {seen}"
        );
    }
}
