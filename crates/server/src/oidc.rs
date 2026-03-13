use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcConfig {
    /// OIDC issuer URL (e.g. "https://accounts.google.com")
    pub issuer: String,
    /// Expected audience (client ID)
    pub audience: String,
}

/// Cached JWKS keys for token validation.
#[derive(Clone)]
pub struct JwksCache {
    config: OidcConfig,
    keys: Arc<RwLock<Option<jsonwebtoken::jwk::JwkSet>>>,
}

#[derive(Debug, Deserialize)]
struct OidcDiscovery {
    jwks_uri: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub iss: String,
    pub aud: serde_json::Value,
    pub exp: u64,
    #[serde(default)]
    pub email: Option<String>,
}

impl JwksCache {
    pub fn new(config: OidcConfig) -> Self {
        Self {
            config,
            keys: Arc::new(RwLock::new(None)),
        }
    }

    /// Fetch JWKS from the issuer's discovery endpoint.
    async fn refresh_keys(&self) -> Result<jsonwebtoken::jwk::JwkSet, String> {
        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            self.config.issuer.trim_end_matches('/')
        );

        let client = reqwest::Client::new();
        let discovery: OidcDiscovery = client
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| format!("OIDC discovery failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("OIDC discovery parse failed: {e}"))?;

        let jwks: jsonwebtoken::jwk::JwkSet = client
            .get(&discovery.jwks_uri)
            .send()
            .await
            .map_err(|e| format!("JWKS fetch failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("JWKS parse failed: {e}"))?;

        let mut cache = self.keys.write().await;
        *cache = Some(jwks.clone());

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
                // Keys might be rotated — refresh and retry once
                let refreshed = self.refresh_keys().await?;
                self.try_validate(token, &refreshed)
            }
        }
    }

    fn try_validate(
        &self,
        token: &str,
        jwks: &jsonwebtoken::jwk::JwkSet,
    ) -> Result<Claims, String> {
        let header = jsonwebtoken::decode_header(token)
            .map_err(|e| format!("invalid JWT header: {e}"))?;

        let kid = header
            .kid
            .as_ref()
            .ok_or("JWT missing kid header")?;

        let jwk = jwks
            .find(kid)
            .ok_or_else(|| format!("no matching key for kid: {kid}"))?;

        let alg = header.alg;
        let decoding_key = DecodingKey::from_jwk(jwk)
            .map_err(|e| format!("invalid JWK: {e}"))?;

        let mut validation = Validation::new(alg);
        validation.set_audience(&[&self.config.audience]);
        validation.set_issuer(&[&self.config.issuer]);

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
}
