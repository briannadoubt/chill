use std::{
    array,
    fmt::Write as _,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const LATENCY_BUCKETS: [(u64, &str); 5] = [
    (5, "0.005"),
    (25, "0.025"),
    (100, "0.1"),
    (500, "0.5"),
    (2_000, "2"),
];
const SURFACES: [&str; 8] = [
    "ingest",
    "replay",
    "query",
    "console",
    "collection",
    "grpc",
    "operational",
    "other",
];

pub(crate) struct Metrics {
    surfaces: [SurfaceMetrics; SURFACES.len()],
    active_requests: AtomicU64,
}

struct SurfaceMetrics {
    total: AtomicU64,
    failed: AtomicU64,
    rejected: AtomicU64,
    throttled: AtomicU64,
    duration_microseconds: AtomicU64,
    duration_buckets: [AtomicU64; LATENCY_BUCKETS.len() + 1],
}

impl Default for SurfaceMetrics {
    fn default() -> Self {
        Self {
            total: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            throttled: AtomicU64::new(0),
            duration_microseconds: AtomicU64::new(0),
            duration_buckets: array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            surfaces: array::from_fn(|_| SurfaceMetrics::default()),
            active_requests: AtomicU64::new(0),
        }
    }
}

impl Metrics {
    pub(crate) fn started(&self) {
        self.active_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn finished(&self, path: &str, status: u16, duration: Duration) {
        self.active_requests.fetch_sub(1, Ordering::Relaxed);
        let metric = &self.surfaces[surface_index(path)];
        metric.total.fetch_add(1, Ordering::Relaxed);
        if status >= 500 {
            metric.failed.fetch_add(1, Ordering::Relaxed);
        } else if status == 429 {
            metric.throttled.fetch_add(1, Ordering::Relaxed);
        } else if matches!(status, 400 | 401 | 403 | 409 | 413 | 415 | 422) {
            metric.rejected.fetch_add(1, Ordering::Relaxed);
        }
        let micros = u64::try_from(duration.as_micros()).unwrap_or(u64::MAX);
        metric
            .duration_microseconds
            .fetch_add(micros, Ordering::Relaxed);
        let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        for (index, (upper, _)) in LATENCY_BUCKETS.iter().enumerate() {
            if millis <= *upper {
                metric.duration_buckets[index].fetch_add(1, Ordering::Relaxed);
            }
        }
        metric.duration_buckets[LATENCY_BUCKETS.len()].fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn render(&self, pool_size: u32, pool_idle: usize) -> String {
        let mut output = String::from(
            "# HELP chill_up Whether the Chill server process is running.\n# TYPE chill_up gauge\nchill_up 1\n\
             # HELP chill_http_active_requests Requests currently executing.\n# TYPE chill_http_active_requests gauge\n",
        );
        let _ = writeln!(
            output,
            "chill_http_active_requests {}",
            self.active_requests.load(Ordering::Relaxed)
        );
        output.push_str(
            "# HELP chill_http_requests_total Requests completed by bounded surface and outcome.\n# TYPE chill_http_requests_total counter\n",
        );
        for (index, name) in SURFACES.iter().enumerate() {
            let metric = &self.surfaces[index];
            for (outcome, value) in [
                ("total", metric.total.load(Ordering::Relaxed)),
                ("failed", metric.failed.load(Ordering::Relaxed)),
                ("rejected", metric.rejected.load(Ordering::Relaxed)),
                ("throttled", metric.throttled.load(Ordering::Relaxed)),
            ] {
                let _ = writeln!(
                    output,
                    "chill_http_requests_total{{surface=\"{name}\",outcome=\"{outcome}\"}} {value}"
                );
            }
        }
        output.push_str(
            "# HELP chill_http_request_duration_seconds Request duration by bounded surface.\n# TYPE chill_http_request_duration_seconds histogram\n",
        );
        for (index, name) in SURFACES.iter().enumerate() {
            let metric = &self.surfaces[index];
            for (bucket, (_, upper)) in LATENCY_BUCKETS.iter().enumerate() {
                let _ = writeln!(
                    output,
                    "chill_http_request_duration_seconds_bucket{{surface=\"{name}\",le=\"{}\"}} {}",
                    upper,
                    metric.duration_buckets[bucket].load(Ordering::Relaxed)
                );
            }
            let _ = writeln!(
                output,
                "chill_http_request_duration_seconds_bucket{{surface=\"{name}\",le=\"+Inf\"}} {}",
                metric.duration_buckets[LATENCY_BUCKETS.len()].load(Ordering::Relaxed)
            );
            let micros = metric.duration_microseconds.load(Ordering::Relaxed);
            let _ = writeln!(
                output,
                "chill_http_request_duration_seconds_sum{{surface=\"{name}\"}} {}.{:06}",
                micros / 1_000_000,
                micros % 1_000_000
            );
            let _ = writeln!(
                output,
                "chill_http_request_duration_seconds_count{{surface=\"{name}\"}} {}",
                metric.total.load(Ordering::Relaxed)
            );
        }
        output.push_str(
            "# HELP chill_postgres_connections PostgreSQL pool connections by state.\n# TYPE chill_postgres_connections gauge\n",
        );
        let _ = writeln!(
            output,
            "chill_postgres_connections{{state=\"open\"}} {pool_size}"
        );
        let _ = writeln!(
            output,
            "chill_postgres_connections{{state=\"idle\"}} {pool_idle}"
        );
        output
    }
}

fn surface_index(path: &str) -> usize {
    if matches!(path, "/v1/logs" | "/v1/traces" | "/v1/metrics") {
        0
    } else if path == "/v1/chill/replay" {
        1
    } else if path.starts_with("/v1/query") {
        2
    } else if path.starts_with("/v1/console") {
        3
    } else if path.starts_with("/v1/collection") {
        4
    } else if path.starts_with("/opentelemetry.proto") {
        5
    } else if matches!(
        path,
        "/healthz" | "/livez" | "/readyz" | "/metrics" | "/version"
    ) {
        6
    } else {
        7
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_are_low_cardinality_and_cumulative() {
        let metrics = Metrics::default();
        metrics.started();
        metrics.finished("/v1/logs", 429, Duration::from_millis(20));
        let rendered = metrics.render(7, 3);
        assert!(
            rendered
                .contains("chill_http_requests_total{surface=\"ingest\",outcome=\"throttled\"} 1")
        );
        assert!(rendered.contains(
            "chill_http_request_duration_seconds_bucket{surface=\"ingest\",le=\"0.025\"} 1"
        ));
        assert!(rendered.contains("chill_postgres_connections{state=\"idle\"} 3"));
        assert!(!rendered.contains("/v1/logs"));
    }
}
