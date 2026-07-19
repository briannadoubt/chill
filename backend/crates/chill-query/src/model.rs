use std::{collections::BTreeMap, time::Duration};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Current public typed-query plan version.
pub const PLAN_VERSION: u8 = 1;

/// A rejected typed-query plan.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("{0}")]
pub struct QueryError(pub(crate) &'static str);

/// Exact tenant scope for one query.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Scope {
    /// Owning organization UUID.
    pub organization_id: String,
    /// Project UUID.
    pub project_id: String,
    /// Environment UUID.
    pub environment_id: String,
}

impl Scope {
    /// Validates canonical lowercase UUIDs.
    ///
    /// # Errors
    ///
    /// Returns an error when any scope component is not a canonical UUID.
    pub fn validate(&self) -> Result<(), QueryError> {
        for value in [
            &self.organization_id,
            &self.project_id,
            &self.environment_id,
        ] {
            let parsed = Uuid::parse_str(value).map_err(|_| QueryError("scope UUID is invalid"))?;
            if parsed.to_string() != *value {
                return Err(QueryError("scope UUID is not canonical"));
            }
        }
        Ok(())
    }
}

/// Inclusive-start, exclusive-end nanosecond range.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TimeRange {
    /// Inclusive Unix epoch nanoseconds.
    pub start_unix_nano: u64,
    /// Exclusive Unix epoch nanoseconds.
    pub end_unix_nano: u64,
}

impl TimeRange {
    fn validate(self, maximum: Duration) -> Result<(), QueryError> {
        let maximum_nanos = u64::try_from(maximum.as_nanos()).unwrap_or(u64::MAX);
        if self.start_unix_nano > i64::MAX as u64
            || self.end_unix_nano > i64::MAX as u64
            || self.end_unix_nano <= self.start_unix_nano
            || self.end_unix_nano - self.start_unix_nano > maximum_nanos
        {
            return Err(QueryError(
                "query time range is invalid or exceeds its bound",
            ));
        }
        Ok(())
    }
}

/// Supported query shape.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Paginated behavior events.
    Events,
    /// Trace records.
    Trace,
    /// Replay chunks.
    Replay,
    /// Ordered conversion steps.
    Funnel,
    /// Subjects meeting a frequency criterion.
    Cohort,
    /// Time-bucketed aggregate.
    Aggregate,
    /// Navigation path transitions.
    Path,
    /// Cohort retention matrix.
    Retention,
}

/// Versioned, exactly-one-payload query plan.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Plan {
    /// Must equal [`PLAN_VERSION`].
    pub version: u8,
    /// Payload discriminator.
    pub kind: Kind,
    /// Scan range.
    pub range: TimeRange,
    /// Event payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<EventsPlan>,
    /// Trace payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TracePlan>,
    /// Replay payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay: Option<ReplayPlan>,
    /// Funnel payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funnel: Option<FunnelPlan>,
    /// Cohort payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cohort: Option<CohortPlan>,
    /// Aggregate payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate: Option<AggregatePlan>,
    /// Path-discovery payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathPlan>,
    /// Retention payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<RetentionPlan>,
}

/// Optional canonical behavior predicates.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct BehaviorFilter {
    /// Canonical behavior kind.
    #[serde(default)]
    pub behavior_kind: String,
    /// Canonical operation.
    #[serde(default)]
    pub operation: String,
    /// Canonical name.
    #[serde(default)]
    pub name: String,
    /// Exact annotation values, emitted in sorted-key order.
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

impl BehaviorFilter {
    fn validate(&self) -> Result<(), QueryError> {
        if [&self.behavior_kind, &self.operation, &self.name]
            .into_iter()
            .any(|value| value.len() > 160)
            || self.annotations.len() > 8
            || self
                .annotations
                .iter()
                .any(|(key, value)| key.is_empty() || key.len() > 128 || value.len() > 512)
        {
            return Err(QueryError("behavior filter is invalid"));
        }
        Ok(())
    }
}

/// Behavior event query.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EventsPlan {
    /// Canonical behavior predicates.
    pub filter: BehaviorFilter,
    /// Optional session ID.
    #[serde(default)]
    pub session_id: String,
    /// Optional lowercase 16-byte hex trace ID.
    #[serde(default)]
    pub trace_id: String,
    /// Optional installation ID.
    #[serde(default)]
    pub installation_id: String,
    /// Optional descending pagination cursor.
    pub before: Option<Cursor>,
    /// Row limit.
    pub limit: usize,
}

/// Stable event pagination cursor.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Cursor {
    /// Effective occurrence time.
    pub effective_occurred_at_unix_nano: u64,
    /// Stable database identity tie-breaker.
    pub canonical_envelope_id: i64,
}

/// Trace query.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TracePlan {
    /// Lowercase 16-byte hex trace ID.
    pub trace_id: String,
    /// Row limit.
    pub limit: usize,
}

/// Replay query.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReplayPlan {
    /// Exact replay ID; mutually exclusive with session ID.
    #[serde(default)]
    pub replay_id: String,
    /// Exact session ID; mutually exclusive with replay ID.
    #[serde(default)]
    pub session_id: String,
    /// Row limit.
    pub limit: usize,
}

/// Ordered conversion query.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FunnelPlan {
    /// `ordered` or `unordered` conversion semantics.
    #[serde(default = "default_funnel_mode")]
    pub mode: String,
    /// Two through eight ordered steps.
    pub steps: Vec<FunnelStep>,
    /// Zero through four subject-level exclusion predicates.
    #[serde(default)]
    pub exclusions: Vec<FunnelStep>,
    /// Allowlisted first-step breakdown dimension.
    #[serde(default = "default_funnel_breakdown")]
    pub breakdown: String,
    /// Maximum elapsed nanoseconds from first to final step.
    pub completion_window_nano: u64,
}

/// One funnel criterion.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FunnelStep {
    /// Public output label.
    pub label: String,
    /// Optional behavior kind.
    #[serde(default)]
    pub kind: String,
    /// Optional operation.
    #[serde(default)]
    pub operation: String,
    /// Optional name.
    #[serde(default)]
    pub name: String,
    /// Exact annotation values required by this step.
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

/// Subject cohort query.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CohortPlan {
    /// Canonical behavior predicates.
    pub filter: BehaviorFilter,
    /// Minimum matching events.
    pub minimum_count: usize,
    /// Optional ordered two-through-eight event/property sequence. When set,
    /// it replaces the frequency predicate above.
    #[serde(default)]
    pub sequence: Vec<FunnelStep>,
    /// Maximum elapsed nanoseconds from the first to final sequence event.
    #[serde(default)]
    pub sequence_window_nano: u64,
    /// Subject limit.
    pub limit: usize,
}

/// Time-bucketed aggregate query.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AggregatePlan {
    /// `count` or `unique_subjects`.
    pub metric: String,
    /// Allowlisted grouping dimension.
    pub dimension: String,
    /// `minute`, `hour`, or `day`.
    pub interval: String,
    /// Canonical behavior predicates.
    pub filter: BehaviorFilter,
    /// Bucket-row limit.
    pub limit: usize,
}

/// Session-scoped path discovery around one semantic event.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PathPlan {
    /// Anchor behavior name.
    pub anchor_name: String,
    /// `before` or `after` the anchor.
    pub direction: String,
    /// Maximum transition depth, one through five.
    pub depth: usize,
    /// Predicates applied before path sequencing.
    pub filter: BehaviorFilter,
    /// Transition-row limit.
    pub limit: usize,
}

/// Retention calculation from a cohort-forming event to a returning event.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RetentionPlan {
    /// Cohort-forming behavior predicate.
    pub start_filter: BehaviorFilter,
    /// Return behavior predicate.
    pub return_filter: BehaviorFilter,
    /// `day` or `week` cohort interval.
    pub interval: String,
    /// Number of periods, one through fifty-two.
    pub periods: usize,
    /// Matrix-row limit.
    pub limit: usize,
}

impl Plan {
    /// Validates shape, range, identifiers, allowlists, and row bounds.
    ///
    /// # Errors
    ///
    /// Returns an error when the plan violates any public query bound or its
    /// kind does not select exactly one matching payload.
    pub fn validate(&self, maximum_range: Duration, maximum_rows: usize) -> Result<(), QueryError> {
        if self.version != PLAN_VERSION {
            return Err(QueryError("query plan version is invalid"));
        }
        self.range.validate(maximum_range)?;
        let payload_count = [
            self.events.is_some(),
            self.trace.is_some(),
            self.replay.is_some(),
            self.funnel.is_some(),
            self.cohort.is_some(),
            self.aggregate.is_some(),
            self.path.is_some(),
            self.retention.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        let matches_kind = match self.kind {
            Kind::Events => self.events.is_some(),
            Kind::Trace => self.trace.is_some(),
            Kind::Replay => self.replay.is_some(),
            Kind::Funnel => self.funnel.is_some(),
            Kind::Cohort => self.cohort.is_some(),
            Kind::Aggregate => self.aggregate.is_some(),
            Kind::Path => self.path.is_some(),
            Kind::Retention => self.retention.is_some(),
        };
        if payload_count != 1 || !matches_kind {
            return Err(QueryError("query kind and payload do not match exactly"));
        }
        match self.kind {
            Kind::Events => self.validate_events(maximum_rows),
            Kind::Trace => {
                let plan = self
                    .trace
                    .as_ref()
                    .ok_or(QueryError("trace payload missing"))?;
                if !valid_trace_id(&plan.trace_id) {
                    return Err(QueryError("trace ID is invalid"));
                }
                validate_limit(plan.limit, maximum_rows)
            }
            Kind::Replay => self.validate_replay(maximum_rows),
            Kind::Funnel => self.validate_funnel(maximum_range),
            Kind::Cohort => {
                let plan = self
                    .cohort
                    .as_ref()
                    .ok_or(QueryError("cohort payload missing"))?;
                plan.filter.validate()?;
                if !(1..=1_000_000).contains(&plan.minimum_count) {
                    return Err(QueryError("cohort minimum count is invalid"));
                }
                let maximum_nanos = u64::try_from(maximum_range.as_nanos()).unwrap_or(u64::MAX);
                if !plan.sequence.is_empty()
                    && (!(2..=8).contains(&plan.sequence.len())
                        || plan.sequence_window_nano == 0
                        || plan.sequence_window_nano > i64::MAX as u64
                        || plan.sequence_window_nano > maximum_nanos)
                {
                    return Err(QueryError("cohort sequence bounds are invalid"));
                }
                for step in &plan.sequence {
                    validate_funnel_step(step)?;
                }
                validate_limit(plan.limit, maximum_rows)
            }
            Kind::Aggregate => self.validate_aggregate(maximum_rows),
            Kind::Path => self.validate_path(maximum_rows),
            Kind::Retention => self.validate_retention(maximum_rows),
        }
    }

    fn validate_events(&self, maximum_rows: usize) -> Result<(), QueryError> {
        let plan = self
            .events
            .as_ref()
            .ok_or(QueryError("events payload missing"))?;
        plan.filter.validate()?;
        validate_limit(plan.limit, maximum_rows)?;
        if !plan.trace_id.is_empty() && !valid_trace_id(&plan.trace_id) {
            return Err(QueryError("event trace ID is invalid"));
        }
        if plan.session_id.len() > 256 || plan.installation_id.len() > 256 {
            return Err(QueryError("query identifier is too long"));
        }
        if plan.before.is_some_and(|cursor| {
            cursor.canonical_envelope_id < 1
                || cursor.effective_occurred_at_unix_nano > i64::MAX as u64
        }) {
            return Err(QueryError("event cursor is invalid"));
        }
        Ok(())
    }

    fn validate_replay(&self, maximum_rows: usize) -> Result<(), QueryError> {
        let plan = self
            .replay
            .as_ref()
            .ok_or(QueryError("replay payload missing"))?;
        if plan.replay_id.is_empty() == plan.session_id.is_empty()
            || plan.replay_id.len() > 256
            || plan.session_id.len() > 256
        {
            return Err(QueryError("replay query requires one valid identifier"));
        }
        validate_limit(plan.limit, maximum_rows)
    }

    fn validate_funnel(&self, maximum_range: Duration) -> Result<(), QueryError> {
        let plan = self
            .funnel
            .as_ref()
            .ok_or(QueryError("funnel payload missing"))?;
        let maximum_nanos = u64::try_from(maximum_range.as_nanos()).unwrap_or(u64::MAX);
        if !["ordered", "unordered"].contains(&plan.mode.as_str())
            || !(2..=8).contains(&plan.steps.len())
            || plan.exclusions.len() > 4
            || !["none", "kind", "name", "page", "platform"].contains(&plan.breakdown.as_str())
            || plan.completion_window_nano == 0
            || plan.completion_window_nano > i64::MAX as u64
            || plan.completion_window_nano > maximum_nanos
        {
            return Err(QueryError("funnel bounds are invalid"));
        }
        for step in plan.steps.iter().chain(&plan.exclusions) {
            validate_funnel_step(step)?;
        }
        Ok(())
    }

    fn validate_aggregate(&self, maximum_rows: usize) -> Result<(), QueryError> {
        let plan = self
            .aggregate
            .as_ref()
            .ok_or(QueryError("aggregate payload missing"))?;
        if !["count", "unique_subjects"].contains(&plan.metric.as_str())
            || !["none", "kind", "operation", "name", "page", "platform"]
                .contains(&plan.dimension.as_str())
            || !["minute", "hour", "day"].contains(&plan.interval.as_str())
        {
            return Err(QueryError("aggregate allowlist value is invalid"));
        }
        plan.filter.validate()?;
        validate_limit(plan.limit, maximum_rows)
    }

    fn validate_path(&self, maximum_rows: usize) -> Result<(), QueryError> {
        let plan = self
            .path
            .as_ref()
            .ok_or(QueryError("path payload missing"))?;
        if plan.anchor_name.is_empty()
            || plan.anchor_name.len() > 160
            || !["before", "after"].contains(&plan.direction.as_str())
            || !(1..=5).contains(&plan.depth)
        {
            return Err(QueryError("path query is invalid"));
        }
        plan.filter.validate()?;
        validate_limit(plan.limit, maximum_rows)
    }

    fn validate_retention(&self, maximum_rows: usize) -> Result<(), QueryError> {
        let plan = self
            .retention
            .as_ref()
            .ok_or(QueryError("retention payload missing"))?;
        if !["day", "week"].contains(&plan.interval.as_str()) || !(1..=52).contains(&plan.periods) {
            return Err(QueryError("retention query is invalid"));
        }
        plan.start_filter.validate()?;
        plan.return_filter.validate()?;
        validate_limit(plan.limit, maximum_rows)
    }

    /// Envelope kinds needed to answer this plan.
    #[must_use]
    pub fn envelope_kinds(&self) -> &'static [&'static str] {
        match self.kind {
            Kind::Trace => &["behavior", "otel.span"],
            Kind::Replay => &["replay"],
            _ => &["behavior"],
        }
    }
}

fn validate_funnel_step(step: &FunnelStep) -> Result<(), QueryError> {
    if step.label.is_empty()
        || step.label.len() > 80
        || (step.kind.is_empty() && step.operation.is_empty() && step.name.is_empty())
    {
        return Err(QueryError("funnel step is invalid"));
    }
    BehaviorFilter {
        behavior_kind: step.kind.clone(),
        operation: step.operation.clone(),
        name: step.name.clone(),
        annotations: step.annotations.clone(),
    }
    .validate()
}

fn default_funnel_mode() -> String {
    "ordered".to_owned()
}

fn default_funnel_breakdown() -> String {
    "none".to_owned()
}

fn validate_limit(value: usize, maximum: usize) -> Result<(), QueryError> {
    if value == 0 || value > maximum {
        return Err(QueryError("query limit is outside its bound"));
    }
    Ok(())
}

fn valid_trace_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.bytes().any(|byte| byte != b'0')
}

/// Stable typed query result.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct QueryResult {
    /// Public columns in plan-defined order.
    pub columns: Vec<String>,
    /// JSON-compatible scalar rows.
    pub rows: Vec<Vec<serde_json::Value>>,
    /// Resource and cache observations.
    pub stats: Stats,
}

/// Resource observations attached to a query result.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Stats {
    /// Whether the result came from the result cache.
    pub cache_hit: bool,
    /// Hash of the immutable input dataset and policies.
    pub manifest_generation: String,
    /// Number of immutable files scanned.
    pub file_count: usize,
    /// Declared compressed bytes scanned.
    pub scan_bytes: i64,
    /// Result row count.
    pub row_count: usize,
    /// Queue wait in nanoseconds.
    pub queue_duration_nano: u64,
    /// Catalog planning in nanoseconds.
    pub planning_duration_nano: u64,
    /// File materialization in nanoseconds.
    pub materialize_duration_nano: u64,
    /// `DuckDB` execution in nanoseconds.
    pub execution_duration_nano: u64,
    /// End-to-end duration in nanoseconds.
    pub total_duration_nano: u64,
}
