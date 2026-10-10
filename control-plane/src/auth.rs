//! OpenID Connect (OIDC) JWT and Internal Gateway Authentication
//! Validates Bearer JWTs, signatures, exp, iss, aud, and internal gateway headers.

use axum::http::HeaderMap;
use chrono::Utc;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::models::OidcClaims;
use crate::service::ServiceError;

pub const DEFAULT_DEV_JWT_SECRET: &[u8] = b"mesh-control-plane-dev-insecure-secret-key-32b";

#[derive(Debug, Serialize, Deserialize)]
struct ClaimsWrapper {
    pub sub: String,
    pub email: String,
    pub tenant_id: String,
    pub roles: Vec<String>,
    pub exp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
}

impl From<OidcClaims> for ClaimsWrapper {
    fn from(c: OidcClaims) -> Self {
        Self {
            sub: c.sub,
            email: c.email,
            tenant_id: c.tenant_id,
            roles: c.roles,
            exp: c.exp,
            iss: c.iss,
            aud: c.aud,
        }
    }
}

impl From<ClaimsWrapper> for OidcClaims {
    fn from(w: ClaimsWrapper) -> Self {
        Self {
            sub: w.sub,
            email: w.email,
            tenant_id: w.tenant_id,
            roles: w.roles,
            exp: w.exp,
            iss: w.iss,
            aud: w.aud,
        }
    }
}

/// Generates a signed HS256 JWT token with the specified claims.
pub fn create_jwt(claims: &OidcClaims, secret: &[u8]) -> Result<String, ServiceError> {
    let wrapper = ClaimsWrapper::from(claims.clone());
    encode(
        &Header::default(),
        &wrapper,
        &EncodingKey::from_secret(secret),
    )
    .map_err(|e| ServiceError::Internal(format!("Failed to encode JWT: {}", e)))
}

/// Verifies a signed HS256 JWT token against the configured secret and validation rules.
pub fn verify_jwt(token: &str, secret: &[u8]) -> Result<OidcClaims, ServiceError> {
    let mut validation = Validation::default();
    validation.validate_exp = true;
    validation.leeway = 0;

    if let Ok(expected_iss) = std::env::var("OIDC_ISSUER") {
        validation.set_issuer(&[expected_iss]);
    }
    if let Ok(expected_aud) = std::env::var("OIDC_AUDIENCE") {
        validation.set_audience(&[expected_aud]);
    }

    let token_data = decode::<ClaimsWrapper>(
        token,
        &DecodingKey::from_secret(secret),
        &validation,
    )
    .map_err(|e| ServiceError::Unauthorized(format!("Invalid or expired JWT: {}", e)))?;

    Ok(token_data.claims.into())
}

/// Authenticates incoming HTTP request headers.
///
/// Priority:
/// 1. `Authorization: Bearer <JWT>` -> Cryptographically verified against `OIDC_JWT_SECRET`.
/// 2. `x-oidc-*` headers -> Only accepted if accompanied by a valid `x-internal-gateway-secret`
///    matching `INTERNAL_GATEWAY_SECRET` (trusted reverse proxy / API gateway integration).
/// 3. Offline test mode -> If `MESH_ALLOW_TEST_CLAIMS=1`, allows header fallback for test fixtures.
/// 4. Otherwise -> Rejects with HTTP 401 Unauthorized.
pub fn authenticate_claims(headers: &HeaderMap) -> Result<OidcClaims, ServiceError> {
    // 1. Check Bearer Token
    if let Some(auth_val) = headers.get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
    {
        let secret = std::env::var("OIDC_JWT_SECRET")
            .map(|s| s.into_bytes())
            .unwrap_or_else(|_| DEFAULT_DEV_JWT_SECRET.to_vec());

        return verify_jwt(auth_val.trim(), &secret);
    }

    // 2. Check Trusted Internal Gateway Authentication
    let gateway_secret_env = std::env::var("INTERNAL_GATEWAY_SECRET").unwrap_or_default();
    let provided_gateway_secret = headers
        .get("x-internal-gateway-secret")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();

    let is_trusted_gateway = !gateway_secret_env.is_empty()
        && provided_gateway_secret == gateway_secret_env;

    // 3. Check explicit test mode override
    let allow_test_claims = std::env::var("MESH_ALLOW_TEST_CLAIMS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if is_trusted_gateway || allow_test_claims {
        let sub = headers
            .get("x-oidc-sub")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("gateway_sub")
            .to_string();

        let email = headers
            .get("x-oidc-email")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("gateway@meshstorage.cloud")
            .to_string();

        let tenant_id = headers
            .get("x-oidc-tenant")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("default_tenant")
            .to_string();

        let roles: Vec<String> = headers
            .get("x-oidc-roles")
            .and_then(|h| h.to_str().ok())
            .map(|r| r.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_else(|| vec!["member".to_string()]);

        return Ok(OidcClaims {
            sub,
            email,
            tenant_id,
            roles,
            exp: Utc::now().timestamp() + 3600,
            iss: Some("https://internal.gateway.meshstorage.io".to_string()),
            aud: Some("mesh-control-plane".to_string()),
        });
    }

    // 4. Client attempted to send x-oidc-* headers directly without gateway secret
    if headers.contains_key("x-oidc-sub") || headers.contains_key("x-oidc-tenant") {
        return Err(ServiceError::Unauthorized(
            "Spoofing detected: x-oidc-* headers are forbidden unless authenticated via trusted internal gateway".to_string(),
        ));
    }

    Err(ServiceError::Unauthorized(
        "Missing Authorization header. Send 'Authorization: Bearer <JWT>'".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_create_and_verify_roundtrip() {
        let secret = b"my-super-secret-signing-key-12345";
        let claims = OidcClaims {
            sub: "user-alice".to_string(),
            email: "alice@acme.com".to_string(),
            tenant_id: "ten_acme".to_string(),
            roles: vec!["admin".to_string()],
            exp: Utc::now().timestamp() + 3600,
            iss: None,
            aud: None,
        };

        let token = create_jwt(&claims, secret).expect("create JWT");
        let decoded = verify_jwt(&token, secret).expect("verify JWT");

        assert_eq!(decoded.sub, "user-alice");
        assert_eq!(decoded.email, "alice@acme.com");
        assert_eq!(decoded.tenant_id, "ten_acme");
        assert!(decoded.has_role("admin"));
    }

    #[test]
    fn test_expired_jwt_rejected() {
        let secret = b"my-super-secret-signing-key-12345";
        let claims = OidcClaims {
            sub: "user-bob".to_string(),
            email: "bob@acme.com".to_string(),
            tenant_id: "ten_acme".to_string(),
            roles: vec!["member".to_string()],
            exp: Utc::now().timestamp() - 60, // Already expired
            iss: None,
            aud: None,
        };

        let token = create_jwt(&claims, secret).expect("create JWT");
        let res = verify_jwt(&token, secret);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Invalid or expired JWT"));
    }

    #[test]
    fn test_tampered_jwt_signature_rejected() {
        let secret = b"my-super-secret-signing-key-12345";
        let wrong_secret = b"wrong-attacker-secret-key-99999";
        let claims = OidcClaims {
            sub: "user-mallory".to_string(),
            email: "mallory@evil.com".to_string(),
            tenant_id: "ten_victim".to_string(),
            roles: vec!["superadmin".to_string()],
            exp: Utc::now().timestamp() + 3600,
            iss: None,
            aud: None,
        };

        let token = create_jwt(&claims, wrong_secret).expect("create JWT with attacker key");
        let res = verify_jwt(&token, secret);
        assert!(res.is_err());
    }

    #[test]
    fn test_authenticate_claims_blocks_spoofed_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-oidc-sub", "attacker".parse().unwrap());
        headers.insert("x-oidc-tenant", "victim_corp".parse().unwrap());
        headers.insert("x-oidc-roles", "superadmin".parse().unwrap());

        let res = authenticate_claims(&headers);
        assert!(res.is_err());
        let err_str = res.unwrap_err().to_string();
        assert!(err_str.contains("Spoofing detected"));
    }

    #[test]
    fn test_authenticate_claims_with_trusted_gateway() {
        unsafe {
            std::env::set_var("INTERNAL_GATEWAY_SECRET", "gateway-token-abc");
        }

        let mut headers = HeaderMap::new();
        headers.insert("x-internal-gateway-secret", "gateway-token-abc".parse().unwrap());
        headers.insert("x-oidc-sub", "verified_user".parse().unwrap());
        headers.insert("x-oidc-tenant", "ten_verified".parse().unwrap());
        headers.insert("x-oidc-roles", "admin".parse().unwrap());

        let res = authenticate_claims(&headers).expect("gateway authenticated");
        assert_eq!(res.sub, "verified_user");
        assert_eq!(res.tenant_id, "ten_verified");
        assert!(res.has_role("admin"));

        unsafe {
            std::env::remove_var("INTERNAL_GATEWAY_SECRET");
        }
    }
}
