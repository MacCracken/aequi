use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve, Jwk, JwkSet, PublicKeyUse};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Deserializer, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcConfig {
    /// OIDC issuer URL (e.g. `https://accounts.google.com`)
    pub issuer: String,
    /// Expected audience (client ID)
    pub audience: String,
    /// Token subjects (`sub`) allowed to use the API.
    #[serde(default)]
    pub allowed_subjects: Vec<String>,
    /// Email addresses allowed to use the API (case-insensitive). Only
    /// honoured when the token says the email is verified.
    #[serde(default)]
    pub allowed_emails: Vec<String>,
}

impl OidcConfig {
    /// True when no allowlist is configured, i.e. every account the issuer
    /// vouches for gets full access.
    pub fn allows_any_account(&self) -> bool {
        self.allowed_subjects.is_empty() && self.allowed_emails.is_empty()
    }

    fn is_authorized(&self, claims: &Claims) -> bool {
        if self.allows_any_account() {
            return true;
        }
        if self.allowed_subjects.iter().any(|s| s == &claims.sub) {
            return true;
        }
        match (&claims.email, claims.email_verified) {
            (Some(email), Some(true)) => self
                .allowed_emails
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(email)),
            _ => false,
        }
    }
}

/// Minimum interval between JWKS fetches once keys are cached, so tokens with
/// unknown key ids can't make us hammer the identity provider.
const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(300);

/// Retry interval while no keys could be fetched yet (IdP unreachable at boot).
const MIN_RETRY_INTERVAL_WITHOUT_KEYS: Duration = Duration::from_secs(30);

#[derive(Default)]
struct KeyState {
    keys: Option<Arc<JwkSet>>,
    /// Bumped on every successful refresh so waiters can tell that another
    /// request already refreshed while they were queued.
    generation: u64,
    last_attempt: Option<Instant>,
}

/// Cached JWKS keys for token validation.
#[derive(Clone)]
pub struct JwksCache {
    config: Arc<OidcConfig>,
    http: reqwest::Client,
    state: Arc<RwLock<KeyState>>,
    /// Single-flight guard: at most one JWKS fetch runs at a time.
    refresh_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Deserialize)]
struct OidcDiscovery {
    issuer: String,
    jwks_uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub iss: String,
    pub aud: serde_json::Value,
    pub exp: u64,
    #[serde(default)]
    pub iat: Option<u64>,
    #[serde(default)]
    pub email: Option<String>,
    /// Some IdPs send this as the string "true"/"false".
    #[serde(default, deserialize_with = "bool_or_string")]
    pub email_verified: Option<bool>,
}

fn bool_or_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum BoolOrString {
        Bool(bool),
        Str(String),
    }
    Ok(match Option::<BoolOrString>::deserialize(d)? {
        Some(BoolOrString::Bool(b)) => Some(b),
        Some(BoolOrString::Str(s)) => Some(s.eq_ignore_ascii_case("true")),
        None => None,
    })
}

#[derive(Debug)]
enum ValidateError {
    /// The token's `kid` is not in the key set — keys may have rotated.
    UnknownKid(String),
    Invalid(String),
}

impl std::fmt::Display for ValidateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidateError::UnknownKid(kid) => write!(f, "no matching key for kid: {kid}"),
            ValidateError::Invalid(msg) => f.write_str(msg),
        }
    }
}

impl JwksCache {
    pub fn new(config: OidcConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client");
        Self {
            config: Arc::new(config),
            http,
            state: Arc::new(RwLock::new(KeyState::default())),
            refresh_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Refresh the key set unless another request already did so after
    /// `seen_generation`, or the last attempt was too recent. Concurrent
    /// callers queue on `refresh_lock` and reuse the winner's result.
    async fn refresh_keys(&self, seen_generation: u64) -> Result<Arc<JwkSet>, String> {
        let _guard = self.refresh_lock.lock().await;

        {
            let state = self.state.read().await;
            if state.generation != seen_generation
                && let Some(keys) = &state.keys
            {
                return Ok(Arc::clone(keys));
            }
            let min_interval = if state.keys.is_some() {
                MIN_REFRESH_INTERVAL
            } else {
                MIN_RETRY_INTERVAL_WITHOUT_KEYS
            };
            if let Some(last) = state.last_attempt
                && last.elapsed() < min_interval
            {
                return Err("JWKS refresh rate limited".to_string());
            }
        }

        // Record the attempt before fetching so a failing IdP is also rate limited.
        self.state.write().await.last_attempt = Some(Instant::now());

        let jwks = Arc::new(self.fetch_jwks().await?);

        let mut state = self.state.write().await;
        state.keys = Some(Arc::clone(&jwks));
        state.generation += 1;
        Ok(jwks)
    }

    /// Fetch JWKS from the issuer's discovery endpoint.
    async fn fetch_jwks(&self) -> Result<JwkSet, String> {
        let issuer = self.config.issuer.trim_end_matches('/');
        let discovery_url = format!("{issuer}/.well-known/openid-configuration");

        let discovery: OidcDiscovery = self
            .http
            .get(&discovery_url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("OIDC discovery failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("OIDC discovery parse failed: {e}"))?;

        // OIDC Discovery §4.3: the document must be for the configured issuer.
        if discovery.issuer.trim_end_matches('/') != issuer {
            return Err(format!(
                "OIDC discovery issuer mismatch: expected {issuer}, got {}",
                discovery.issuer
            ));
        }

        self.http
            .get(&discovery.jwks_uri)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("JWKS fetch failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("JWKS parse failed: {e}"))
    }

    /// Validate a JWT token and check it against the configured allowlists.
    /// Returns claims on success.
    pub async fn validate_token(&self, token: &str) -> Result<Claims, String> {
        let (cached, generation) = {
            let state = self.state.read().await;
            (state.keys.clone(), state.generation)
        };

        let jwks = match cached {
            Some(keys) => keys,
            None => self.refresh_keys(generation).await?,
        };

        let claims = match self.try_validate(token, &jwks) {
            Ok(claims) => claims,
            // Only an unknown key id suggests rotation; other failures
            // (expired, bad signature, wrong audience) never trigger a fetch.
            Err(ValidateError::UnknownKid(_)) => {
                let refreshed = self.refresh_keys(generation).await?;
                self.try_validate(token, &refreshed)
                    .map_err(|e| e.to_string())?
            }
            Err(e) => return Err(e.to_string()),
        };

        if !self.config.is_authorized(&claims) {
            return Err(format!(
                "subject {} is not in the OIDC allowlist",
                claims.sub
            ));
        }
        Ok(claims)
    }

    fn try_validate(&self, token: &str, jwks: &JwkSet) -> Result<Claims, ValidateError> {
        let header = jsonwebtoken::decode_header(token)
            .map_err(|e| ValidateError::Invalid(format!("invalid JWT header: {e}")))?;

        // Symmetric algorithms would let anyone holding a public key forge tokens.
        if !is_asymmetric(header.alg) {
            return Err(ValidateError::Invalid(format!(
                "algorithm {:?} not allowed",
                header.alg
            )));
        }

        let kid = header
            .kid
            .as_ref()
            .ok_or_else(|| ValidateError::Invalid("JWT missing kid header".to_string()))?;

        let jwk = jwks
            .find(kid)
            .ok_or_else(|| ValidateError::UnknownKid(kid.clone()))?;

        let alg = algorithm_for_key(jwk, header.alg).map_err(ValidateError::Invalid)?;

        let decoding_key = DecodingKey::from_jwk(jwk)
            .map_err(|e| ValidateError::Invalid(format!("invalid JWK: {e}")))?;

        let mut validation = Validation::new(alg);
        validation.set_audience(&[&self.config.audience]);
        validation.set_issuer(&[&self.config.issuer]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.leeway = 60; // 60 second clock skew tolerance

        let data = decode::<Claims>(token, &decoding_key, &validation)
            .map_err(|e| ValidateError::Invalid(format!("JWT validation failed: {e}")))?;

        Ok(data.claims)
    }
}

fn is_asymmetric(alg: Algorithm) -> bool {
    !matches!(alg, Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512)
}

/// The algorithm to verify with, decided by the key rather than the token:
/// the JWK's own `alg` if it has one, otherwise the algorithms its key type
/// supports. The token's `alg` must match.
fn algorithm_for_key(jwk: &Jwk, token_alg: Algorithm) -> Result<Algorithm, String> {
    if matches!(jwk.common.public_key_use, Some(PublicKeyUse::Encryption)) {
        return Err("JWK is an encryption key".to_string());
    }

    let allowed: &[Algorithm] = match (&jwk.common.key_algorithm, &jwk.algorithm) {
        (Some(key_alg), _) => {
            let alg = Algorithm::try_from(*key_alg)
                .map_err(|_| format!("JWK algorithm {key_alg:?} is not a signing algorithm"))?;
            if !is_asymmetric(alg) {
                return Err(format!("JWK algorithm {alg:?} not allowed"));
            }
            if alg != token_alg {
                return Err(format!(
                    "token algorithm {token_alg:?} does not match key algorithm {alg:?}"
                ));
            }
            return Ok(alg);
        }
        (None, AlgorithmParameters::RSA(_)) => &[
            Algorithm::RS256,
            Algorithm::RS384,
            Algorithm::RS512,
            Algorithm::PS256,
            Algorithm::PS384,
            Algorithm::PS512,
        ],
        (None, AlgorithmParameters::EllipticCurve(p)) => match p.curve {
            EllipticCurve::P256 => &[Algorithm::ES256],
            EllipticCurve::P384 => &[Algorithm::ES384],
            _ => &[],
        },
        (None, AlgorithmParameters::OctetKeyPair(p)) => match p.curve {
            EllipticCurve::Ed25519 => &[Algorithm::EdDSA],
            _ => &[],
        },
        (None, _) => &[],
    };

    if allowed.contains(&token_alg) {
        Ok(token_alg)
    } else {
        Err(format!(
            "token algorithm {token_alg:?} not allowed for this key type"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> OidcConfig {
        OidcConfig {
            issuer: "https://example.com".into(),
            audience: "test".into(),
            allowed_subjects: vec![],
            allowed_emails: vec![],
        }
    }

    fn claims(sub: &str, email: Option<&str>, verified: Option<bool>) -> Claims {
        Claims {
            sub: sub.into(),
            iss: "https://example.com".into(),
            aud: serde_json::json!("test"),
            exp: 0,
            iat: None,
            email: email.map(Into::into),
            email_verified: verified,
        }
    }

    fn rsa_jwk(alg: Option<&str>) -> Jwk {
        let mut json = serde_json::json!({
            "kty": "RSA",
            "kid": "k1",
            "n": "sXchDaQebHnPiGvyDOAT4saGEUetSyo9MKLOoWFsueri23bOdgWp4Dy1WlUzewbgBHod5pcM9H95GQRV3JDXboIRROSBigeC5yjU1hGzHHyXss8UDprecbAYxknTcQkhslANGRUZmdTOQ5qTRsLAt6BTYuyvVRdhS8exSZEy_c4gs_7svlJJQ4H9_NxsiIoLwAEk7-Q3UXERGYw_75IDrGA84-lA_-Ct4eTlXHBIY2EaV7t7LjJaynVJCpkv4LKjTTAumiGUIuQhrNhZLuF_RJLqHpM2kgWFLU7-VTdL1VbC2tejvcI2BlMkEpk1BzBZI0KQB0GaDWFLN-aEAw3vRw",
            "e": "AQAB"
        });
        if let Some(a) = alg {
            json["alg"] = serde_json::json!(a);
        }
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn oidc_config_serde() {
        let json = r#"{
            "issuer": "https://accounts.google.com",
            "audience": "my-client-id"
        }"#;
        let config: OidcConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.issuer, "https://accounts.google.com");
        assert_eq!(config.audience, "my-client-id");
        assert!(config.allows_any_account());
    }

    #[test]
    fn oidc_config_rejects_unknown_fields() {
        // A typo must not silently drop a setting (e.g. the allowlist).
        let json = r#"{"issuer_url": "https://x", "audience": "a"}"#;
        assert!(serde_json::from_str::<OidcConfig>(json).is_err());
        let json = r#"{"issuer": "https://x", "audience": "a", "allowed_email": ["a@b.c"]}"#;
        assert!(serde_json::from_str::<OidcConfig>(json).is_err());
    }

    #[test]
    fn allowlist_by_subject() {
        let mut cfg = config();
        cfg.allowed_subjects = vec!["user-1".into()];
        assert!(cfg.is_authorized(&claims("user-1", None, None)));
        assert!(!cfg.is_authorized(&claims("user-2", None, None)));
    }

    #[test]
    fn allowlist_by_email_requires_verification() {
        let mut cfg = config();
        cfg.allowed_emails = vec!["Owner@Example.com".into()];
        assert!(cfg.is_authorized(&claims("x", Some("owner@example.com"), Some(true))));
        assert!(!cfg.is_authorized(&claims("x", Some("owner@example.com"), Some(false))));
        assert!(!cfg.is_authorized(&claims("x", Some("owner@example.com"), None)));
        assert!(!cfg.is_authorized(&claims("x", Some("other@example.com"), Some(true))));
    }

    #[test]
    fn no_allowlist_allows_any_account() {
        assert!(config().is_authorized(&claims("anyone", None, None)));
    }

    #[test]
    fn jwks_cache_creation() {
        let _cache = JwksCache::new(config());
    }

    #[test]
    fn claims_deserialize() {
        let json = r#"{
            "sub": "user123",
            "iss": "https://example.com",
            "aud": "my-app",
            "exp": 9999999999,
            "email": "user@example.com",
            "email_verified": "true"
        }"#;
        let claims: Claims = serde_json::from_str(json).unwrap();
        assert_eq!(claims.sub, "user123");
        assert_eq!(claims.email.unwrap(), "user@example.com");
        assert_eq!(claims.email_verified, Some(true));
    }

    #[test]
    fn invalid_token_rejected() {
        let cache = JwksCache::new(config());
        let jwks = JwkSet { keys: vec![] };
        let result = cache.try_validate("not-a-jwt", &jwks);
        assert!(result.is_err());
    }

    #[test]
    fn hs256_algorithm_rejected() {
        let cache = JwksCache::new(config());
        let jwks = JwkSet { keys: vec![] };
        // Craft a token header with HS256 (base64url encoded {"alg":"HS256","typ":"JWT"})
        let fake_token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxIn0.sig";
        match cache.try_validate(fake_token, &jwks) {
            Err(ValidateError::Invalid(msg)) => assert!(msg.contains("not allowed")),
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[test]
    fn unknown_kid_is_reported_for_refresh() {
        let cache = JwksCache::new(config());
        let jwks = JwkSet { keys: vec![] };
        // {"alg":"RS256","kid":"missing"}
        let token = "eyJhbGciOiJSUzI1NiIsImtpZCI6Im1pc3NpbmcifQ.eyJzdWIiOiIxIn0.sig";
        assert!(matches!(
            cache.try_validate(token, &jwks),
            Err(ValidateError::UnknownKid(k)) if k == "missing"
        ));
    }

    #[test]
    fn algorithm_comes_from_the_key() {
        // Key pins RS256: a PS256 token is refused even though PS256 is RSA.
        let pinned = rsa_jwk(Some("RS256"));
        assert_eq!(
            algorithm_for_key(&pinned, Algorithm::RS256).unwrap(),
            Algorithm::RS256
        );
        assert!(algorithm_for_key(&pinned, Algorithm::PS256).is_err());

        // No alg on the key: anything from the RSA family, nothing else.
        let unpinned = rsa_jwk(None);
        assert!(algorithm_for_key(&unpinned, Algorithm::RS384).is_ok());
        assert!(algorithm_for_key(&unpinned, Algorithm::ES256).is_err());
        assert!(algorithm_for_key(&unpinned, Algorithm::HS256).is_err());
    }

    #[tokio::test]
    async fn failed_refresh_is_rate_limited() {
        // Unroutable issuer: the first attempt fails, and the next one must not
        // go out again (no outbound request per inbound request).
        let mut cfg = config();
        cfg.issuer = "http://127.0.0.1:1".into();
        let cache = JwksCache::new(cfg);
        let first = cache.refresh_keys(0).await.unwrap_err();
        assert!(!first.contains("rate limited"), "{first}");
        let second = cache.refresh_keys(0).await.unwrap_err();
        assert!(second.contains("rate limited"), "{second}");
    }
}
