use crate::{
    Batch, DATASET_SCHEMA_VERSION, LakeError, Manifest, WRITER_FORMAT_VERSION, model::invalid,
};

/// Encodes the newline-terminated immutable manifest JSON.
///
/// # Errors
///
/// Returns an error for invalid batch/object identity or JSON serialization.
pub fn encode_manifest(
    batch: &Batch,
    object_key: &str,
    object_digest: [u8; 32],
    byte_count: i64,
) -> Result<Vec<u8>, LakeError> {
    batch.validate()?;
    if object_key.is_empty() || byte_count < 1 {
        return Err(invalid("manifest object identity is invalid"));
    }
    let mut supersedes = batch.supersedes.clone();
    supersedes.sort();
    supersedes.dedup();
    let manifest = Manifest {
        format: "chill.lake-manifest/v1".to_owned(),
        dataset_schema_version: DATASET_SCHEMA_VERSION.to_owned(),
        writer_format_version: WRITER_FORMAT_VERSION.to_owned(),
        batch_id: batch.id.clone(),
        batch_kind: batch.kind.clone(),
        organization_id: batch.organization_id.clone(),
        project_id: batch.project_id.clone(),
        environment_id: batch.environment_id.clone(),
        partition_day: batch.partition_day.to_string(),
        partition_hour: batch.partition_hour,
        envelope_kind: batch.envelope_kind.clone(),
        object_key: object_key.to_owned(),
        object_sha256: hex::encode(object_digest),
        byte_count,
        row_count: batch.row_count,
        min_server_received_at_unix_nano: batch.min_server_received_at_unix_nano,
        max_server_received_at_unix_nano: batch.max_server_received_at_unix_nano,
        min_effective_occurred_at_unix_nano: batch.min_effective_occurred_at_unix_nano,
        max_effective_occurred_at_unix_nano: batch.max_effective_occurred_at_unix_nano,
        supersedes,
    };
    let mut body = serde_json::to_vec(&manifest)?;
    body.push(b'\n');
    Ok(body)
}
