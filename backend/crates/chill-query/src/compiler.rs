use crate::{BehaviorFilter, FunnelStep, Kind, Plan, QueryError, TimeRange};

/// A strongly typed positional `DuckDB` argument.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Argument {
    /// UTF-8 text.
    Text(String),
    /// Signed 64-bit integer.
    Integer(i64),
}

/// Parameterized `DuckDB` SQL plus its stable public columns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Compiled {
    /// SQL containing only trusted compiler fragments and placeholders.
    pub sql: String,
    /// Positional parameter values.
    pub arguments: Vec<Argument>,
    /// Stable output column names.
    pub columns: Vec<&'static str>,
}

/// Compiles an already validated plan and local immutable Parquet paths.
///
/// Paths and every user-controlled value are positional arguments. Only
/// allowlisted plan enums can select SQL fragments.
///
/// # Errors
///
/// Returns an error if a required typed payload is absent or a numeric value
/// cannot be represented by `DuckDB`'s signed integer parameter.
pub fn compile(plan: &Plan, paths: &[String]) -> Result<Compiled, QueryError> {
    if paths.is_empty() {
        return Ok(Compiled {
            sql: String::new(),
            arguments: Vec::new(),
            columns: columns_for(plan.kind),
        });
    }
    let relation = format!(
        "read_parquet([{}], union_by_name=true)",
        vec!["?"; paths.len()].join(",")
    );
    let arguments = paths.iter().cloned().map(Argument::Text).collect();
    match plan.kind {
        Kind::Events => compile_events(plan, &relation, arguments),
        Kind::Trace => compile_trace(plan, &relation, arguments),
        Kind::Replay => compile_replay(plan, &relation, arguments),
        Kind::Funnel => compile_funnel(plan, &relation, arguments),
        Kind::Cohort => compile_cohort(plan, &relation, arguments),
        Kind::Aggregate => compile_aggregate(plan, &relation, arguments),
        Kind::Path => compile_path(plan, &relation, arguments),
        Kind::Retention => compile_retention(plan, &relation, arguments),
    }
}

fn compile_events(
    plan: &Plan,
    relation: &str,
    mut arguments: Vec<Argument>,
) -> Result<Compiled, QueryError> {
    let events = plan
        .events
        .as_ref()
        .ok_or(QueryError("events payload missing"))?;
    let mut predicates = behavior_where(plan.range, &events.filter, &mut arguments)?;
    for (column, value) in [
        ("session_id", &events.session_id),
        ("trace_id", &events.trace_id),
        ("installation_id", &events.installation_id),
    ] {
        if !value.is_empty() {
            predicates.push(format!("{column}=?"));
            arguments.push(Argument::Text(value.clone()));
        }
    }
    if let Some(cursor) = events.before {
        predicates.push("(effective_occurred_at_unix_nano < ? OR (effective_occurred_at_unix_nano = ? AND canonical_envelope_id < ?))".to_owned());
        let occurred = as_i64(cursor.effective_occurred_at_unix_nano)?;
        arguments.extend([
            Argument::Integer(occurred),
            Argument::Integer(occurred),
            Argument::Integer(cursor.canonical_envelope_id),
        ]);
    }
    arguments.push(Argument::Integer(as_limit(events.limit)?));
    Ok(Compiled {
        sql: format!(
            "SELECT canonical_envelope_id,record_id,subject_id,kind,operation,name,\
session_id,trace_id,span_id,effective_occurred_at_unix_nano,canonical_json \
FROM (SELECT *,json_extract_string(canonical_json,'$.subject_id') AS subject_id,\
json_extract_string(canonical_json,'$.kind') AS kind,\
json_extract_string(canonical_json,'$.operation') AS operation,\
json_extract_string(canonical_json,'$.name') AS name FROM {relation}) \
WHERE {} ORDER BY effective_occurred_at_unix_nano DESC,canonical_envelope_id DESC LIMIT ?",
            predicates.join(" AND ")
        ),
        arguments,
        columns: columns_for(Kind::Events),
    })
}

fn compile_trace(
    plan: &Plan,
    relation: &str,
    mut arguments: Vec<Argument>,
) -> Result<Compiled, QueryError> {
    let trace = plan
        .trace
        .as_ref()
        .ok_or(QueryError("trace payload missing"))?;
    arguments.extend([
        Argument::Integer(as_i64(plan.range.start_unix_nano)?),
        Argument::Integer(as_i64(plan.range.end_unix_nano)?),
        Argument::Text(trace.trace_id.clone()),
        Argument::Integer(as_limit(trace.limit)?),
    ]);
    Ok(Compiled {
        sql: format!(
            "SELECT canonical_envelope_id,envelope_kind,record_id,trace_id,span_id,\
effective_occurred_at_unix_nano,canonical_json FROM {relation} \
WHERE effective_occurred_at_unix_nano>=? AND effective_occurred_at_unix_nano<? AND trace_id=? \
ORDER BY effective_occurred_at_unix_nano,canonical_envelope_id LIMIT ?"
        ),
        arguments,
        columns: columns_for(Kind::Trace),
    })
}

fn compile_replay(
    plan: &Plan,
    relation: &str,
    mut arguments: Vec<Argument>,
) -> Result<Compiled, QueryError> {
    let replay = plan
        .replay
        .as_ref()
        .ok_or(QueryError("replay payload missing"))?;
    let (column, identifier) = if replay.replay_id.is_empty() {
        ("session_id", &replay.session_id)
    } else {
        ("replay_id", &replay.replay_id)
    };
    arguments.extend([
        Argument::Integer(as_i64(plan.range.start_unix_nano)?),
        Argument::Integer(as_i64(plan.range.end_unix_nano)?),
        Argument::Text(identifier.clone()),
        Argument::Integer(as_limit(replay.limit)?),
    ]);
    Ok(Compiled {
        sql: format!(
            "SELECT canonical_envelope_id,replay_id,replay_chunk_id,session_id,installation_id,\
effective_occurred_at_unix_nano,canonical_json FROM {relation} \
WHERE effective_occurred_at_unix_nano>=? AND effective_occurred_at_unix_nano<? AND {column}=? \
ORDER BY effective_occurred_at_unix_nano,canonical_envelope_id LIMIT ?"
        ),
        arguments,
        columns: columns_for(Kind::Replay),
    })
}

fn compile_funnel(
    plan: &Plan,
    relation: &str,
    mut arguments: Vec<Argument>,
) -> Result<Compiled, QueryError> {
    let funnel = plan
        .funnel
        .as_ref()
        .ok_or(QueryError("funnel payload missing"))?;
    arguments.extend([
        Argument::Integer(as_i64(plan.range.start_unix_nano)?),
        Argument::Integer(as_i64(plan.range.end_unix_nano)?),
    ]);
    let breakdown = match funnel.breakdown.as_str() {
        "none" => "'all'",
        "kind" => "json_extract_string(canonical_json,'$.kind')",
        "name" => "json_extract_string(canonical_json,'$.name')",
        "page" => "json_extract_string(canonical_json,'$.context.page.path')",
        "platform" => "json_extract_string(canonical_json,'$.source.platform')",
        _ => return Err(QueryError("funnel breakdown is invalid")),
    };
    let mut ctes = vec![format!(
        "raw AS MATERIALIZED (SELECT json_extract_string(canonical_json,'$.subject_id') AS subject_id,\
session_id,trace_id,json_extract_string(canonical_json,'$.kind') AS kind,\
json_extract_string(canonical_json,'$.operation') AS operation,json_extract_string(canonical_json,'$.name') AS name,\
coalesce({breakdown},'(unknown)') AS breakdown_value,effective_occurred_at_unix_nano AS occurred,canonical_json \
FROM {relation} WHERE envelope_kind='behavior' AND effective_occurred_at_unix_nano>=? \
AND effective_occurred_at_unix_nano<? AND json_extract_string(canonical_json,'$.subject_id') IS NOT NULL)"
    )];
    if funnel.exclusions.is_empty() {
        ctes.push("base AS MATERIALIZED (SELECT * FROM raw)".to_owned());
    } else {
        let mut exclusion_predicates = Vec::with_capacity(funnel.exclusions.len());
        for exclusion in &funnel.exclusions {
            exclusion_predicates.push(format!(
                "({})",
                step_where(exclusion, &mut arguments).join(" AND ")
            ));
        }
        ctes.push(format!(
            "excluded AS MATERIALIZED (SELECT DISTINCT subject_id FROM raw WHERE {}),\
base AS MATERIALIZED (SELECT * FROM raw WHERE subject_id NOT IN (SELECT subject_id FROM excluded))",
            exclusion_predicates.join(" OR ")
        ));
    }
    for (index, step) in funnel.steps.iter().enumerate() {
        let mut matches = step_where(step, &mut arguments);
        let ordinal = index + 1;
        if index == 0 {
            ctes.push(format!(
                "step_{ordinal} AS (SELECT subject_id,breakdown_value,min(occurred) AS started_at,\
min(occurred) AS reached_at,min(session_id) AS example_session_id,min(trace_id) AS example_trace_id \
FROM base WHERE {} GROUP BY subject_id,breakdown_value)",
                matches.join(" AND ")
            ));
        } else {
            // DuckDB consumes this placeholder before the step criteria.
            let criteria_count = matches.len();
            let split = arguments.len() - criteria_count;
            arguments.insert(
                split,
                Argument::Integer(as_i64(funnel.completion_window_nano)?),
            );
            for clause in &mut matches {
                *clause = if clause.starts_with("json_extract_string(") {
                    clause.replacen("canonical_json", "event.canonical_json", 1)
                } else {
                    format!("event.{clause}")
                };
            }
            ctes.push(format!(
                "step_{ordinal} AS (SELECT event.subject_id,prior.breakdown_value,prior.started_at,\
min(event.occurred) AS reached_at,prior.example_session_id,prior.example_trace_id \
FROM base AS event JOIN step_{index} AS prior USING(subject_id) WHERE {} AND {} \
GROUP BY event.subject_id,prior.breakdown_value,prior.started_at,prior.example_session_id,prior.example_trace_id)",
                if funnel.mode == "ordered" { "event.occurred>prior.reached_at AND event.occurred<=prior.started_at+?" } else { "abs(event.occurred-prior.started_at)<=?" },
                matches.join(" AND ")
            ));
        }
    }
    let mut outputs = Vec::with_capacity(funnel.steps.len());
    for (index, step) in funnel.steps.iter().enumerate() {
        let ordinal = index + 1;
        outputs.push(format!(
            "SELECT {ordinal} AS step_index,? AS step_label,breakdown_value,count(*) AS subjects,\
min(example_session_id) AS example_session_id,min(example_trace_id) AS example_trace_id \
FROM step_{ordinal} GROUP BY breakdown_value"
        ));
        arguments.push(Argument::Text(step.label.clone()));
    }
    Ok(Compiled {
        sql: format!(
            "WITH {} {} ORDER BY step_index",
            ctes.join(","),
            outputs.join(" UNION ALL ")
        ),
        arguments,
        columns: columns_for(Kind::Funnel),
    })
}

fn compile_cohort(
    plan: &Plan,
    relation: &str,
    mut arguments: Vec<Argument>,
) -> Result<Compiled, QueryError> {
    let cohort = plan
        .cohort
        .as_ref()
        .ok_or(QueryError("cohort payload missing"))?;
    if !cohort.sequence.is_empty() {
        arguments.extend([
            Argument::Integer(as_i64(plan.range.start_unix_nano)?),
            Argument::Integer(as_i64(plan.range.end_unix_nano)?),
        ]);
        let mut ctes = vec![format!(
            "raw AS MATERIALIZED (SELECT json_extract_string(canonical_json,'$.subject_id') AS subject_id,\
json_extract_string(canonical_json,'$.kind') AS kind,json_extract_string(canonical_json,'$.operation') AS operation,\
json_extract_string(canonical_json,'$.name') AS name,effective_occurred_at_unix_nano AS occurred,canonical_json \
FROM {relation} WHERE envelope_kind='behavior' AND effective_occurred_at_unix_nano>=? \
AND effective_occurred_at_unix_nano<? AND json_extract_string(canonical_json,'$.subject_id') IS NOT NULL)"
        )];
        for (index, step) in cohort.sequence.iter().enumerate() {
            let ordinal = index + 1;
            let mut matches = step_where(step, &mut arguments);
            if index == 0 {
                ctes.push(format!(
                    "step_{ordinal} AS (SELECT subject_id,min(occurred) AS started_at,min(occurred) AS reached_at \
FROM raw WHERE {} GROUP BY subject_id)",
                    matches.join(" AND ")
                ));
            } else {
                let criteria_count = matches.len();
                let split = arguments.len() - criteria_count;
                arguments.insert(
                    split,
                    Argument::Integer(as_i64(cohort.sequence_window_nano)?),
                );
                for clause in &mut matches {
                    *clause = if clause.starts_with("json_extract_string(") {
                        clause.replacen("canonical_json", "event.canonical_json", 1)
                    } else {
                        format!("event.{clause}")
                    };
                }
                ctes.push(format!(
                    "step_{ordinal} AS (SELECT event.subject_id,prior.started_at,min(event.occurred) AS reached_at \
FROM raw AS event JOIN step_{index} AS prior USING(subject_id) \
WHERE event.occurred>prior.reached_at AND event.occurred<=prior.started_at+? AND {} \
GROUP BY event.subject_id,prior.started_at)",
                    matches.join(" AND ")
                ));
            }
        }
        arguments.push(Argument::Integer(as_limit(cohort.limit)?));
        let final_step = cohort.sequence.len();
        return Ok(Compiled {
            sql: format!(
                "WITH {} SELECT subject_id,started_at AS first_seen,reached_at AS last_seen,\
{final_step} AS event_count FROM step_{final_step} ORDER BY last_seen DESC,subject_id LIMIT ?",
                ctes.join(",")
            ),
            arguments,
            columns: columns_for(Kind::Cohort),
        });
    }
    let predicates = behavior_where(plan.range, &cohort.filter, &mut arguments)?;
    arguments.extend([
        Argument::Integer(as_limit(cohort.minimum_count)?),
        Argument::Integer(as_limit(cohort.limit)?),
    ]);
    Ok(Compiled {
        sql: format!(
            "SELECT subject_id,min(effective_occurred_at_unix_nano) AS first_seen,\
max(effective_occurred_at_unix_nano) AS last_seen,count(*) AS event_count \
FROM (SELECT *,json_extract_string(canonical_json,'$.subject_id') AS subject_id,\
json_extract_string(canonical_json,'$.kind') AS kind,json_extract_string(canonical_json,'$.operation') AS operation,\
json_extract_string(canonical_json,'$.name') AS name FROM {relation}) WHERE {} AND subject_id IS NOT NULL \
GROUP BY subject_id HAVING count(*)>=? ORDER BY event_count DESC,subject_id LIMIT ?",
            predicates.join(" AND ")
        ),
        arguments,
        columns: columns_for(Kind::Cohort),
    })
}

fn compile_aggregate(
    plan: &Plan,
    relation: &str,
    mut arguments: Vec<Argument>,
) -> Result<Compiled, QueryError> {
    let aggregate = plan
        .aggregate
        .as_ref()
        .ok_or(QueryError("aggregate payload missing"))?;
    let predicates = behavior_where(plan.range, &aggregate.filter, &mut arguments)?;
    let dimension = match aggregate.dimension.as_str() {
        "none" => "'all'",
        "kind" => "kind",
        "operation" => "operation",
        "name" => "name",
        "page" => "json_extract_string(canonical_json,'$.context.page.path')",
        "platform" => "json_extract_string(canonical_json,'$.source.platform')",
        _ => return Err(QueryError("aggregate dimension is invalid")),
    };
    let metric = match aggregate.metric.as_str() {
        "count" => "count(*)",
        "unique_subjects" => "count(DISTINCT subject_id)",
        _ => return Err(QueryError("aggregate metric is invalid")),
    };
    let interval = match aggregate.interval.as_str() {
        "minute" => "minute",
        "hour" => "hour",
        "day" => "day",
        _ => return Err(QueryError("aggregate interval is invalid")),
    };
    arguments.push(Argument::Integer(as_limit(aggregate.limit)?));
    Ok(Compiled {
        sql: format!(
            "SELECT CAST(date_trunc('{interval}',make_timestamp_ns(CAST(effective_occurred_at_unix_nano AS BIGINT))) AS VARCHAR) AS bucket,\
coalesce({dimension},'(unknown)') AS dimension_value,{metric} AS metric_value \
FROM (SELECT *,json_extract_string(canonical_json,'$.subject_id') AS subject_id,\
json_extract_string(canonical_json,'$.kind') AS kind,json_extract_string(canonical_json,'$.operation') AS operation,\
json_extract_string(canonical_json,'$.name') AS name FROM {relation}) WHERE {} \
GROUP BY bucket,dimension_value ORDER BY bucket,dimension_value LIMIT ?",
            predicates.join(" AND ")
        ),
        arguments,
        columns: columns_for(Kind::Aggregate),
    })
}

fn compile_path(
    plan: &Plan,
    relation: &str,
    mut arguments: Vec<Argument>,
) -> Result<Compiled, QueryError> {
    let path = plan
        .path
        .as_ref()
        .ok_or(QueryError("path payload missing"))?;
    let predicates = behavior_where(plan.range, &path.filter, &mut arguments)?;
    let window = if path.direction == "after" {
        "lead"
    } else {
        "lag"
    };
    let mut neighbors = Vec::with_capacity(path.depth);
    for depth in 1..=path.depth {
        neighbors.push(format!(
            "{window}(name,{depth}) OVER session_order AS neighbor_{depth}"
        ));
    }
    let mut transitions = Vec::with_capacity(path.depth);
    for depth in 1..=path.depth {
        let (from, to) = if path.direction == "after" {
            ("name".to_owned(), format!("neighbor_{depth}"))
        } else {
            (format!("neighbor_{depth}"), "name".to_owned())
        };
        transitions.push(format!("SELECT ? AS direction,{depth} AS depth,{from} AS from_name,{to} AS to_name,count(DISTINCT subject_id) AS subjects,count(DISTINCT session_id) AS sessions,min(session_id) AS example_session_id,min(trace_id) AS example_trace_id FROM sequenced WHERE name=? AND neighbor_{depth} IS NOT NULL GROUP BY from_name,to_name"));
        arguments.push(Argument::Text(path.direction.clone()));
        arguments.push(Argument::Text(path.anchor_name.clone()));
    }
    arguments.push(Argument::Integer(as_limit(path.limit)?));
    Ok(Compiled {
        sql: format!(
            "WITH base AS MATERIALIZED (SELECT json_extract_string(canonical_json,'$.subject_id') AS subject_id,session_id,trace_id,json_extract_string(canonical_json,'$.name') AS name,effective_occurred_at_unix_nano AS occurred,canonical_envelope_id FROM {relation} WHERE {} AND session_id IS NOT NULL),sequenced AS (SELECT *,{} FROM base WINDOW session_order AS (PARTITION BY session_id ORDER BY occurred,canonical_envelope_id)) {} ORDER BY sessions DESC,depth,from_name,to_name LIMIT ?",
            predicates.join(" AND "),
            neighbors.join(","),
            transitions.join(" UNION ALL ")
        ),
        arguments,
        columns: columns_for(Kind::Path),
    })
}

fn compile_retention(
    plan: &Plan,
    relation: &str,
    mut arguments: Vec<Argument>,
) -> Result<Compiled, QueryError> {
    let retention = plan
        .retention
        .as_ref()
        .ok_or(QueryError("retention payload missing"))?;
    let start = behavior_where(plan.range, &retention.start_filter, &mut arguments)?;
    let returning = behavior_where(plan.range, &retention.return_filter, &mut arguments)?;
    arguments.push(Argument::Integer(
        i64::try_from(retention.periods)
            .map_err(|_| QueryError("retention periods exceed signed range"))?,
    ));
    arguments.push(Argument::Integer(as_limit(retention.limit)?));
    let interval = retention.interval.as_str();
    Ok(Compiled {
        sql: format!(
            "WITH base AS MATERIALIZED (SELECT *,json_extract_string(canonical_json,'$.subject_id') AS subject_id,json_extract_string(canonical_json,'$.kind') AS kind,json_extract_string(canonical_json,'$.operation') AS operation,json_extract_string(canonical_json,'$.name') AS name FROM {relation}),starts AS MATERIALIZED (SELECT subject_id,min(effective_occurred_at_unix_nano) AS started FROM base WHERE {} GROUP BY subject_id),cohorts AS MATERIALIZED (SELECT subject_id,CAST(date_trunc('{interval}',make_timestamp_ns(CAST(started AS BIGINT))) AS DATE) AS cohort_date FROM starts WHERE subject_id IS NOT NULL),returns AS MATERIALIZED (SELECT subject_id,CAST(date_trunc('{interval}',make_timestamp_ns(CAST(effective_occurred_at_unix_nano AS BIGINT))) AS DATE) AS return_date FROM base WHERE {}),sizes AS (SELECT cohort_date,count(*) AS cohort_subjects FROM cohorts GROUP BY cohort_date) SELECT CAST(cohort.cohort_date AS VARCHAR) AS cohort_bucket,date_diff('{interval}',cohort.cohort_date,returns.return_date) AS period,sizes.cohort_subjects,count(DISTINCT returns.subject_id) AS retained_subjects,count(DISTINCT returns.subject_id)::DOUBLE/sizes.cohort_subjects AS retention_rate FROM cohorts cohort JOIN sizes USING(cohort_date) JOIN returns USING(subject_id) WHERE returns.return_date>=cohort.cohort_date AND date_diff('{interval}',cohort.cohort_date,returns.return_date)<=? GROUP BY cohort.cohort_date,period,sizes.cohort_subjects ORDER BY cohort.cohort_date,period LIMIT ?",
            start.join(" AND "),
            returning.join(" AND ")
        ),
        arguments,
        columns: columns_for(Kind::Retention),
    })
}

fn behavior_where(
    range: TimeRange,
    filter: &BehaviorFilter,
    arguments: &mut Vec<Argument>,
) -> Result<Vec<String>, QueryError> {
    let mut predicates = vec![
        "envelope_kind='behavior'".to_owned(),
        "effective_occurred_at_unix_nano>=?".to_owned(),
        "effective_occurred_at_unix_nano<?".to_owned(),
    ];
    arguments.extend([
        Argument::Integer(as_i64(range.start_unix_nano)?),
        Argument::Integer(as_i64(range.end_unix_nano)?),
    ]);
    for (column, value) in [
        ("kind", &filter.behavior_kind),
        ("operation", &filter.operation),
        ("name", &filter.name),
    ] {
        if !value.is_empty() {
            predicates.push(format!("{column}=?"));
            arguments.push(Argument::Text(value.clone()));
        }
    }
    for (key, value) in &filter.annotations {
        predicates.push("json_extract_string(canonical_json,?)=?".to_owned());
        arguments.extend([
            Argument::Text(format!("/annotations/{}", escape_json_pointer(key))),
            Argument::Text(value.clone()),
        ]);
    }
    Ok(predicates)
}

fn step_where(step: &FunnelStep, arguments: &mut Vec<Argument>) -> Vec<String> {
    let mut predicates = Vec::new();
    for (column, value) in [
        ("kind", &step.kind),
        ("operation", &step.operation),
        ("name", &step.name),
    ] {
        if !value.is_empty() {
            predicates.push(format!("{column}=?"));
            arguments.push(Argument::Text(value.clone()));
        }
    }
    for (key, value) in &step.annotations {
        predicates.push("json_extract_string(canonical_json,?)=?".to_owned());
        arguments.extend([
            Argument::Text(format!("/annotations/{}", escape_json_pointer(key))),
            Argument::Text(value.clone()),
        ]);
    }
    predicates
}

fn escape_json_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn as_i64(value: u64) -> Result<i64, QueryError> {
    i64::try_from(value).map_err(|_| QueryError("query numeric value exceeds signed range"))
}

fn as_limit(value: usize) -> Result<i64, QueryError> {
    i64::try_from(value).map_err(|_| QueryError("query limit exceeds signed range"))
}

fn columns_for(kind: Kind) -> Vec<&'static str> {
    match kind {
        Kind::Events => vec![
            "canonical_envelope_id",
            "record_id",
            "subject_id",
            "kind",
            "operation",
            "name",
            "session_id",
            "trace_id",
            "span_id",
            "effective_occurred_at_unix_nano",
            "canonical_json",
        ],
        Kind::Trace => vec![
            "canonical_envelope_id",
            "envelope_kind",
            "record_id",
            "trace_id",
            "span_id",
            "effective_occurred_at_unix_nano",
            "canonical_json",
        ],
        Kind::Replay => vec![
            "canonical_envelope_id",
            "replay_id",
            "replay_chunk_id",
            "session_id",
            "installation_id",
            "effective_occurred_at_unix_nano",
            "canonical_json",
        ],
        Kind::Funnel => vec![
            "step_index",
            "step_label",
            "breakdown_value",
            "subjects",
            "example_session_id",
            "example_trace_id",
        ],
        Kind::Cohort => vec!["subject_id", "first_seen", "last_seen", "event_count"],
        Kind::Aggregate => vec!["bucket", "dimension_value", "metric_value"],
        Kind::Path => vec![
            "direction",
            "depth",
            "from_name",
            "to_name",
            "subjects",
            "sessions",
            "example_session_id",
            "example_trace_id",
        ],
        Kind::Retention => vec![
            "cohort_bucket",
            "period",
            "cohort_subjects",
            "retained_subjects",
            "retention_rate",
        ],
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, time::Duration};

    use super::*;
    use crate::{EventsPlan, PLAN_VERSION};

    fn events_plan(name: &str) -> Plan {
        Plan {
            version: PLAN_VERSION,
            kind: Kind::Events,
            range: TimeRange {
                start_unix_nano: 1,
                end_unix_nano: 1_000,
            },
            events: Some(EventsPlan {
                filter: BehaviorFilter {
                    name: name.to_owned(),
                    ..BehaviorFilter::default()
                },
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
        }
    }

    #[test]
    fn keeps_untrusted_values_out_of_sql() -> Result<(), QueryError> {
        let attack = "checkout' OR read_text('/etc/passwd') IS NOT NULL --";
        let path = "/safe/cache/digest.parquet";
        let mut plan = events_plan(attack);
        plan.events
            .as_mut()
            .ok_or(QueryError("test payload missing"))?
            .filter
            .annotations = BTreeMap::from([("danger/key".to_owned(), attack.to_owned())]);
        plan.validate(Duration::from_hours(1), 100)?;
        let compiled = compile(&plan, &[path.to_owned()])?;
        assert!(!compiled.sql.contains(attack));
        assert!(!compiled.sql.contains(path));
        assert!(
            compiled
                .arguments
                .contains(&Argument::Text(attack.to_owned()))
        );
        assert!(
            compiled
                .arguments
                .contains(&Argument::Text("/annotations/danger~1key".to_owned()))
        );
        Ok(())
    }

    #[test]
    fn rejects_ambiguous_and_unbounded_plans() {
        let mut ambiguous = events_plan("");
        ambiguous.trace = Some(crate::TracePlan {
            trace_id: "11111111111111111111111111111111".to_owned(),
            limit: 1,
        });
        assert!(ambiguous.validate(Duration::from_hours(1), 100).is_err());
        let mut unbounded = events_plan("");
        unbounded.range.end_unix_nano = 7_200_000_000_000;
        assert!(unbounded.validate(Duration::from_hours(1), 100).is_err());
    }
}
