use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, HeaderValue, Request, StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post},
};
use serde::Deserialize;

use crate::{
    ActivatePrivacyRequest, ActivateSamplingRequest, AnalyticsScope, ControlPlaneError,
    CreateAlertRequest, CreateDashboardRequest, CreateDataSourceRequest, CreateEnvironmentRequest,
    CreateProjectRequest, CreateSDKKeyRequest, CreateSavedQueryRequest, CreateSchemaRequest, Store,
    UpdateDashboardRequest, UpdateSavedQueryRequest,
};

const MAXIMUM_CONSOLE_REQUEST_BYTES: usize = 1 << 20;

/// Builds the authenticated project-console API with security headers and a 1 MiB body cap.
pub fn console_router(store: Store) -> Router {
    Router::new()
        .route("/v1/console/overview", get(overview))
        .route("/v1/console/projects", post(create_project))
        .route("/v1/console/environments", post(create_environment))
        .route(
            "/v1/console/environments/{environment_id}/retention",
            patch(update_retention),
        )
        .route("/v1/console/data-sources", post(create_data_source))
        .route("/v1/console/sdk-keys", post(create_sdk_key))
        .route("/v1/console/sdk-keys/{key_id}/rotate", post(rotate_sdk_key))
        .route("/v1/console/sdk-keys/{key_id}", delete(revoke_sdk_key))
        .route("/v1/console/sampling", post(activate_sampling))
        .route("/v1/console/privacy", post(activate_privacy))
        .route("/v1/console/schemas", post(create_schema))
        .route("/v1/console/analytics", get(analytics_workspace))
        .route("/v1/console/debugger", get(debugger_snapshot))
        .route("/v1/console/saved-queries", post(create_saved_query))
        .route("/v1/console/saved-queries/{id}", patch(update_saved_query))
        .route("/v1/console/dashboards", post(create_dashboard))
        .route("/v1/console/dashboards/{id}", patch(update_dashboard))
        .route("/v1/console/alerts", post(create_alert))
        .route(
            "/v1/console/analytics/{kind}/{id}",
            delete(archive_analytics),
        )
        .layer(DefaultBodyLimit::max(MAXIMUM_CONSOLE_REQUEST_BYTES))
        .layer(middleware::from_fn(security_headers))
        .with_state(store)
}

async fn analytics_workspace(
    State(store): State<Store>,
    headers: HeaderMap,
    axum::extract::Query(scope): axum::extract::Query<AnalyticsScope>,
) -> Result<impl IntoResponse, ApiError> {
    let session = bearer_session(&headers)?;
    Ok(Json(store.analytics_workspace(session, scope).await?))
}
async fn debugger_snapshot(
    State(store): State<Store>,
    headers: HeaderMap,
    axum::extract::Query(scope): axum::extract::Query<AnalyticsScope>,
) -> Result<impl IntoResponse, ApiError> {
    let session = bearer_session(&headers)?;
    Ok(Json(store.debugger_snapshot(session, scope).await?))
}
async fn create_saved_query(
    State(store): State<Store>,
    headers: HeaderMap,
    Json(body): Json<CreateSavedQueryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let session = bearer_session(&headers)?;
    Ok((
        StatusCode::CREATED,
        Json(store.create_saved_query(session, body).await?),
    ))
}
async fn create_dashboard(
    State(store): State<Store>,
    headers: HeaderMap,
    Json(body): Json<CreateDashboardRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let session = bearer_session(&headers)?;
    Ok((
        StatusCode::CREATED,
        Json(store.create_dashboard(session, body).await?),
    ))
}
async fn update_saved_query(
    State(store): State<Store>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<UpdateSavedQueryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let session = bearer_session(&headers)?;
    Ok(Json(store.update_saved_query(session, &id, body).await?))
}
async fn update_dashboard(
    State(store): State<Store>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<UpdateDashboardRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let session = bearer_session(&headers)?;
    Ok(Json(store.update_dashboard(session, &id, body).await?))
}
async fn create_alert(
    State(store): State<Store>,
    headers: HeaderMap,
    Json(body): Json<CreateAlertRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let session = bearer_session(&headers)?;
    Ok((
        StatusCode::CREATED,
        Json(store.create_alert(session, body).await?),
    ))
}
async fn archive_analytics(
    State(store): State<Store>,
    Path((kind, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let session = bearer_session(&headers)?;
    store
        .archive_analytics_resource(session, &kind, &id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_project(
    State(store): State<Store>,
    headers: HeaderMap,
    Json(body): Json<CreateProjectRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let session = bearer_session(&headers)?;
    Ok((
        StatusCode::CREATED,
        Json(store.create_project(session, body).await?),
    ))
}

async fn create_environment(
    State(store): State<Store>,
    headers: HeaderMap,
    Json(body): Json<CreateEnvironmentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let session = bearer_session(&headers)?;
    Ok((
        StatusCode::CREATED,
        Json(store.create_environment(session, body).await?),
    ))
}

async fn overview(
    State(store): State<Store>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let session = bearer_session(&headers)?;
    Ok(Json(store.console_overview(session).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetentionBody {
    retention_days: i32,
}

async fn update_retention(
    State(store): State<Store>,
    Path(environment_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<RetentionBody>,
) -> Result<StatusCode, ApiError> {
    let session = bearer_session(&headers)?;
    store
        .update_environment_retention(session, &environment_id, body.retention_days)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_data_source(
    State(store): State<Store>,
    headers: HeaderMap,
    Json(body): Json<CreateDataSourceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let session = bearer_session(&headers)?;
    Ok((
        StatusCode::CREATED,
        Json(store.create_data_source(session, body).await?),
    ))
}

async fn create_sdk_key(
    State(store): State<Store>,
    headers: HeaderMap,
    Json(body): Json<CreateSDKKeyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let session = bearer_session(&headers)?;
    Ok((
        StatusCode::CREATED,
        Json(store.create_sdk_key(session, body).await?),
    ))
}

async fn rotate_sdk_key(
    State(store): State<Store>,
    Path(key_id): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let session = bearer_session(&headers)?;
    Ok((
        StatusCode::CREATED,
        Json(store.rotate_sdk_key(session, &key_id).await?),
    ))
}

async fn revoke_sdk_key(
    State(store): State<Store>,
    Path(key_id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let session = bearer_session(&headers)?;
    store.revoke_sdk_key(session, &key_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn activate_sampling(
    State(store): State<Store>,
    headers: HeaderMap,
    Json(body): Json<ActivateSamplingRequest>,
) -> Result<StatusCode, ApiError> {
    let session = bearer_session(&headers)?;
    store.activate_sampling_policy(session, body).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn activate_privacy(
    State(store): State<Store>,
    headers: HeaderMap,
    Json(body): Json<ActivatePrivacyRequest>,
) -> Result<StatusCode, ApiError> {
    let session = bearer_session(&headers)?;
    store.activate_privacy_policy(session, body).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_schema(
    State(store): State<Store>,
    headers: HeaderMap,
    Json(body): Json<CreateSchemaRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let session = bearer_session(&headers)?;
    Ok((
        StatusCode::CREATED,
        Json(store.create_behavior_schema(session, body).await?),
    ))
}

fn bearer_session(headers: &HeaderMap) -> Result<&str, ApiError> {
    let values: Vec<_> = headers.get_all(AUTHORIZATION).iter().collect();
    if values.len() != 1 {
        return Err(ApiError(ControlPlaneError::Unauthorized));
    }
    let value = values[0]
        .to_str()
        .map_err(|_| ApiError(ControlPlaneError::Unauthorized))?;
    let credential = value
        .strip_prefix("Bearer ")
        .ok_or(ApiError(ControlPlaneError::Unauthorized))?;
    if credential.is_empty() || credential.trim() != credential {
        return Err(ApiError(ControlPlaneError::Unauthorized));
    }
    Ok(credential)
}

struct ApiError(ControlPlaneError);

impl From<ControlPlaneError> for ApiError {
    fn from(error: ControlPlaneError) -> Self {
        Self(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self.0 {
            ControlPlaneError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            ControlPlaneError::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            ControlPlaneError::Conflict => (StatusCode::CONFLICT, "conflict"),
            ControlPlaneError::InvalidInput(_) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "request failed")
            }
            ControlPlaneError::Database(_) => (StatusCode::INTERNAL_SERVER_ERROR, "request failed"),
            ControlPlaneError::Credential(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "request failed")
            }
        };
        (status, message).into_response()
    }
}

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_parser_requires_one_unpadded_value() {
        for values in [
            vec![],
            vec!["Basic nope"],
            vec!["Bearer "],
            vec!["Bearer one", "Bearer two"],
            vec!["Bearer padded "],
        ] {
            let mut headers = HeaderMap::new();
            for value in values {
                headers.append(
                    AUTHORIZATION,
                    HeaderValue::from_str(value).unwrap_or_else(|_| unreachable!()),
                );
            }
            assert!(bearer_session(&headers).is_err());
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer ch_us_secret"),
        );
        assert_eq!(bearer_session(&headers).ok(), Some("ch_us_secret"));
    }
}
