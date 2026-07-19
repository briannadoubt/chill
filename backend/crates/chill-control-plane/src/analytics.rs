#![allow(
    missing_docs,
    reason = "wire structs use self-describing stable field names"
)]
#![allow(
    clippy::missing_errors_doc,
    clippy::items_after_statements,
    reason = "authenticated store methods share the crate error contract and local row aliases stay near their queries"
)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::types::Json;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::{Capability, ControlPlaneError, Store};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalyticsScope {
    pub project_id: String,
    pub environment_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSavedQueryRequest {
    pub project_id: String,
    pub environment_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub plan: Value,
    pub visualization: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateDashboardRequest {
    pub project_id: String,
    pub environment_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub sharing: String,
    pub layout: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAlertRequest {
    pub project_id: String,
    pub environment_id: String,
    pub saved_query_id: String,
    pub name: String,
    pub operator: String,
    pub threshold: f64,
    pub schedule_minutes: i32,
}

#[derive(Debug, Serialize)]
pub struct SavedQuery {
    pub id: String,
    pub name: String,
    pub description: String,
    pub plan: Value,
    pub visualization: String,
    pub updated_at: String,
}
#[derive(Debug, Serialize)]
pub struct Dashboard {
    pub id: String,
    pub name: String,
    pub description: String,
    pub sharing: String,
    pub layout: Value,
    pub updated_at: String,
}
#[derive(Debug, Serialize)]
pub struct Alert {
    pub id: String,
    pub saved_query_id: String,
    pub name: String,
    pub operator: String,
    pub threshold: f64,
    pub schedule_minutes: i32,
    pub status: String,
    pub next_evaluation_at: String,
    pub last_evaluated_at: Option<String>,
    pub last_value: Option<f64>,
    pub last_state: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct AnalyticsWorkspace {
    pub saved_queries: Vec<SavedQuery>,
    pub dashboards: Vec<Dashboard>,
    pub alerts: Vec<Alert>,
}

#[derive(Debug, Serialize)]
pub struct DebuggerSnapshot {
    pub requests: Vec<IngestDiagnostic>,
    pub sources: Vec<SdkDiagnostic>,
}
#[derive(Debug, Serialize)]
pub struct IngestDiagnostic {
    pub id: i64,
    pub request_id: String,
    pub data_source_id: String,
    pub signal_kind: String,
    pub payload_format: String,
    pub record_count: i32,
    pub status: String,
    pub attempt_count: i32,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub metadata: Value,
    pub received_at: String,
}
#[derive(Debug, Serialize)]
pub struct SdkDiagnostic {
    pub data_source_id: String,
    pub source_name: String,
    pub source_kind: String,
    pub source_status: String,
    pub active_keys: i64,
    pub last_used_at: Option<String>,
}

impl Store {
    pub async fn analytics_workspace(
        &self,
        raw: &str,
        scope: AnalyticsScope,
    ) -> Result<AnalyticsWorkspace, ControlPlaneError> {
        let session = self
            .require_user_capability(raw, Capability::DataRead)
            .await?;
        validate_scope(&scope)?;
        let mut tx = self.begin_tenant(&session.organization_id).await?;
        ensure_scope(&mut tx, &scope).await?;
        let saved = sqlx::query_as::<_, (String,String,String,Json<Value>,String,OffsetDateTime)>("SELECT id::text,name,description,plan,visualization,updated_at FROM product.saved_queries WHERE project_id=$1::uuid AND environment_id=$2::uuid AND status='active' ORDER BY lower(name),id")
            .bind(&scope.project_id).bind(&scope.environment_id).fetch_all(&mut *tx).await?.into_iter().map(|row| SavedQuery { id: row.0, name: row.1, description: row.2, plan: row.3.0, visualization: row.4, updated_at: timestamp(row.5) }).collect();
        let dashboards = sqlx::query_as::<_, (String,String,String,String,Json<Value>,OffsetDateTime)>("SELECT id::text,name,description,sharing,layout,updated_at FROM product.dashboards WHERE project_id=$1::uuid AND environment_id=$2::uuid AND status='active' ORDER BY lower(name),id")
            .bind(&scope.project_id).bind(&scope.environment_id).fetch_all(&mut *tx).await?.into_iter().map(|row| Dashboard { id: row.0, name: row.1, description: row.2, sharing: row.3, layout: row.4.0, updated_at: timestamp(row.5) }).collect();
        let alerts = sqlx::query_as::<_, (String,String,String,String,f64,i32,String,OffsetDateTime,Option<OffsetDateTime>,Option<f64>,Option<String>)>("SELECT id::text,saved_query_id::text,name,operator,threshold,schedule_minutes,status,next_evaluation_at,last_evaluated_at,last_value,last_state FROM product.alerts WHERE project_id=$1::uuid AND environment_id=$2::uuid AND status<>'archived' ORDER BY lower(name),id")
            .bind(&scope.project_id).bind(&scope.environment_id).fetch_all(&mut *tx).await?.into_iter().map(|row| Alert { id: row.0, saved_query_id: row.1, name: row.2, operator: row.3, threshold: row.4, schedule_minutes: row.5, status: row.6, next_evaluation_at: timestamp(row.7), last_evaluated_at: row.8.map(timestamp), last_value: row.9, last_state: row.10 }).collect();
        tx.commit().await?;
        Ok(AnalyticsWorkspace {
            saved_queries: saved,
            dashboards,
            alerts,
        })
    }

    pub async fn create_saved_query(
        &self,
        raw: &str,
        body: CreateSavedQueryRequest,
    ) -> Result<SavedQuery, ControlPlaneError> {
        validate_create_saved(&body)?;
        let session = self
            .require_user_capability(raw, Capability::ControlWrite)
            .await?;
        let scope = AnalyticsScope {
            project_id: body.project_id,
            environment_id: body.environment_id,
        };
        let mut tx = self.begin_tenant(&session.organization_id).await?;
        ensure_scope(&mut tx, &scope).await?;
        let row = sqlx::query_as::<_, (String,OffsetDateTime)>("INSERT INTO product.saved_queries (organization_id,project_id,environment_id,owner_user_id,name,description,plan,visualization) VALUES ($1::uuid,$2::uuid,$3::uuid,$4::uuid,$5,$6,$7,$8) RETURNING id::text,updated_at")
            .bind(&session.organization_id).bind(&scope.project_id).bind(&scope.environment_id).bind(&session.user_id).bind(&body.name).bind(&body.description).bind(Json(body.plan.clone())).bind(&body.visualization).fetch_one(&mut *tx).await.map_err(map_conflict)?;
        tx.commit().await?;
        Ok(SavedQuery {
            id: row.0,
            name: body.name,
            description: body.description,
            plan: body.plan,
            visualization: body.visualization,
            updated_at: timestamp(row.1),
        })
    }

    pub async fn create_dashboard(
        &self,
        raw: &str,
        body: CreateDashboardRequest,
    ) -> Result<Dashboard, ControlPlaneError> {
        validate_text(&body.name, 160, "dashboard name")?;
        validate_text_optional(&body.description, 2000, "dashboard description")?;
        if !["private", "organization"].contains(&body.sharing.as_str())
            || !body.layout.is_array()
            || serde_json::to_vec(&body.layout)
                .map_err(|_| invalid("dashboard layout"))?
                .len()
                > 65_536
        {
            return Err(invalid("dashboard"));
        }
        let session = self
            .require_user_capability(raw, Capability::ControlWrite)
            .await?;
        let scope = AnalyticsScope {
            project_id: body.project_id,
            environment_id: body.environment_id,
        };
        validate_scope(&scope)?;
        let mut tx = self.begin_tenant(&session.organization_id).await?;
        ensure_scope(&mut tx, &scope).await?;
        let row=sqlx::query_as::<_,(String,OffsetDateTime)>("INSERT INTO product.dashboards (organization_id,project_id,environment_id,owner_user_id,name,description,sharing,layout) VALUES ($1::uuid,$2::uuid,$3::uuid,$4::uuid,$5,$6,$7,$8) RETURNING id::text,updated_at").bind(&session.organization_id).bind(&scope.project_id).bind(&scope.environment_id).bind(&session.user_id).bind(&body.name).bind(&body.description).bind(&body.sharing).bind(Json(body.layout.clone())).fetch_one(&mut *tx).await.map_err(map_conflict)?;
        tx.commit().await?;
        Ok(Dashboard {
            id: row.0,
            name: body.name,
            description: body.description,
            sharing: body.sharing,
            layout: body.layout,
            updated_at: timestamp(row.1),
        })
    }

    pub async fn create_alert(
        &self,
        raw: &str,
        body: CreateAlertRequest,
    ) -> Result<Alert, ControlPlaneError> {
        validate_text(&body.name, 160, "alert name")?;
        if !["gt", "gte", "lt", "lte", "eq"].contains(&body.operator.as_str())
            || !body.threshold.is_finite()
            || !(5..=10080).contains(&body.schedule_minutes)
        {
            return Err(invalid("alert"));
        }
        let session = self
            .require_user_capability(raw, Capability::ControlWrite)
            .await?;
        let scope = AnalyticsScope {
            project_id: body.project_id,
            environment_id: body.environment_id,
        };
        validate_scope(&scope)?;
        validate_uuid(&body.saved_query_id)?;
        let mut tx = self.begin_tenant(&session.organization_id).await?;
        ensure_scope(&mut tx, &scope).await?;
        let row=sqlx::query_as::<_,(String,OffsetDateTime)>("INSERT INTO product.alerts (organization_id,project_id,environment_id,saved_query_id,owner_user_id,name,operator,threshold,schedule_minutes) SELECT $1::uuid,$2::uuid,$3::uuid,id,$4::uuid,$5,$6,$7,$8 FROM product.saved_queries WHERE id=$9::uuid AND project_id=$2::uuid AND environment_id=$3::uuid AND status='active' RETURNING id::text,next_evaluation_at").bind(&session.organization_id).bind(&scope.project_id).bind(&scope.environment_id).bind(&session.user_id).bind(&body.name).bind(&body.operator).bind(body.threshold).bind(body.schedule_minutes).bind(&body.saved_query_id).fetch_optional(&mut *tx).await.map_err(map_conflict)?.ok_or(ControlPlaneError::Forbidden)?;
        tx.commit().await?;
        Ok(Alert {
            id: row.0,
            saved_query_id: body.saved_query_id,
            name: body.name,
            operator: body.operator,
            threshold: body.threshold,
            schedule_minutes: body.schedule_minutes,
            status: "active".into(),
            next_evaluation_at: timestamp(row.1),
            last_evaluated_at: None,
            last_value: None,
            last_state: None,
        })
    }

    pub async fn archive_analytics_resource(
        &self,
        raw: &str,
        kind: &str,
        id: &str,
    ) -> Result<(), ControlPlaneError> {
        validate_uuid(id)?;
        let session = self
            .require_user_capability(raw, Capability::ControlWrite)
            .await?;
        let sql = match kind {
            "saved-query" => {
                "UPDATE product.saved_queries SET status='archived',updated_at=clock_timestamp() WHERE id=$1::uuid AND status<>'archived'"
            }
            "dashboard" => {
                "UPDATE product.dashboards SET status='archived',updated_at=clock_timestamp() WHERE id=$1::uuid AND status<>'archived'"
            }
            "alert" => {
                "UPDATE product.alerts SET status='archived',updated_at=clock_timestamp() WHERE id=$1::uuid AND status<>'archived'"
            }
            _ => return Err(invalid("analytics resource")),
        };
        let mut tx = self.begin_tenant(&session.organization_id).await?;
        let changed = sqlx::query(sql).bind(id).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(ControlPlaneError::Forbidden);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn debugger_snapshot(
        &self,
        raw: &str,
        scope: AnalyticsScope,
    ) -> Result<DebuggerSnapshot, ControlPlaneError> {
        let session = self
            .require_user_capability(raw, Capability::DataRead)
            .await?;
        validate_scope(&scope)?;
        let mut tx = self.begin_tenant(&session.organization_id).await?;
        ensure_scope(&mut tx, &scope).await?;
        type RequestRow = (
            i64,
            String,
            String,
            String,
            String,
            i32,
            String,
            i32,
            Option<String>,
            Option<String>,
            Json<Value>,
            OffsetDateTime,
        );
        let requests=sqlx::query_as::<_,RequestRow>("SELECT id,request_id,data_source_id::text,signal_kind,payload_format,record_count,status,attempt_count,last_error_code,last_error_message,metadata,server_received_at FROM ingest.inbox WHERE project_id=$1::uuid AND environment_id=$2::uuid ORDER BY server_received_at DESC,id DESC LIMIT 100").bind(&scope.project_id).bind(&scope.environment_id).fetch_all(&mut *tx).await?.into_iter().map(|r|IngestDiagnostic{id:r.0,request_id:r.1,data_source_id:r.2,signal_kind:r.3,payload_format:r.4,record_count:r.5,status:r.6,attempt_count:r.7,error_code:r.8,error_message:r.9,metadata:r.10.0,received_at:timestamp(r.11)}).collect();
        let sources=sqlx::query_as::<_,(String,String,String,String,i64,Option<OffsetDateTime>)>("SELECT source.id::text,source.name,source.kind,source.status,count(key.id),max(key.last_used_at) FROM control.data_sources source LEFT JOIN control.sdk_keys key ON key.data_source_id=source.id AND key.status='active' WHERE source.project_id=$1::uuid AND source.environment_id=$2::uuid GROUP BY source.id,source.name,source.kind,source.status ORDER BY lower(source.name)").bind(&scope.project_id).bind(&scope.environment_id).fetch_all(&mut *tx).await?.into_iter().map(|r|SdkDiagnostic{data_source_id:r.0,source_name:r.1,source_kind:r.2,source_status:r.3,active_keys:r.4,last_used_at:r.5.map(timestamp)}).collect();
        tx.commit().await?;
        Ok(DebuggerSnapshot { requests, sources })
    }
}

async fn ensure_scope(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    scope: &AnalyticsScope,
) -> Result<(), ControlPlaneError> {
    let exists=sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM control.environments WHERE id=$1::uuid AND project_id=$2::uuid AND status='active')").bind(&scope.environment_id).bind(&scope.project_id).fetch_one(&mut **tx).await?;
    if exists {
        Ok(())
    } else {
        Err(ControlPlaneError::Forbidden)
    }
}
fn validate_scope(scope: &AnalyticsScope) -> Result<(), ControlPlaneError> {
    validate_uuid(&scope.project_id)?;
    validate_uuid(&scope.environment_id)
}
fn validate_uuid(value: &str) -> Result<(), ControlPlaneError> {
    let parsed = Uuid::parse_str(value).map_err(|_| invalid("identifier"))?;
    if parsed.to_string() == value {
        Ok(())
    } else {
        Err(invalid("identifier"))
    }
}
fn validate_create_saved(body: &CreateSavedQueryRequest) -> Result<(), ControlPlaneError> {
    validate_scope(&AnalyticsScope {
        project_id: body.project_id.clone(),
        environment_id: body.environment_id.clone(),
    })?;
    validate_text(&body.name, 160, "query name")?;
    validate_text_optional(&body.description, 2000, "query description")?;
    if !body.plan.is_object()
        || serde_json::to_vec(&body.plan)
            .map_err(|_| invalid("query plan"))?
            .len()
            > 65_536
        || !["table", "line", "bar", "funnel", "retention", "path"]
            .contains(&body.visualization.as_str())
    {
        return Err(invalid("query"));
    }
    Ok(())
}
fn validate_text(value: &str, max: usize, label: &str) -> Result<(), ControlPlaneError> {
    if value.trim() != value || value.is_empty() || value.len() > max {
        Err(invalid(label))
    } else {
        Ok(())
    }
}
fn validate_text_optional(value: &str, max: usize, label: &str) -> Result<(), ControlPlaneError> {
    if value.trim() != value || value.len() > max {
        Err(invalid(label))
    } else {
        Ok(())
    }
}
fn invalid(label: &str) -> ControlPlaneError {
    ControlPlaneError::InvalidInput(format!("{label} is invalid"))
}
fn map_conflict(error: sqlx::Error) -> ControlPlaneError {
    if error
        .as_database_error()
        .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
    {
        ControlPlaneError::Conflict
    } else {
        ControlPlaneError::Database(error)
    }
}
fn timestamp(value: OffsetDateTime) -> String {
    value
        .format(&Rfc3339)
        .unwrap_or_else(|_| value.unix_timestamp().to_string())
}
