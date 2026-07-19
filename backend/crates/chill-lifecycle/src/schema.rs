use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use sqlx::{PgPool, types::Json};
use thiserror::Error;

/// Reader/writer compatibility contract for a dataset schema proposal.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Compatibility {
    /// New readers can read old data.
    Backward,
    /// Old readers can read new data.
    Forward,
    /// Both backward and forward compatible.
    Full,
}

impl Compatibility {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Backward => "backward",
            Self::Forward => "forward",
            Self::Full => "full",
        }
    }
}

/// One stable public dataset column.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SchemaField {
    /// Column name.
    pub name: String,
    /// Stable logical type independent of Parquet physical encoding.
    pub logical_type: String,
    /// Whether every writer and reader must provide the field.
    pub required: bool,
}

/// Versioned public lake dataset schema.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DatasetSchema {
    /// Monotonically increasing schema version.
    pub version: i32,
    /// Uniquely named columns.
    pub fields: Vec<SchemaField>,
}

impl DatasetSchema {
    /// Returns fields in canonical name order for stable hashing.
    #[must_use]
    pub fn canonicalized(mut self) -> Self {
        self.fields
            .sort_by(|left, right| left.name.cmp(&right.name));
        self
    }
}

/// Dataset schema validation or registry failure.
#[derive(Debug, Error)]
pub enum SchemaError {
    /// Evolution would break the declared compatibility contract.
    #[error("dataset schema evolution is invalid: {0}")]
    Invalid(String),
    /// Database operation failed.
    #[error("dataset schema registry operation failed: {0}")]
    Database(#[from] sqlx::Error),
    /// Canonical JSON encoding failed.
    #[error("encode dataset schema: {0}")]
    Json(#[from] serde_json::Error),
    /// Compiled and durable active versions disagree.
    #[error("compiled lake schema is version {compiled} but registry active version is {active}")]
    ActiveVersionMismatch {
        /// Compiled schema version.
        compiled: i32,
        /// Active durable schema version.
        active: i32,
    },
    /// A non-draft version cannot be edited.
    #[error("dataset schema version is no longer an editable draft")]
    ImmutableDraft,
}

/// Validates an exact one-version evolution under the selected contract.
///
/// # Errors
///
/// Rejects malformed/duplicate fields, skipped versions, type changes, and
/// incompatible requiredness changes.
pub fn validate_evolution(
    previous: &DatasetSchema,
    next: &DatasetSchema,
    mode: Compatibility,
) -> Result<(), SchemaError> {
    if previous.version < 1 || next.version != previous.version + 1 {
        return Err(invalid("versions must advance by exactly one"));
    }
    let old = field_map(&previous.fields, "previous")?;
    let new = field_map(&next.fields, "next")?;
    for (name, old_field) in &old {
        if let Some(new_field) = new.get(name)
            && new_field.logical_type != old_field.logical_type
        {
            return Err(invalid(format!("field {name:?} changes logical type")));
        }
        if matches!(mode, Compatibility::Forward | Compatibility::Full)
            && old_field.required
            && new.get(name).is_none_or(|field| !field.required)
        {
            return Err(invalid(format!(
                "field {name:?} is required by forward readers"
            )));
        }
    }
    if matches!(mode, Compatibility::Backward | Compatibility::Full) {
        for (name, new_field) in &new {
            if new_field.required && old.get(name).is_none_or(|field| !field.required) {
                return Err(invalid(format!(
                    "field {name:?} cannot become required for backward readers"
                )));
            }
        }
    }
    Ok(())
}

fn field_map<'a>(
    fields: &'a [SchemaField],
    label: &str,
) -> Result<BTreeMap<&'a str, &'a SchemaField>, SchemaError> {
    if fields.is_empty() {
        return Err(invalid(format!(
            "{label} schema requires at least one field"
        )));
    }
    let mut result = BTreeMap::new();
    for field in fields {
        if field.name.is_empty() || field.logical_type.is_empty() {
            return Err(invalid(format!(
                "{label} field name and logical type are required"
            )));
        }
        if result.insert(field.name.as_str(), field).is_some() {
            return Err(invalid(format!(
                "{label} field {:?} is duplicated",
                field.name
            )));
        }
    }
    Ok(result)
}

/// Returns the exact schema emitted by `chill-lake` version 1.
#[must_use]
pub fn current_dataset_schema() -> DatasetSchema {
    let required = |name: &str, logical_type: &str| SchemaField {
        name: name.to_owned(),
        logical_type: logical_type.to_owned(),
        required: true,
    };
    let optional = |name: &str, logical_type: &str| SchemaField {
        name: name.to_owned(),
        logical_type: logical_type.to_owned(),
        required: false,
    };
    DatasetSchema {
        version: 1,
        fields: vec![
            required("dataset_schema_version", "string"),
            required("batch_id", "string"),
            required("row_ordinal", "int32"),
            required("canonical_envelope_id", "int64"),
            required("organization_id", "string"),
            required("project_id", "string"),
            required("environment_id", "string"),
            required("data_source_id", "string"),
            required("envelope_version", "string"),
            required("envelope_kind", "string"),
            required("record_id", "string"),
            required("record_sha256", "string"),
            optional("installation_id", "string"),
            optional("session_id", "string"),
            optional("replay_id", "string"),
            optional("replay_chunk_id", "string"),
            optional("trace_id", "string"),
            optional("span_id", "string"),
            optional("occurred_at_unix_nano", "uint64"),
            optional("source_observed_at_unix_nano", "uint64"),
            required("server_received_at_unix_nano", "uint64"),
            required("effective_occurred_at_unix_nano", "uint64"),
            optional("monotonic_nano", "uint64"),
            optional("boot_id", "string"),
            optional("sequence_number", "uint64"),
            required("clock_skew_nano", "int64"),
            required("timing_class", "string"),
            required("late_arrival", "bool"),
            required("canonical_json", "json"),
            required("normalized_at_unix_nano", "int64"),
        ],
    }
    .canonicalized()
}

/// Serialized registry writer guarded by a `PostgreSQL` advisory transaction lock.
#[derive(Clone)]
pub struct SchemaRegistry {
    pool: PgPool,
}

impl SchemaRegistry {
    /// Creates an administrative schema registry boundary.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Validates and inserts or replaces an editable draft schema proposal.
    ///
    /// # Errors
    ///
    /// Returns an evolution, active-version, immutable-draft, encoding, or database error.
    pub async fn propose(
        &self,
        next: DatasetSchema,
        mode: Compatibility,
    ) -> Result<[u8; 32], SchemaError> {
        let current = current_dataset_schema();
        validate_evolution(&current, &next, mode)?;
        let next = next.canonicalized();
        let body = serde_json::to_vec(&next)?;
        let digest: [u8; 32] = Sha256::digest(&body).into();
        let definition: Value = serde_json::from_slice(&body)?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('chill-dataset-schema-registry'))")
            .execute(&mut *transaction)
            .await?;
        let active: i32 = sqlx::query_scalar(
            "SELECT version FROM lake.dataset_schemas WHERE status='active' ORDER BY version DESC LIMIT 1")
            .fetch_one(&mut *transaction).await?;
        if active != current.version {
            return Err(SchemaError::ActiveVersionMismatch {
                compiled: current.version,
                active,
            });
        }
        let changed = sqlx::query(
            r"INSERT INTO lake.dataset_schemas
            (version,definition,digest,compatibility,status) VALUES ($1,$2,$3,$4,'draft')
            ON CONFLICT (version) DO UPDATE SET definition=EXCLUDED.definition,
              digest=EXCLUDED.digest,compatibility=EXCLUDED.compatibility,status='draft'
            WHERE lake.dataset_schemas.status='draft'",
        )
        .bind(next.version)
        .bind(Json(definition))
        .bind(digest.as_slice())
        .bind(mode.as_str())
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed != 1 {
            return Err(SchemaError::ImmutableDraft);
        }
        transaction.commit().await?;
        Ok(digest)
    }
}

fn invalid(message: impl Into<String>) -> SchemaError {
    SchemaError::Invalid(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, logical_type: &str, required: bool) -> SchemaField {
        SchemaField {
            name: name.to_owned(),
            logical_type: logical_type.to_owned(),
            required,
        }
    }

    #[test]
    fn compatibility_fails_closed() {
        let previous = DatasetSchema {
            version: 1,
            fields: vec![field("id", "string", true)],
        };
        let additive = DatasetSchema {
            version: 2,
            fields: vec![field("id", "string", true), field("page", "string", false)],
        };
        assert!(validate_evolution(&previous, &additive, Compatibility::Full).is_ok());
        let mut breaking = additive.clone();
        breaking.fields[1].required = true;
        assert!(validate_evolution(&previous, &breaking, Compatibility::Backward).is_err());
        let mut type_change = additive;
        type_change.fields[0].logical_type = "int64".to_owned();
        assert!(validate_evolution(&previous, &type_change, Compatibility::Full).is_err());
        let optional_previous = DatasetSchema {
            version: 1,
            fields: vec![field("page", "string", false)],
        };
        let required_next = DatasetSchema {
            version: 2,
            fields: vec![field("page", "string", true)],
        };
        assert!(
            validate_evolution(&optional_previous, &required_next, Compatibility::Backward)
                .is_err()
        );
        let optional_next = DatasetSchema {
            version: 2,
            fields: vec![field("id", "string", false)],
        };
        assert!(validate_evolution(&previous, &optional_next, Compatibility::Forward).is_err());
    }

    #[test]
    fn current_schema_matches_lake_version() {
        let current = current_dataset_schema();
        assert_eq!(
            current.version.to_string(),
            chill_lake::DATASET_SCHEMA_VERSION
        );
        assert_eq!(current.fields.len(), 30);
    }
}
