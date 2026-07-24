use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use duckdb::{Connection, InterruptHandle, params_from_iter, types::Value};
use serde_json::{Number, Value as JsonValue};
use thiserror::Error;

use crate::{Argument, Compiled, Result, Stats};

/// Sandboxed `DuckDB` resource configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineConfiguration {
    /// Resident memory bound in MiB.
    pub memory_limit_mib: usize,
    /// Execution threads.
    pub threads: usize,
    /// Spill-file bound in MiB.
    pub maximum_temporary_mib: usize,
}

impl Default for EngineConfiguration {
    fn default() -> Self {
        Self {
            memory_limit_mib: 768,
            threads: 2,
            maximum_temporary_mib: 512,
        }
    }
}

/// Query-engine construction or execution failure.
#[derive(Debug, Error)]
pub enum EngineError {
    /// Engine limits or cache root are invalid.
    #[error("DuckDB query engine configuration is invalid")]
    InvalidConfiguration,
    /// Cache filesystem operation failed.
    #[error("DuckDB cache filesystem operation failed: {0}")]
    Filesystem(#[from] std::io::Error),
    /// `DuckDB` rejected configuration or execution.
    #[error("DuckDB operation failed: {0}")]
    DuckDb(#[from] duckdb::Error),
    /// Another thread poisoned the single connection.
    #[error("DuckDB connection lock is unavailable")]
    Lock,
    /// Returned columns did not match the typed plan.
    #[error("DuckDB result shape does not match the typed plan")]
    Shape,
    /// Row or serialized-byte bound was exceeded.
    #[error("query result exceeded its resource limit")]
    ResultLimit,
    /// `DuckDB` returned a value outside the public JSON scalar contract.
    #[error("DuckDB returned an unsupported result value")]
    UnsupportedValue,
}

/// A single-connection, locked-down embedded analytical engine.
pub struct Engine {
    connection: Mutex<Connection>,
    interrupt: Arc<InterruptHandle>,
    cache_root: PathBuf,
}

impl Engine {
    /// Creates a sandboxed in-memory engine restricted to one cache root.
    ///
    /// # Errors
    ///
    /// Returns an error if limits are invalid, directories cannot be prepared,
    /// or any security/resource setting cannot be applied and locked.
    pub fn new(
        cache_root: impl AsRef<Path>,
        configuration: EngineConfiguration,
    ) -> std::result::Result<Self, EngineError> {
        if !(128..=3_072).contains(&configuration.memory_limit_mib)
            || !(1..=4).contains(&configuration.threads)
            || !(64..=4_096).contains(&configuration.maximum_temporary_mib)
        {
            return Err(EngineError::InvalidConfiguration);
        }
        fs::create_dir_all(cache_root.as_ref())?;
        let cache_root = cache_root.as_ref().canonicalize()?;
        let temporary_root = cache_root.join("temp");
        fs::create_dir_all(&temporary_root)?;
        let connection = Connection::open_in_memory()?;
        let statements = [
            format!("SET memory_limit='{}MB'", configuration.memory_limit_mib),
            format!("SET threads={}", configuration.threads),
            format!(
                "SET temp_directory='{}'",
                quote_literal(&temporary_root.to_string_lossy())
            ),
            format!(
                "SET max_temp_directory_size='{}MB'",
                configuration.maximum_temporary_mib
            ),
            format!(
                "SET allowed_directories=['{}']",
                quote_literal(&cache_root.to_string_lossy())
            ),
            "SET enable_external_access=false".to_owned(),
            "SET autoinstall_known_extensions=false".to_owned(),
            "SET autoload_known_extensions=false".to_owned(),
            "SET allow_community_extensions=false".to_owned(),
            "SET enable_logging=false".to_owned(),
            "SET lock_configuration=true".to_owned(),
        ];
        for statement in statements {
            connection.execute_batch(&statement)?;
        }
        let interrupt = connection.interrupt_handle();
        Ok(Self {
            connection: Mutex::new(connection),
            interrupt,
            cache_root,
        })
    }

    /// Returns the canonical directory allowed for Parquet reads.
    #[must_use]
    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    /// Interrupts any currently executing statement on this engine.
    pub fn interrupt(&self) {
        self.interrupt.interrupt();
    }

    /// Executes parameterized SQL with a second outer row cap and a serialized
    /// JSON byte cap.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid limits, engine failure, shape mismatch,
    /// unsupported result types, or a breached row/byte cap.
    pub fn execute(
        &self,
        compiled: &Compiled,
        maximum_rows: usize,
        maximum_bytes: usize,
    ) -> std::result::Result<Result, EngineError> {
        if maximum_rows == 0 || maximum_bytes < 1_024 {
            return Err(EngineError::InvalidConfiguration);
        }
        if compiled.sql.is_empty() {
            return Ok(Result {
                columns: compiled.columns.iter().map(ToString::to_string).collect(),
                rows: Vec::new(),
                stats: Stats::default(),
            });
        }
        let sql = format!(
            "SELECT * FROM ({}) AS chill_bounded_result LIMIT ?",
            compiled.sql
        );
        let mut parameters: Vec<Value> = compiled
            .arguments
            .iter()
            .map(|argument| match argument {
                Argument::Text(value) => Value::Text(value.clone()),
                Argument::Integer(value) => Value::BigInt(*value),
            })
            .collect();
        parameters.push(Value::BigInt(
            i64::try_from(maximum_rows.saturating_add(1))
                .map_err(|_| EngineError::InvalidConfiguration)?,
        ));
        let connection = self.connection.lock().map_err(|_| EngineError::Lock)?;
        let mut statement = connection.prepare(&sql)?;
        let mut cursor = statement.query(params_from_iter(parameters))?;
        let metadata = cursor.as_ref().ok_or(EngineError::Shape)?;
        let columns = metadata.column_names();
        if columns
            != compiled
                .columns
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        {
            return Err(EngineError::Shape);
        }
        let column_count = columns.len();
        let mut rows = Vec::new();
        let mut serialized_bytes = 0_usize;
        while let Some(row) = cursor.next()? {
            if rows.len() >= maximum_rows {
                return Err(EngineError::ResultLimit);
            }
            let mut values = Vec::with_capacity(column_count);
            for index in 0..column_count {
                values.push(to_json(row.get::<usize, Value>(index)?)?);
            }
            serialized_bytes = serialized_bytes
                .checked_add(serde_json::to_vec(&values)?.len())
                .ok_or(EngineError::ResultLimit)?;
            if serialized_bytes > maximum_bytes {
                return Err(EngineError::ResultLimit);
            }
            rows.push(values);
        }
        Ok(Result {
            columns,
            stats: Stats {
                row_count: rows.len(),
                ..Stats::default()
            },
            rows,
        })
    }

    #[cfg(test)]
    fn execute_batch(&self, sql: &str) -> std::result::Result<(), EngineError> {
        self.connection
            .lock()
            .map_err(|_| EngineError::Lock)?
            .execute_batch(sql)?;
        Ok(())
    }
}

fn quote_literal(value: &str) -> String {
    value.replace('\'', "''")
}

#[allow(
    clippy::match_same_arms,
    clippy::too_many_lines,
    reason = "document current unsupported composites while future DuckDB variants fail closed"
)]
fn to_json(value: Value) -> std::result::Result<JsonValue, EngineError> {
    Ok(match value {
        Value::Null => JsonValue::Null,
        Value::Boolean(value) => JsonValue::Bool(value),
        Value::TinyInt(value) => JsonValue::Number(Number::from(value)),
        Value::SmallInt(value) => JsonValue::Number(Number::from(value)),
        Value::Int(value) | Value::Date32(value) => JsonValue::Number(Number::from(value)),
        Value::BigInt(value) => JsonValue::Number(Number::from(value)),
        Value::HugeInt(value) => JsonValue::String(value.to_string()),
        Value::UTinyInt(value) => JsonValue::Number(Number::from(value)),
        Value::USmallInt(value) => JsonValue::Number(Number::from(value)),
        Value::UInt(value) => JsonValue::Number(Number::from(value)),
        Value::UBigInt(value) => JsonValue::Number(Number::from(value)),
        Value::Float(value) => Number::from_f64(f64::from(value))
            .map(JsonValue::Number)
            .ok_or(EngineError::UnsupportedValue)?,
        Value::Double(value) => Number::from_f64(value)
            .map(JsonValue::Number)
            .ok_or(EngineError::UnsupportedValue)?,
        Value::Decimal(value) => JsonValue::String(value.to_string()),
        Value::Timestamp(_, value) | Value::Time64(_, value) => {
            JsonValue::Number(Number::from(value))
        }
        Value::Text(value) | Value::Enum(value) => JsonValue::String(value),
        Value::Blob(value) => JsonValue::String(String::from_utf8_lossy(&value).into_owned()),
        Value::Interval { .. }
        | Value::List(_)
        | Value::Struct(_)
        | Value::Array(_)
        | Value::Map(_)
        | Value::Union(_) => return Err(EngineError::UnsupportedValue),
        _ => return Err(EngineError::UnsupportedValue),
    })
}

impl From<serde_json::Error> for EngineError {
    fn from(_: serde_json::Error) -> Self {
        Self::UnsupportedValue
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, env, fs, path::Path, time::Duration};

    use chill_lake::{Batch, Row, deterministic_batch_id, encode_parquet};
    use sha2::{Digest as _, Sha256};
    use time::{Date, Month};

    use super::*;
    use crate::{
        AggregatePlan, BehaviorFilter, CohortPlan, EventsPlan, FunnelPlan, FunnelStep, Kind,
        PLAN_VERSION, PathPlan, Plan, ReplayPlan, RetentionPlan, TimeRange, TracePlan, compile,
    };

    #[test]
    fn executes_parameterized_parquet_and_enforces_caps() -> anyhow::Result<()> {
        let root = env::temp_dir().join(format!("chill-query-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root)?;
        let rows = fixture_rows();
        let path = write_fixture(&root, "behavior", &rows)?;
        let engine = Engine::new(&root, EngineConfiguration::default())?;
        assert!(engine.execute_batch("SET threads=4").is_err());
        assert!(
            engine
                .execute_batch("SELECT * FROM read_text('/etc/passwd')")
                .is_err()
        );
        let plan = Plan {
            version: PLAN_VERSION,
            kind: Kind::Events,
            range: TimeRange {
                start_unix_nano: 1,
                end_unix_nano: 1_000,
            },
            events: Some(EventsPlan {
                filter: BehaviorFilter::default(),
                session_id: String::new(),
                trace_id: String::new(),
                installation_id: String::new(),
                before: None,
                limit: 10,
            }),
            trace: None,
            replay: None,
            funnel: None,
            cohort: None,
            aggregate: None,
            path: None,
            retention: None,
        };
        plan.validate(Duration::from_hours(1), 100)?;
        let compiled = compile(&plan, std::slice::from_ref(&path))?;
        assert_eq!(engine.execute(&compiled, 10, 1 << 20)?.rows.len(), 2);
        assert!(matches!(
            engine.execute(&compiled, 1, 1 << 20),
            Err(EngineError::ResultLimit)
        ));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one reference fixture verifies every public typed query plan"
    )]
    fn executes_every_typed_plan() -> anyhow::Result<()> {
        let root = env::temp_dir().join(format!("chill-query-plans-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root)?;
        let behavior_path = write_fixture(&root, "behavior", &fixture_rows())?;
        let mut replay_row = fixture_rows().remove(0);
        replay_row.envelope_kind = "replay".to_owned();
        replay_row.replay_id = Some("replay-1".to_owned());
        replay_row.replay_chunk_id = Some("chunk-1".to_owned());
        let replay_path = write_fixture(&root, "replay", &[replay_row])?;
        let engine = Engine::new(&root, EngineConfiguration::default())?;
        let range = TimeRange {
            start_unix_nano: 1,
            end_unix_nano: 1_000,
        };
        let plans = [
            (
                typed_plan(
                    Kind::Events,
                    range,
                    Payload::Events(EventsPlan {
                        filter: BehaviorFilter {
                            annotations: BTreeMap::from([("tier".to_owned(), "pro".to_owned())]),
                            ..BehaviorFilter::default()
                        },
                        session_id: String::new(),
                        trace_id: String::new(),
                        installation_id: String::new(),
                        before: None,
                        limit: 10,
                    }),
                ),
                behavior_path.clone(),
                2,
            ),
            (
                typed_plan(
                    Kind::Path,
                    range,
                    Payload::Path(PathPlan {
                        anchor_name: "checkout".to_owned(),
                        direction: "after".to_owned(),
                        depth: 1,
                        filter: BehaviorFilter::default(),
                        limit: 10,
                    }),
                ),
                behavior_path.clone(),
                1,
            ),
            (
                typed_plan(
                    Kind::Retention,
                    range,
                    Payload::Retention(RetentionPlan {
                        start_filter: BehaviorFilter {
                            name: "checkout".to_owned(),
                            ..BehaviorFilter::default()
                        },
                        return_filter: BehaviorFilter {
                            name: "purchase".to_owned(),
                            ..BehaviorFilter::default()
                        },
                        interval: "day".to_owned(),
                        periods: 2,
                        limit: 10,
                    }),
                ),
                behavior_path.clone(),
                1,
            ),
            (
                typed_plan(
                    Kind::Trace,
                    range,
                    Payload::Trace(TracePlan {
                        trace_id: "11111111111111111111111111111111".to_owned(),
                        limit: 10,
                    }),
                ),
                behavior_path.clone(),
                2,
            ),
            (
                typed_plan(
                    Kind::Replay,
                    range,
                    Payload::Replay(ReplayPlan {
                        replay_id: "replay-1".to_owned(),
                        session_id: String::new(),
                        limit: 10,
                    }),
                ),
                replay_path,
                1,
            ),
            (
                typed_plan(
                    Kind::Funnel,
                    range,
                    Payload::Funnel(FunnelPlan {
                        mode: "ordered".to_owned(),
                        exclusions: Vec::new(),
                        breakdown: "none".to_owned(),
                        steps: vec![
                            FunnelStep {
                                label: "Checkout".to_owned(),
                                kind: String::new(),
                                operation: String::new(),
                                name: "checkout".to_owned(),
                                annotations: BTreeMap::new(),
                            },
                            FunnelStep {
                                label: "Purchase".to_owned(),
                                kind: String::new(),
                                operation: String::new(),
                                name: "purchase".to_owned(),
                                annotations: BTreeMap::new(),
                            },
                        ],
                        completion_window_nano: 500,
                    }),
                ),
                behavior_path.clone(),
                2,
            ),
            (
                typed_plan(
                    Kind::Cohort,
                    range,
                    Payload::Cohort(CohortPlan {
                        filter: BehaviorFilter {
                            name: "checkout".to_owned(),
                            ..BehaviorFilter::default()
                        },
                        minimum_count: 1,
                        sequence: Vec::new(),
                        sequence_window_nano: 0,
                        limit: 10,
                    }),
                ),
                behavior_path.clone(),
                1,
            ),
            (
                typed_plan(
                    Kind::Cohort,
                    range,
                    Payload::Cohort(CohortPlan {
                        filter: BehaviorFilter::default(),
                        minimum_count: 1,
                        sequence: vec![
                            FunnelStep {
                                label: "Checkout".to_owned(),
                                kind: String::new(),
                                operation: String::new(),
                                name: "checkout".to_owned(),
                                annotations: BTreeMap::new(),
                            },
                            FunnelStep {
                                label: "Purchase".to_owned(),
                                kind: String::new(),
                                operation: String::new(),
                                name: "purchase".to_owned(),
                                annotations: BTreeMap::new(),
                            },
                        ],
                        sequence_window_nano: 500,
                        limit: 10,
                    }),
                ),
                behavior_path.clone(),
                1,
            ),
            (
                typed_plan(
                    Kind::Aggregate,
                    range,
                    Payload::Aggregate(AggregatePlan {
                        metric: "count".to_owned(),
                        dimension: "name".to_owned(),
                        interval: "hour".to_owned(),
                        filter: BehaviorFilter::default(),
                        limit: 10,
                    }),
                ),
                behavior_path,
                2,
            ),
        ];
        for (plan, path, expected_rows) in plans {
            plan.validate(Duration::from_hours(1), 100)?;
            let result = engine.execute(&compile(&plan, &[path])?, 100, 1 << 20)?;
            assert_eq!(result.rows.len(), expected_rows, "{:?}", plan.kind);
        }
        fs::remove_dir_all(root)?;
        Ok(())
    }

    enum Payload {
        Events(EventsPlan),
        Trace(TracePlan),
        Replay(ReplayPlan),
        Funnel(FunnelPlan),
        Cohort(CohortPlan),
        Aggregate(AggregatePlan),
        Path(PathPlan),
        Retention(RetentionPlan),
    }

    fn typed_plan(kind: Kind, range: TimeRange, payload: Payload) -> Plan {
        let mut plan = Plan {
            version: PLAN_VERSION,
            kind,
            range,
            events: None,
            trace: None,
            replay: None,
            funnel: None,
            cohort: None,
            aggregate: None,
            path: None,
            retention: None,
        };
        match payload {
            Payload::Events(value) => plan.events = Some(value),
            Payload::Trace(value) => plan.trace = Some(value),
            Payload::Replay(value) => plan.replay = Some(value),
            Payload::Funnel(value) => plan.funnel = Some(value),
            Payload::Cohort(value) => plan.cohort = Some(value),
            Payload::Aggregate(value) => plan.aggregate = Some(value),
            Payload::Path(value) => plan.path = Some(value),
            Payload::Retention(value) => plan.retention = Some(value),
        }
        plan
    }

    fn write_fixture(root: &Path, kind: &str, rows: &[Row]) -> anyhow::Result<String> {
        let digests: Vec<Vec<u8>> = rows
            .iter()
            .map(|row| Sha256::digest(row.canonical_json.as_bytes()).to_vec())
            .collect();
        let batch = Batch {
            id: deterministic_batch_id("micro", kind, &digests, &[]),
            organization_id: "11111111-1111-4111-8111-111111111111".to_owned(),
            project_id: "22222222-2222-4222-8222-222222222222".to_owned(),
            environment_id: "33333333-3333-4333-8333-333333333333".to_owned(),
            kind: "micro".to_owned(),
            partition_day: Date::from_calendar_date(1970, Month::January, 1)?,
            partition_hour: 0,
            envelope_kind: kind.to_owned(),
            row_count: rows.len(),
            min_server_received_at_unix_nano: 1,
            max_server_received_at_unix_nano: 1_000,
            min_effective_occurred_at_unix_nano: 1,
            max_effective_occurred_at_unix_nano: 1_000,
            attempt_count: 1,
            supersedes: Vec::new(),
        };
        let path = root.join(format!("{kind}.parquet"));
        fs::write(&path, encode_parquet(&batch, rows)?)?;
        Ok(path.to_string_lossy().into_owned())
    }

    fn fixture_rows() -> Vec<Row> {
        ["checkout", "purchase"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| Row {
                dataset_schema_version: String::new(),
                batch_id: String::new(),
                row_ordinal: 0,
                canonical_envelope_id: i64::try_from(index + 1).unwrap_or_default(),
                organization_id: "11111111-1111-4111-8111-111111111111".to_owned(),
                project_id: "22222222-2222-4222-8222-222222222222".to_owned(),
                environment_id: "33333333-3333-4333-8333-333333333333".to_owned(),
                data_source_id: "44444444-4444-4444-8444-444444444444".to_owned(),
                envelope_version: "1.0.0".to_owned(),
                envelope_kind: "behavior".to_owned(),
                record_id: format!("record-{index}"),
                record_sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                    .to_owned(),
                installation_id: None,
                session_id: Some("session-1".to_owned()),
                replay_id: None,
                replay_chunk_id: None,
                trace_id: Some("11111111111111111111111111111111".to_owned()),
                span_id: None,
                occurred_at_unix_nano: None,
                source_observed_at_unix_nano: None,
                server_received_at_unix_nano: u64::try_from(index + 100).unwrap_or_default(),
                effective_occurred_at_unix_nano: u64::try_from(index + 100).unwrap_or_default(),
                monotonic_nano: None,
                boot_id: None,
                sequence_number: None,
                clock_skew_nano: 0,
                timing_class: "on_time".to_owned(),
                late_arrival: false,
                canonical_json: format!(
                    r#"{{"subject_id":"user-1","kind":"action","operation":"tap","name":"{name}","annotations":{{"tier":"pro"}}}}"#
                ),
                normalized_at_unix_nano: i64::try_from(index + 100).unwrap_or_default(),
            })
            .collect()
    }
}
