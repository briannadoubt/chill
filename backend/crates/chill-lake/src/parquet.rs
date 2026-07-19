use std::sync::Arc;

use arrow::{
    array::{Array as _, ArrayRef, BooleanArray, Int32Array, Int64Array, StringArray, UInt64Array},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use parquet::{
    arrow::{ArrowWriter, arrow_reader::ParquetRecordBatchReaderBuilder},
    basic::{Compression, ZstdLevel},
    file::properties::WriterProperties,
};

use crate::{Batch, DATASET_SCHEMA_VERSION, LakeError, Row, WRITER_FORMAT_VERSION, model::invalid};

/// Encodes a stable, source-ID-sorted Zstandard Parquet object.
///
/// # Errors
///
/// Returns an error for invalid batch/row identity, count mismatch, Arrow shape,
/// ordinal overflow, or Parquet serialization.
pub fn encode_parquet(batch: &Batch, rows: &[Row]) -> Result<Vec<u8>, LakeError> {
    batch.validate()?;
    if rows.len() != batch.row_count {
        return Err(invalid(format!(
            "row count {} does not match batch row count {}",
            rows.len(),
            batch.row_count
        )));
    }
    let mut rows = rows.to_vec();
    rows.sort_by_key(|row| row.canonical_envelope_id);
    for (index, row) in rows.iter_mut().enumerate() {
        DATASET_SCHEMA_VERSION.clone_into(&mut row.dataset_schema_version);
        row.batch_id.clone_from(&batch.id);
        row.row_ordinal =
            i32::try_from(index).map_err(|_| invalid("row ordinal exceeds signed 32-bit range"))?;
        if row.canonical_envelope_id < 1
            || row.organization_id != batch.organization_id
            || row.project_id != batch.project_id
            || row.environment_id != batch.environment_id
            || row.envelope_kind != batch.envelope_kind
        {
            return Err(invalid("row does not match batch identity and partition"));
        }
    }
    let schema = lake_schema();
    let record_batch = RecordBatch::try_new(schema.clone(), columns(&rows))?;
    let properties = WriterProperties::builder()
        .set_created_by(format!(
            "chill version {DATASET_SCHEMA_VERSION} ({WRITER_FORMAT_VERSION})"
        ))
        .set_compression(Compression::ZSTD(ZstdLevel::default()))
        .set_dictionary_enabled(true)
        .build();
    let mut writer = ArrowWriter::try_new(Vec::new(), schema, Some(properties))?;
    writer.write(&record_batch)?;
    Ok(writer.into_inner()?)
}

/// Decodes the exact stable lake schema into public rows.
///
/// # Errors
///
/// Returns an error when Parquet cannot be read, the schema differs from the
/// released 30-column contract, a column has the wrong Arrow type, or a
/// required value is null.
pub fn decode_parquet(body: bytes::Bytes) -> Result<Vec<Row>, LakeError> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(body)?;
    if builder.schema().as_ref() != lake_schema().as_ref() {
        return Err(invalid("Parquet lake schema does not match version 1"));
    }
    let reader = builder.build()?;
    let mut result = Vec::new();
    for batch in reader {
        let batch = batch?;
        for index in 0..batch.num_rows() {
            result.push(decode_row(&batch, index)?);
        }
    }
    Ok(result)
}

#[allow(
    clippy::too_many_lines,
    reason = "field order is the released 30-column Parquet contract"
)]
fn decode_row(batch: &RecordBatch, row: usize) -> Result<Row, LakeError> {
    Ok(Row {
        dataset_schema_version: required_string(batch, 0, row)?,
        batch_id: required_string(batch, 1, row)?,
        row_ordinal: required_i32(batch, 2, row)?,
        canonical_envelope_id: required_i64(batch, 3, row)?,
        organization_id: required_string(batch, 4, row)?,
        project_id: required_string(batch, 5, row)?,
        environment_id: required_string(batch, 6, row)?,
        data_source_id: required_string(batch, 7, row)?,
        envelope_version: required_string(batch, 8, row)?,
        envelope_kind: required_string(batch, 9, row)?,
        record_id: required_string(batch, 10, row)?,
        record_sha256: required_string(batch, 11, row)?,
        installation_id: optional_string(batch, 12, row)?,
        session_id: optional_string(batch, 13, row)?,
        replay_id: optional_string(batch, 14, row)?,
        replay_chunk_id: optional_string(batch, 15, row)?,
        trace_id: optional_string(batch, 16, row)?,
        span_id: optional_string(batch, 17, row)?,
        occurred_at_unix_nano: optional_u64_value(batch, 18, row)?,
        source_observed_at_unix_nano: optional_u64_value(batch, 19, row)?,
        server_received_at_unix_nano: required_u64(batch, 20, row)?,
        effective_occurred_at_unix_nano: required_u64(batch, 21, row)?,
        monotonic_nano: optional_u64_value(batch, 22, row)?,
        boot_id: optional_string(batch, 23, row)?,
        sequence_number: optional_u64_value(batch, 24, row)?,
        clock_skew_nano: required_i64(batch, 25, row)?,
        timing_class: required_string(batch, 26, row)?,
        late_arrival: required_bool(batch, 27, row)?,
        canonical_json: required_string(batch, 28, row)?,
        normalized_at_unix_nano: required_i64(batch, 29, row)?,
    })
}

fn strings_at(batch: &RecordBatch, column: usize) -> Result<&StringArray, LakeError> {
    batch
        .column(column)
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| invalid("Parquet string column has wrong Arrow type"))
}

fn required_string(batch: &RecordBatch, column: usize, row: usize) -> Result<String, LakeError> {
    let values = strings_at(batch, column)?;
    if values.is_null(row) {
        return Err(invalid("required Parquet string is null"));
    }
    Ok(values.value(row).to_owned())
}

fn optional_string(
    batch: &RecordBatch,
    column: usize,
    row: usize,
) -> Result<Option<String>, LakeError> {
    let values = strings_at(batch, column)?;
    Ok((!values.is_null(row)).then(|| values.value(row).to_owned()))
}

macro_rules! primitive_accessors {
    ($required:ident, $optional:ident, $array:ty, $value:ty, $message:literal) => {
        fn $required(batch: &RecordBatch, column: usize, row: usize) -> Result<$value, LakeError> {
            let values = batch
                .column(column)
                .as_any()
                .downcast_ref::<$array>()
                .ok_or_else(|| invalid($message))?;
            if values.is_null(row) {
                return Err(invalid("required Parquet primitive is null"));
            }
            Ok(values.value(row))
        }

        #[allow(dead_code, reason = "macro keeps primitive null handling uniform")]
        fn $optional(
            batch: &RecordBatch,
            column: usize,
            row: usize,
        ) -> Result<Option<$value>, LakeError> {
            let values = batch
                .column(column)
                .as_any()
                .downcast_ref::<$array>()
                .ok_or_else(|| invalid($message))?;
            Ok((!values.is_null(row)).then(|| values.value(row)))
        }
    };
}

primitive_accessors!(
    required_i32,
    optional_i32,
    Int32Array,
    i32,
    "wrong i32 Arrow type"
);
primitive_accessors!(
    required_i64,
    optional_i64,
    Int64Array,
    i64,
    "wrong i64 Arrow type"
);
primitive_accessors!(
    required_u64,
    optional_u64_value,
    UInt64Array,
    u64,
    "wrong u64 Arrow type"
);
primitive_accessors!(
    required_bool,
    optional_bool,
    BooleanArray,
    bool,
    "wrong bool Arrow type"
);

fn lake_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        string_field("dataset_schema_version", false),
        string_field("batch_id", false),
        Field::new("row_ordinal", DataType::Int32, false),
        Field::new("canonical_envelope_id", DataType::Int64, false),
        string_field("organization_id", false),
        string_field("project_id", false),
        string_field("environment_id", false),
        string_field("data_source_id", false),
        string_field("envelope_version", false),
        string_field("envelope_kind", false),
        string_field("record_id", false),
        string_field("record_sha256", false),
        string_field("installation_id", true),
        string_field("session_id", true),
        string_field("replay_id", true),
        string_field("replay_chunk_id", true),
        string_field("trace_id", true),
        string_field("span_id", true),
        Field::new("occurred_at_unix_nano", DataType::UInt64, true),
        Field::new("source_observed_at_unix_nano", DataType::UInt64, true),
        Field::new("server_received_at_unix_nano", DataType::UInt64, false),
        Field::new("effective_occurred_at_unix_nano", DataType::UInt64, false),
        Field::new("monotonic_nano", DataType::UInt64, true),
        string_field("boot_id", true),
        Field::new("sequence_number", DataType::UInt64, true),
        Field::new("clock_skew_nano", DataType::Int64, false),
        string_field("timing_class", false),
        Field::new("late_arrival", DataType::Boolean, false),
        string_field("canonical_json", false),
        Field::new("normalized_at_unix_nano", DataType::Int64, false),
    ]))
}

fn string_field(name: &str, nullable: bool) -> Field {
    Field::new(name, DataType::Utf8, nullable)
}

#[allow(
    clippy::too_many_lines,
    reason = "column order is the public Parquet schema"
)]
fn columns(rows: &[Row]) -> Vec<ArrayRef> {
    vec![
        strings(rows.iter().map(|row| row.dataset_schema_version.as_str())),
        strings(rows.iter().map(|row| row.batch_id.as_str())),
        Arc::new(Int32Array::from_iter_values(
            rows.iter().map(|row| row.row_ordinal),
        )),
        Arc::new(Int64Array::from_iter_values(
            rows.iter().map(|row| row.canonical_envelope_id),
        )),
        strings(rows.iter().map(|row| row.organization_id.as_str())),
        strings(rows.iter().map(|row| row.project_id.as_str())),
        strings(rows.iter().map(|row| row.environment_id.as_str())),
        strings(rows.iter().map(|row| row.data_source_id.as_str())),
        strings(rows.iter().map(|row| row.envelope_version.as_str())),
        strings(rows.iter().map(|row| row.envelope_kind.as_str())),
        strings(rows.iter().map(|row| row.record_id.as_str())),
        strings(rows.iter().map(|row| row.record_sha256.as_str())),
        optional_strings(rows.iter().map(|row| row.installation_id.as_deref())),
        optional_strings(rows.iter().map(|row| row.session_id.as_deref())),
        optional_strings(rows.iter().map(|row| row.replay_id.as_deref())),
        optional_strings(rows.iter().map(|row| row.replay_chunk_id.as_deref())),
        optional_strings(rows.iter().map(|row| row.trace_id.as_deref())),
        optional_strings(rows.iter().map(|row| row.span_id.as_deref())),
        optional_u64(rows.iter().map(|row| row.occurred_at_unix_nano)),
        optional_u64(rows.iter().map(|row| row.source_observed_at_unix_nano)),
        Arc::new(UInt64Array::from_iter_values(
            rows.iter().map(|row| row.server_received_at_unix_nano),
        )),
        Arc::new(UInt64Array::from_iter_values(
            rows.iter().map(|row| row.effective_occurred_at_unix_nano),
        )),
        optional_u64(rows.iter().map(|row| row.monotonic_nano)),
        optional_strings(rows.iter().map(|row| row.boot_id.as_deref())),
        optional_u64(rows.iter().map(|row| row.sequence_number)),
        Arc::new(Int64Array::from_iter_values(
            rows.iter().map(|row| row.clock_skew_nano),
        )),
        strings(rows.iter().map(|row| row.timing_class.as_str())),
        Arc::new(
            rows.iter()
                .map(|row| Some(row.late_arrival))
                .collect::<BooleanArray>(),
        ),
        strings(rows.iter().map(|row| row.canonical_json.as_str())),
        Arc::new(Int64Array::from_iter_values(
            rows.iter().map(|row| row.normalized_at_unix_nano),
        )),
    ]
}

fn strings<'a>(values: impl Iterator<Item = &'a str>) -> ArrayRef {
    Arc::new(StringArray::from_iter_values(values))
}

fn optional_strings<'a>(values: impl Iterator<Item = Option<&'a str>>) -> ArrayRef {
    Arc::new(values.collect::<StringArray>())
}

fn optional_u64(values: impl Iterator<Item = Option<u64>>) -> ArrayRef {
    Arc::new(values.collect::<UInt64Array>())
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use time::{Date, Month};

    use super::{decode_parquet, encode_parquet};
    use crate::{Batch, Row, deterministic_batch_id};

    #[test]
    fn writes_public_schema_in_source_identity_order() {
        let id = deterministic_batch_id("micro", "partition", &[vec![1; 32]], &[]);
        let batch = Batch {
            id,
            organization_id: "10000000-0000-4000-8000-000000000001".to_owned(),
            project_id: "20000000-0000-4000-8000-000000000001".to_owned(),
            environment_id: "30000000-0000-4000-8000-000000000001".to_owned(),
            kind: "micro".to_owned(),
            partition_day: Date::from_calendar_date(2026, Month::July, 16).unwrap_or(Date::MIN),
            partition_hour: 12,
            envelope_kind: "otel.log".to_owned(),
            row_count: 1,
            min_server_received_at_unix_nano: 10,
            max_server_received_at_unix_nano: 10,
            min_effective_occurred_at_unix_nano: 9,
            max_effective_occurred_at_unix_nano: 9,
            attempt_count: 0,
            supersedes: Vec::new(),
        };
        let row = Row {
            dataset_schema_version: String::new(),
            batch_id: String::new(),
            row_ordinal: -1,
            canonical_envelope_id: 1,
            organization_id: batch.organization_id.clone(),
            project_id: batch.project_id.clone(),
            environment_id: batch.environment_id.clone(),
            data_source_id: "40000000-0000-4000-8000-000000000001".to_owned(),
            envelope_version: "1.0.0".to_owned(),
            envelope_kind: batch.envelope_kind.clone(),
            record_id: "record".to_owned(),
            record_sha256: "00".repeat(32),
            installation_id: None,
            session_id: None,
            replay_id: None,
            replay_chunk_id: None,
            trace_id: None,
            span_id: None,
            occurred_at_unix_nano: Some(9),
            source_observed_at_unix_nano: None,
            server_received_at_unix_nano: 10,
            effective_occurred_at_unix_nano: 9,
            monotonic_nano: None,
            boot_id: None,
            sequence_number: None,
            clock_skew_nano: 1,
            timing_class: "on_time".to_owned(),
            late_arrival: false,
            canonical_json: "{}".to_owned(),
            normalized_at_unix_nano: 11,
        };
        let body = encode_parquet(&batch, &[row]).unwrap_or_default();
        assert!(body.starts_with(b"PAR1"));
        let decoded = decode_parquet(Bytes::from(body.clone())).unwrap_or_default();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].canonical_envelope_id, 1);
        let reader = ParquetRecordBatchReaderBuilder::try_new(Bytes::from(body))
            .and_then(ParquetRecordBatchReaderBuilder::build);
        let mut reader = reader
            .unwrap_or_else(|error| unreachable!("encoded Parquet should be readable: {error}"));
        let record_batch = reader
            .next()
            .transpose()
            .unwrap_or_else(|error| unreachable!("read record batch: {error}"))
            .unwrap_or_else(|| unreachable!("one record batch should exist"));
        assert_eq!(record_batch.num_rows(), 1);
        assert_eq!(record_batch.num_columns(), 30);
    }
}
