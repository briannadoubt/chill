use std::time::Duration;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq as _;

use crate::{ControlPlaneError, Store, VerifiedIdentity};

const AUTHORIZATION_HEADER: &str = "x-chill-sites-auth";
const IDENTITY_ISSUER: &str = "chill.email";
const MAXIMUM_REQUEST_BYTES: usize = 2048;
const SESSION_LIFETIME: Duration = Duration::from_hours(8);

/// Server-only configuration for exchanging an authenticated private-Sites identity.
#[derive(Clone)]
pub struct SitesAuthentication {
    secret_digest: [u8; 32],
}

impl SitesAuthentication {
    /// Validates and hashes the server-to-server shared secret.
    ///
    /// # Errors
    ///
    /// Returns an input error when the secret is too short or unreasonably large.
    pub fn new(secret: &str) -> Result<Self, ControlPlaneError> {
        if !(32..=512).contains(&secret.len()) || secret.trim() != secret {
            return Err(ControlPlaneError::InvalidInput(
                "Sites authentication secret is invalid".to_owned(),
            ));
        }
        Ok(Self {
            secret_digest: Sha256::digest(secret.as_bytes()).into(),
        })
    }

    fn authenticates(&self, candidate: &str) -> bool {
        let candidate_digest: [u8; 32] = Sha256::digest(candidate.as_bytes()).into();
        bool::from(self.secret_digest.ct_eq(&candidate_digest))
    }
}

/// Builds the narrowly scoped private-Sites identity exchange endpoint.
pub fn sites_auth_router(store: Store, authentication: SitesAuthentication) -> Router {
    Router::new()
        .route("/v1/auth/sites-session", post(exchange_session))
        .layer(DefaultBodyLimit::max(MAXIMUM_REQUEST_BYTES))
        .with_state(SitesAuthState {
            store,
            authentication,
        })
}

#[derive(Clone)]
struct SitesAuthState {
    store: Store,
    authentication: SitesAuthentication,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExchangeRequest {
    email: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExchangeResponse {
    credential: String,
    expires_at: String,
}

async fn exchange_session(
    State(state): State<SitesAuthState>,
    headers: HeaderMap,
    Json(body): Json<ExchangeRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let candidate = single_header(&headers, AUTHORIZATION_HEADER)?;
    if !state.authentication.authenticates(candidate) {
        return Err(ApiError(ControlPlaneError::Unauthorized));
    }
    let issued = state
        .store
        .issue_user_session_for_verified_identity(
            &VerifiedIdentity {
                issuer: IDENTITY_ISSUER.to_owned(),
                subject: body.email,
            },
            SESSION_LIFETIME,
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        [("cache-control", HeaderValue::from_static("no-store"))],
        Json(ExchangeResponse {
            credential: issued.credential,
            expires_at: issued.expires_at.to_string(),
        }),
    ))
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, ApiError> {
    let values: Vec<_> = headers.get_all(name).iter().collect();
    if values.len() != 1 {
        return Err(ApiError(ControlPlaneError::Unauthorized));
    }
    let value = values[0]
        .to_str()
        .map_err(|_| ApiError(ControlPlaneError::Unauthorized))?;
    if value.is_empty() || value.trim() != value {
        return Err(ApiError(ControlPlaneError::Unauthorized));
    }
    Ok(value)
}

struct ApiError(ControlPlaneError);

impl From<ControlPlaneError> for ApiError {
    fn from(error: ControlPlaneError) -> Self {
        Self(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            ControlPlaneError::Unauthorized => StatusCode::UNAUTHORIZED,
            ControlPlaneError::InvalidInput(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ControlPlaneError::Forbidden => StatusCode::FORBIDDEN,
            ControlPlaneError::Conflict => StatusCode::CONFLICT,
            ControlPlaneError::Database(_) | ControlPlaneError::Credential(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, "authentication failed").into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_validation_and_comparison_are_strict() {
        for invalid in ["short", " 01234567890123456789012345678901", ""] {
            assert!(SitesAuthentication::new(invalid).is_err());
        }
        let secret = "01234567890123456789012345678901";
        let authentication = SitesAuthentication::new(secret)
            .unwrap_or_else(|_| unreachable!("test secret is valid"));
        assert!(authentication.authenticates(secret));
        assert!(!authentication.authenticates("11234567890123456789012345678901"));
    }

    #[test]
    fn header_parser_requires_one_unpadded_value() {
        let mut headers = HeaderMap::new();
        assert!(single_header(&headers, AUTHORIZATION_HEADER).is_err());
        headers.append(AUTHORIZATION_HEADER, HeaderValue::from_static("first"));
        headers.append(AUTHORIZATION_HEADER, HeaderValue::from_static("second"));
        assert!(single_header(&headers, AUTHORIZATION_HEADER).is_err());
        headers = HeaderMap::new();
        headers.insert(AUTHORIZATION_HEADER, HeaderValue::from_static(" secret "));
        assert!(single_header(&headers, AUTHORIZATION_HEADER).is_err());
        headers.insert(AUTHORIZATION_HEADER, HeaderValue::from_static("secret"));
        assert_eq!(
            single_header(&headers, AUTHORIZATION_HEADER).ok(),
            Some("secret")
        );
    }
}
