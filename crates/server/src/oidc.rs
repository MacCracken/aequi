use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcConfig {
    /// OIDC issuer URL (e.g. `https://accounts.google.com`)
    pub issuer: String,
    /// Expected audience (client ID)
    pub audience: String,
}

/// Allowed JWT signing algorithms (pinned — prevents algorithm confusion attacks).
const ALLOWED_ALGORITHMS: &[Algorithm] = &[Algorithm::RS256, Algorithm::RS384, Algorithm::RS512];

/// Minimum interval between JWKS refreshes to prevent DoS via invalid tokens.
const MIN_REFRESH_INTERVAL_SECS: u64 = 300; // 5 minutes

/// Cached JWKS keys for token validation.
#[derive(Clone)]
pub struct JwksCache {
    config: OidcConfig,
    keys: Arc<RwLock<Option<jsonwebtoken::jwk::JwkSet>>>,
    last_refresh: Arc<RwLock<Option<std::time::Instant>>>,
}

#[derive(Debug, Deserialize)]
struct OidcDiscovery {
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
}

impl JwksCache {
    pub fn new(config: OidcConfig) -> Self {
        Self {
            config,
            keys: Arc::new(RwLock::new(None)),
            last_refresh: Arc::new(RwLock::new(None)),
        }
    }

    /// Check if enough time has passed since the last JWKS refresh.
    async fn can_refresh(&self) -> bool {
        let last = self.last_refresh.read().await;
        match *last {
            None => true,
            Some(t) => t.elapsed().as_secs() >= MIN_REFRESH_INTERVAL_SECS,
        }
    }

    /// Fetch JWKS from the issuer's discovery endpoint.
    async fn refresh_keys(&self) -> Result<jsonwebtoken::jwk::JwkSet, String> {
        if !self.can_refresh().await {
            return Err("JWKS refresh rate limited".to_string());
        }

        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            self.config.issuer.trim_end_matches('/')
        );

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| format!("HTTP client error: {e}"))?;

        let discovery: OidcDiscovery = client
            .get(&discovery_url)
            .send()
            .await
            .map_err(|_| "OIDC discovery failed".to_string())?
            .json()
            .await
            .map_err(|_| "OIDC discovery parse failed".to_string())?;

        let jwks: jsonwebtoken::jwk::JwkSet = client
            .get(&discovery.jwks_uri)
            .send()
            .await
            .map_err(|_| "JWKS fetch failed".to_string())?
            .json()
            .await
            .map_err(|_| "JWKS parse failed".to_string())?;

        let mut cache = self.keys.write().await;
        *cache = Some(jwks.clone());

        let mut last = self.last_refresh.write().await;
        *last = Some(std::time::Instant::now());

        Ok(jwks)
    }

    /// Validate a JWT token. Returns claims on success.
    pub async fn validate_token(&self, token: &str) -> Result<Claims, String> {
        // Try cached keys first, refresh if needed
        let jwks = {
            let cache = self.keys.read().await;
            cache.clone()
        };

        let jwks = match jwks {
            Some(keys) => keys,
            None => self.refresh_keys().await?,
        };

        match self.try_validate(token, &jwks) {
            Ok(claims) => Ok(claims),
            Err(_) => {
                // Keys might be rotated — refresh and retry once (rate-limited)
                if self.can_refresh().await {
                    let refreshed = self.refresh_keys().await?;
                    self.try_validate(token, &refreshed)
                } else {
                    Err("JWT validation failed and JWKS refresh rate limited".to_string())
                }
            }
        }
    }

    fn try_validate(
        &self,
        token: &str,
        jwks: &jsonwebtoken::jwk::JwkSet,
    ) -> Result<Claims, String> {
        let header =
            jsonwebtoken::decode_header(token).map_err(|e| format!("invalid JWT header: {e}"))?;

        // Reject algorithms not in the allowed list (prevents algorithm confusion)
        if !ALLOWED_ALGORITHMS.contains(&header.alg) {
            return Err(format!("algorithm {:?} not allowed", header.alg));
        }

        let kid = header.kid.as_ref().ok_or("JWT missing kid header")?;

        let jwk = jwks
            .find(kid)
            .ok_or_else(|| format!("no matching key for kid: {kid}"))?;

        let decoding_key = DecodingKey::from_jwk(jwk).map_err(|e| format!("invalid JWK: {e}"))?;

        let mut validation = Validation::new(header.alg);
        validation.set_audience(&[&self.config.audience]);
        validation.set_issuer(&[&self.config.issuer]);
        validation.leeway = 60; // 60 second clock skew tolerance

        let data = decode::<Claims>(token, &decoding_key, &validation)
            .map_err(|e| format!("JWT validation failed: {e}"))?;

        Ok(data.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oidc_config_serde() {
        let json = r#"{
            "issuer": "https://accounts.google.com",
            "audience": "my-client-id"
        }"#;
        let config: OidcConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.issuer, "https://accounts.google.com");
        assert_eq!(config.audience, "my-client-id");
    }

    #[test]
    fn jwks_cache_creation() {
        let config = OidcConfig {
            issuer: "https://example.com".into(),
            audience: "test".into(),
        };
        let _cache = JwksCache::new(config);
    }

    #[test]
    fn claims_deserialize() {
        let json = r#"{
            "sub": "user123",
            "iss": "https://example.com",
            "aud": "my-app",
            "exp": 9999999999,
            "email": "user@example.com"
        }"#;
        let claims: Claims = serde_json::from_str(json).unwrap();
        assert_eq!(claims.sub, "user123");
        assert_eq!(claims.email.unwrap(), "user@example.com");
    }

    #[test]
    fn invalid_token_rejected() {
        let config = OidcConfig {
            issuer: "https://example.com".into(),
            audience: "test".into(),
        };
        let cache = JwksCache::new(config);
        let jwks = jsonwebtoken::jwk::JwkSet { keys: vec![] };
        let result = cache.try_validate("not-a-jwt", &jwks);
        assert!(result.is_err());
    }

    #[test]
    fn hs256_algorithm_rejected() {
        let config = OidcConfig {
            issuer: "https://example.com".into(),
            audience: "test".into(),
        };
        let cache = JwksCache::new(config);
        let jwks = jsonwebtoken::jwk::JwkSet { keys: vec![] };
        // Craft a token header with HS256 (base64url encoded {"alg":"HS256","typ":"JWT"})
        let fake_token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxIn0.sig";
        let result = cache.try_validate(fake_token, &jwks);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not allowed"));
    }
}
