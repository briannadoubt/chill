use chill_control_plane::{
    annotation_classification_is_eligible, parse_environment_privacy_policy,
};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use sqlx::{Postgres, Transaction, types::Json};
use time::OffsetDateTime;

use crate::{InboxItem, ProcessorError, Record};

const BEHAVIOR_SCHEMA: &str = "https://schemas.chill.dev/behavior/v1/envelope.schema.json";

pub(crate) async fn validate_and_filter(
    transaction: &mut Transaction<'_, Postgres>,
    item: &InboxItem,
    records: &[Record],
) -> Result<Vec<Record>, ProcessorError> {
    let (retention_days, replay_retention_days, scope_deletion) =
        sqlx::query_as::<_, (i32, i32, bool)>(
            r"
            SELECT environment.retention_days,environment.replay_retention_days,
                EXISTS (
                    SELECT 1 FROM lifecycle.deletion_requests AS request
                    WHERE request.organization_id=$1::uuid
                      AND request.status IN ('pending','leased')
                      AND (request.kind='tenant' OR (request.kind='environment'
                        AND request.project_id=$2::uuid AND request.environment_id=$3::uuid))
                )
            FROM control.environments AS environment
            WHERE environment.organization_id=$1::uuid AND environment.project_id=$2::uuid
              AND environment.id=$3::uuid
            ",
        )
        .bind(&item.organization_id)
        .bind(&item.project_id)
        .bind(&item.environment_id)
        .fetch_one(&mut **transaction)
        .await?;
    if scope_deletion {
        return Ok(Vec::new());
    }
    let now = OffsetDateTime::now_utc().unix_timestamp_nanos();
    let retention_cutoff = cutoff(now, retention_days);
    let replay_cutoff = cutoff(now, replay_retention_days);
    let mut kept = Vec::with_capacity(records.len());
    for record in records {
        let cutoff = if record.envelope_kind == "replay" {
            replay_cutoff
        } else {
            retention_cutoff
        };
        if record.timing.effective_occurred < cutoff
            || tombstoned(transaction, item, record).await?
        {
            continue;
        }
        if record.envelope_kind == "behavior" {
            validate_behavior(transaction, item, record).await?;
        }
        kept.push(record.clone());
    }
    Ok(kept)
}

async fn validate_behavior(
    transaction: &mut Transaction<'_, Postgres>,
    item: &InboxItem,
    record: &Record,
) -> Result<(), ProcessorError> {
    let definition = sqlx::query_scalar::<_, Json<Value>>(
        r"
        SELECT definition FROM control.behavior_schemas
        WHERE project_id=$1::uuid AND version='1.0.0' AND schema_url=$2 AND status='active'
        ",
    )
    .bind(&item.project_id)
    .bind(BEHAVIOR_SCHEMA)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| integrity("active canonical behavior schema is unavailable"))?;
    let validator = jsonschema::validator_for(&definition.0)
        .map_err(|error| integrity(format!("compile behavior schema: {error}")))?;
    if let Err(error) = validator.validate(&record.canonical) {
        let path = error.instance_path().to_string();
        return Err(integrity(format!(
            "canonical behavior schema validation at {path}: {error}"
        )));
    }
    validate_invariants(&record.canonical)?;
    let policy_document = sqlx::query_scalar::<_, Json<Value>>(
        r"
        SELECT document FROM control.privacy_policies
        WHERE organization_id=$1::uuid AND project_id=$2::uuid
          AND environment_id=$3::uuid AND status='active'
        ",
    )
    .bind(&item.organization_id)
    .bind(&item.project_id)
    .bind(&item.environment_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| integrity("active environment privacy policy is unavailable"))?;
    let policy = parse_environment_privacy_policy(policy_document.0)
        .map_err(|error| integrity(error.to_string()))?;
    let privacy = object(&record.canonical, "privacy")?;
    if privacy.get("policy_version").and_then(Value::as_str) != Some(&policy.policy_version) {
        return Err(integrity(
            "behavior privacy policy version is stale or unknown",
        ));
    }
    let annotations = object(&record.canonical, "annotations")?;
    let classifications = privacy
        .get("annotation_classifications")
        .and_then(Value::as_object)
        .ok_or_else(|| integrity("behavior annotation classifications are missing"))?;
    if annotations.len() != classifications.len() {
        return Err(integrity(
            "every annotation requires exactly one classification",
        ));
    }
    for name in annotations.keys() {
        let classification = classifications
            .get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| integrity("annotation classification is invalid"))?;
        if policy.annotation_allowlist.get(name).map(String::as_str) != Some(classification)
            || !annotation_classification_is_eligible(classification)
        {
            return Err(integrity(format!(
                "annotation {name:?} is not allowed by the active privacy policy"
            )));
        }
    }
    Ok(())
}

fn validate_invariants(document: &Value) -> Result<(), ProcessorError> {
    let kind = text(document, "kind")?;
    let operation = text(document, "operation")?;
    if matches!(kind, "action" | "impression" | "event")
        && text(document, "record_id")? != text(document, "subject_id")?
    {
        return Err(integrity("instant subject must equal record ID"));
    }
    let clock = object(document, "clock")?;
    let occurred = decimal(clock.get("occurred_at_unix_nano"))?;
    let observed = decimal(clock.get("observed_at_unix_nano"))?;
    if observed < occurred {
        return Err(integrity("canonical observed time precedes occurrence"));
    }
    if document.get("duration_nano").is_some() && operation != "end" {
        return Err(integrity("duration is valid only for end records"));
    }
    Ok(())
}

async fn tombstoned(
    transaction: &mut Transaction<'_, Postgres>,
    item: &InboxItem,
    record: &Record,
) -> Result<bool, ProcessorError> {
    for (kind, value) in [
        ("installation_id", record.installation_id.as_deref()),
        ("session_id", record.session_id.as_deref()),
        ("replay_id", record.replay_id.as_deref()),
    ] {
        let Some(value) = value else { continue };
        let digest = Sha256::digest(value.as_bytes());
        let exists = sqlx::query_scalar::<_, bool>(
            r"
            SELECT EXISTS (SELECT 1 FROM lifecycle.subject_tombstones
                WHERE environment_id=$1::uuid AND target_kind=$2 AND target_sha256=$3)
            ",
        )
        .bind(&item.environment_id)
        .bind(kind)
        .bind(digest.as_slice())
        .fetch_one(&mut **transaction)
        .await?;
        if exists {
            return Ok(true);
        }
    }
    Ok(false)
}

fn cutoff(now: i128, days: i32) -> u64 {
    let nanos_per_day = 86_400_i128 * 1_000_000_000;
    let value = now.saturating_sub(i128::from(days).saturating_mul(nanos_per_day));
    u64::try_from(value.clamp(0, i128::from(u64::MAX))).unwrap_or_default()
}

fn object<'a>(
    document: &'a Value,
    name: &str,
) -> Result<&'a serde_json::Map<String, Value>, ProcessorError> {
    document
        .get(name)
        .and_then(Value::as_object)
        .ok_or_else(|| integrity(format!("canonical {name} is missing")))
}

fn text<'a>(document: &'a Value, name: &str) -> Result<&'a str, ProcessorError> {
    document
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| integrity(format!("canonical {name} is missing")))
}

fn decimal(value: Option<&Value>) -> Result<u64, ProcessorError> {
    value
        .and_then(Value::as_str)
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| integrity("canonical timestamp is invalid"))
}

fn integrity(message: impl Into<String>) -> ProcessorError {
    ProcessorError::Integrity(message.into())
}
