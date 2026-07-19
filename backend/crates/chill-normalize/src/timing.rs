use std::time::Duration;

use time::OffsetDateTime;

/// Stable source/server timing classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimingClass {
    /// Source time is within policy bounds.
    OnTime,
    /// Source time predates the late threshold.
    Late,
    /// Source time exceeds the future allowance.
    FutureClockSkew,
    /// No source wall-clock time was supplied.
    MissingSourceTime,
}

impl TimingClass {
    /// Returns the database wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OnTime => "on_time",
            Self::Late => "late",
            Self::FutureClockSkew => "future_clock_skew",
            Self::MissingSourceTime => "missing_source_time",
        }
    }
}

/// Bounds used to classify source wall-clock timestamps.
#[derive(Clone, Copy, Debug)]
pub struct TimingPolicy {
    /// Permitted future source skew.
    pub future_allowance: Duration,
    /// Age after which a record is a late arrival.
    pub late_after: Duration,
}

impl Default for TimingPolicy {
    fn default() -> Self {
        Self {
            future_allowance: Duration::from_mins(5),
            late_after: Duration::from_hours(24),
        }
    }
}

/// Derived canonical timing fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Timing {
    /// Classification.
    pub class: TimingClass,
    /// Trusted database receipt time.
    pub server_received_nano: u64,
    /// Time used for retention and query order.
    pub effective_occurred: u64,
    /// Canonical observation time.
    pub canonical_observed: u64,
    /// Signed server-minus-source difference.
    pub clock_skew_nano: i128,
    /// Whether this is a late arrival.
    pub late: bool,
}

impl TimingPolicy {
    /// Classifies a source timestamp without rewriting an authoritative in-range value.
    #[must_use]
    pub fn classify(self, received: OffsetDateTime, source: Option<u64>) -> Timing {
        let received_nano = u64::try_from(
            received
                .unix_timestamp_nanos()
                .clamp(0, i128::from(u64::MAX)),
        )
        .unwrap_or(u64::MAX);
        let mut result = Timing {
            class: TimingClass::MissingSourceTime,
            server_received_nano: received_nano,
            effective_occurred: received_nano,
            canonical_observed: received_nano,
            clock_skew_nano: 0,
            late: false,
        };
        let Some(source) = source else {
            return result;
        };
        result.canonical_observed = received_nano.max(source);
        result.clock_skew_nano = i128::from(received_nano) - i128::from(source);
        let future_limit = received_nano.saturating_add(duration_nanos(self.future_allowance));
        if source > future_limit {
            result.class = TimingClass::FutureClockSkew;
            return result;
        }
        result.effective_occurred = source;
        if received_nano.saturating_sub(source) > duration_nanos(self.late_after) {
            result.class = TimingClass::Late;
            result.late = true;
            return result;
        }
        result.class = TimingClass::OnTime;
        result
    }
}

fn duration_nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{TimingClass, TimingPolicy};
    use time::OffsetDateTime;

    #[test]
    fn preserves_source_time_and_classifies_late_and_future_values() {
        let received = OffsetDateTime::from_unix_timestamp(1_800_000_000)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
        let nanos = u64::try_from(received.unix_timestamp_nanos()).unwrap_or_default();
        let policy = TimingPolicy::default();
        assert_eq!(
            policy.classify(received, None).class,
            TimingClass::MissingSourceTime
        );
        assert_eq!(
            policy
                .classify(received, Some(nanos - 25 * 60 * 60 * 1_000_000_000))
                .class,
            TimingClass::Late
        );
        let future = nanos + 5 * 60 * 1_000_000_000 + 1;
        let timing = policy.classify(received, Some(future));
        assert_eq!(timing.class, TimingClass::FutureClockSkew);
        assert_eq!(timing.effective_occurred, nanos);
    }
}
