use std::{sync::Arc, time::Duration};

use chill_query::{Plan, Result as QueryResult, Scope, Service as QueryService};
use serde_json::Value;
use sqlx::{PgPool, types::Json};
use time::OffsetDateTime;
use tracing::{error, info, warn};

type ClaimedAlert = (
    String,
    String,
    String,
    String,
    Json<Value>,
    String,
    f64,
    i32,
);

pub(crate) async fn run(database: PgPool, query: Arc<QueryService>) {
    let mut interval = tokio::time::interval(Duration::from_mins(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        if let Err(cause) = evaluate_due(&database, &query).await {
            error!(%cause, "scheduled analytics alert pass failed");
        }
    }
}

async fn evaluate_due(database: &PgPool, query: &QueryService) -> anyhow::Result<()> {
    let alerts = sqlx::query_as::<_, ClaimedAlert>(
        "SELECT alert_id,organization_id,project_id,environment_id,plan,operator,threshold,schedule_minutes FROM product.claim_due_alerts($1)",
    )
    .bind(32_i32)
    .fetch_all(database)
    .await?;
    let count = alerts.len();
    for (id, organization_id, project_id, environment_id, Json(mut plan), operator, threshold, _) in
        alerts
    {
        if let Err(cause) = resolve_relative_range(&mut plan, OffsetDateTime::now_utc()) {
            warn!(alert_id = %id, %cause, "scheduled analytics alert range is malformed");
            complete_error(database, &id).await?;
            continue;
        }
        let plan: Plan = match serde_json::from_value(plan) {
            Ok(value) => value,
            Err(cause) => {
                warn!(alert_id = %id, %cause, "scheduled analytics alert plan is malformed");
                complete_error(database, &id).await?;
                continue;
            }
        };
        let scope = Scope {
            organization_id,
            project_id,
            environment_id,
        };
        let outcome = match query.execute(&scope, &plan).await {
            Ok(result) => match alert_value(&result) {
                Some(value) if compare(&operator, value, threshold) => (value, "triggered"),
                Some(value) => (value, "ok"),
                None => {
                    warn!(alert_id = %id, "scheduled analytics alert returned no numeric value");
                    (0.0, "error")
                }
            },
            Err(cause) => {
                warn!(alert_id = %id, %cause, "scheduled analytics alert query failed");
                (0.0, "error")
            }
        };
        let completed = sqlx::query_scalar::<_, bool>(
            "SELECT product.complete_alert_evaluation($1::uuid,$2,$3)",
        )
        .bind(&id)
        .bind(outcome.0)
        .bind(outcome.1)
        .fetch_one(database)
        .await?;
        if !completed {
            warn!(alert_id = %id, "scheduled analytics alert was no longer active at completion");
        }
    }
    if count > 0 {
        info!(count, "scheduled analytics alert pass completed");
    }
    Ok(())
}

async fn complete_error(database: &PgPool, id: &str) -> anyhow::Result<()> {
    let completed =
        sqlx::query_scalar::<_, bool>("SELECT product.complete_alert_evaluation($1::uuid,$2,$3)")
            .bind(id)
            .bind(0.0_f64)
            .bind("error")
            .fetch_one(database)
            .await?;
    if !completed {
        warn!(alert_id = %id, "malformed scheduled analytics alert was no longer active at completion");
    }
    Ok(())
}

fn resolve_relative_range(plan: &mut Value, now: OffsetDateTime) -> anyhow::Result<()> {
    let Some(range) = plan.get_mut("range").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    let Some(relative) = range.get("relative") else {
        return Ok(());
    };
    let relative = relative
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("relative range must be an object"))?;
    let amount = relative
        .get("amount")
        .and_then(Value::as_u64)
        .filter(|amount| (1..=744).contains(amount))
        .ok_or_else(|| anyhow::anyhow!("relative range amount is invalid"))?;
    let nanos_per_unit = match relative.get("unit").and_then(Value::as_str) {
        Some("hour") => 3_600_000_000_000_u64,
        Some("day") => 86_400_000_000_000_u64,
        Some("week") => 604_800_000_000_000_u64,
        _ => return Err(anyhow::anyhow!("relative range unit is invalid")),
    };
    let end = u64::try_from(now.unix_timestamp_nanos())
        .map_err(|_| anyhow::anyhow!("current time is outside the query range"))?;
    let duration = amount
        .checked_mul(nanos_per_unit)
        .ok_or_else(|| anyhow::anyhow!("relative range duration overflowed"))?;
    let start = end
        .checked_sub(duration)
        .ok_or_else(|| anyhow::anyhow!("relative range starts before the Unix epoch"))?;
    range.insert("start_unix_nano".into(), Value::from(start));
    range.insert("end_unix_nano".into(), Value::from(end));
    Ok(())
}

fn alert_value(result: &QueryResult) -> Option<f64> {
    result.rows.first().and_then(|row| {
        row.iter().rev().find_map(|value| match value {
            Value::Number(number) => number.as_f64().filter(|value| value.is_finite()),
            _ => None,
        })
    })
}

fn compare(operator: &str, value: f64, threshold: f64) -> bool {
    match operator {
        "gt" => value > threshold,
        "gte" => value >= threshold,
        "lt" => value < threshold,
        "lte" => value <= threshold,
        "eq" => (value - threshold).abs() <= f64::EPSILON,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use chill_query::{Result, Stats};
    use serde_json::json;
    use time::macros::datetime;

    use super::*;

    #[test]
    fn extracts_the_rightmost_numeric_result_and_applies_operators() {
        let result = Result {
            columns: vec!["bucket".into(), "metric_value".into()],
            rows: vec![vec![json!("2026-07-17"), json!(42)]],
            stats: Stats::default(),
        };
        assert_eq!(alert_value(&result), Some(42.0));
        assert!(compare("gte", 42.0, 42.0));
        assert!(compare("lt", 41.0, 42.0));
        assert!(!compare("gt", 41.0, 42.0));
    }

    #[test]
    fn rejects_non_numeric_results() {
        let result = Result {
            columns: vec!["value".into()],
            rows: vec![vec![json!("masked")]],
            stats: Stats::default(),
        };
        assert_eq!(alert_value(&result), None);
    }

    #[test]
    fn refreshes_relative_saved_query_ranges_before_scheduled_evaluation() {
        let mut plan = json!({
            "version": 1,
            "kind": "aggregate",
            "range": {
                "start_unix_nano": 1,
                "end_unix_nano": 2,
                "relative": { "amount": 7, "unit": "day" }
            },
            "aggregate": {}
        });
        let now = datetime!(2026-07-24 12:00 UTC);
        let resolution = resolve_relative_range(&mut plan, now);
        assert!(resolution.is_ok(), "{resolution:?}");
        let end = u64::try_from(now.unix_timestamp_nanos()).unwrap_or_default();
        assert_eq!(plan["range"]["end_unix_nano"], json!(end));
        assert_eq!(
            plan["range"]["start_unix_nano"],
            json!(end - 7 * 86_400_000_000_000_u64)
        );
    }

    #[test]
    fn rejects_malformed_relative_saved_query_ranges() {
        let mut plan = json!({
            "range": {
                "start_unix_nano": 1,
                "end_unix_nano": 2,
                "relative": { "amount": 0, "unit": "day" }
            }
        });
        assert!(resolve_relative_range(&mut plan, datetime!(2026-07-24 12:00 UTC)).is_err());
    }
}
