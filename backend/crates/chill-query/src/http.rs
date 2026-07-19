use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::post,
};
use chill_control_plane::{Capability, ControlPlaneError, Store};
use serde::Deserialize;

use crate::{CatalogError, Plan, Scope, Service, ServiceError};

const MAXIMUM_QUERY_REQUEST_BYTES: usize = 1 << 20;

#[derive(Clone)]
struct QueryState {
    control: Store,
    query: Arc<Service>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryRequest {
    project_id: String,
    environment_id: String,
    plan: Plan,
}

/// Builds the authenticated typed query endpoint.
pub fn query_router(control: Store, query: Arc<Service>) -> Router {
    Router::new()
        .route("/v1/query", post(execute))
        .layer(DefaultBodyLimit::max(MAXIMUM_QUERY_REQUEST_BYTES))
        .with_state(QueryState { control, query })
}

async fn execute(
    State(state): State<QueryState>,
    headers: HeaderMap,
    Json(body): Json<QueryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let session = bearer_session(&headers)?;
    let principal = state
        .control
        .require_user_capability(session, Capability::DataRead)
        .await?;
    let scope = Scope {
        organization_id: principal.organization_id,
        project_id: body.project_id,
        environment_id: body.environment_id,
    };
    Ok(Json(state.query.execute(&scope, &body.plan).await?))
}

fn bearer_session(headers: &HeaderMap) -> Result<&str, ApiError> {
    let values: Vec<_> = headers.get_all(AUTHORIZATION).iter().collect();
    if values.len() != 1 {
        return Err(ApiError::Unauthorized);
    }
    let value = values[0].to_str().map_err(|_| ApiError::Unauthorized)?;
    let credential = value
        .strip_prefix("Bearer ")
        .filter(|value| !value.is_empty() && !value.contains(char::is_whitespace))
        .ok_or(ApiError::Unauthorized)?;
    Ok(credential)
}

#[derive(Debug)]
enum ApiError {
    Unauthorized,
    Forbidden,
    Invalid,
    NotFound,
    Busy,
    Limited,
    Deadline,
    Internal,
}

impl From<ControlPlaneError> for ApiError {
    fn from(value: ControlPlaneError) -> Self {
        match value {
            ControlPlaneError::Unauthorized => Self::Unauthorized,
            ControlPlaneError::Forbidden => Self::Forbidden,
            ControlPlaneError::InvalidInput(_) => Self::Invalid,
            ControlPlaneError::Conflict
            | ControlPlaneError::Database(_)
            | ControlPlaneError::Credential(_) => Self::Internal,
        }
    }
}

impl From<ServiceError> for ApiError {
    fn from(value: ServiceError) -> Self {
        match value {
            ServiceError::Invalid(_) | ServiceError::InvalidConfiguration => Self::Invalid,
            ServiceError::Catalog(CatalogError::ScopeNotFound) => Self::NotFound,
            ServiceError::Busy => Self::Busy,
            ServiceError::ResourceLimit => Self::Limited,
            ServiceError::Deadline => Self::Deadline,
            ServiceError::Catalog(_)
            | ServiceError::Files(_)
            | ServiceError::Results(_)
            | ServiceError::Engine(_)
            | ServiceError::Join(_)
            | ServiceError::CacheKey => Self::Internal,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::Invalid => (StatusCode::BAD_REQUEST, "invalid_query"),
            Self::NotFound => (StatusCode::NOT_FOUND, "query_scope_unavailable"),
            Self::Busy => (StatusCode::TOO_MANY_REQUESTS, "query_busy"),
            Self::Limited => (StatusCode::UNPROCESSABLE_ENTITY, "query_resource_limit"),
            Self::Deadline => (StatusCode::GATEWAY_TIMEOUT, "query_deadline"),
            Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "query_failed"),
        };
        (status, Json(serde_json::json!({ "error": code }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_parser_rejects_ambiguity_and_padding()
    -> Result<(), axum::http::header::InvalidHeaderValue> {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Bearer ch_us_example".parse()?);
        assert!(matches!(bearer_session(&headers), Ok("ch_us_example")));
        headers.append(AUTHORIZATION, "Bearer second".parse()?);
        assert!(matches!(
            bearer_session(&headers),
            Err(ApiError::Unauthorized)
        ));
        Ok(())
    }
}
