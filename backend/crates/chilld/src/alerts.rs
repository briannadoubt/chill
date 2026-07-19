use std::{sync::Arc, time::Duration};

use chill_query::{Plan, Result as QueryResult, Scope, Service as QueryService};
use serde_json::Value;
use sqlx::{PgPool, types::Json};
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
    for (id, organization_id, project_id, environment_id, Json(plan), operator, threshold, _) in
        alerts
    {
        let plan: Plan = match serde_json::from_value(plan) {
            Ok(value) => value,
            Err(cause) => {
                warn!(alert_id = %id, %cause, "scheduled analytics alert plan is malformed");
                let completed = sqlx::query_scalar::<_, bool>(
                    "SELECT product.complete_alert_evaluation($1::uuid,$2,$3)",
                )
                .bind(&id)
                .bind(0.0_f64)
                .bind("error")
                .fetch_one(database)
                .await?;
                if !completed {
                    warn!(
                        alert_id = %id,
                        "malformed scheduled analytics alert was no longer active at completion"
                    );
                }
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
}
