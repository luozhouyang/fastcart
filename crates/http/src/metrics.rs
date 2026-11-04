//! Minimal Prometheus-compatible HTTP metrics.
//!
//! The collector is dependency-free and intentionally low-cardinality: it
//! tracks total requests, error counts, in-flight requests, status classes and
//! a latency histogram. Route labels are omitted so an attacker cannot create
//! unbounded series.

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

/// Histogram bucket upper bounds in seconds. The implicit `+Inf` bucket is
/// reported from the total request count.
pub const LATENCY_BUCKETS_SECONDS: [f64; 12] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
];

/// Process-wide HTTP metrics.
#[derive(Debug)]
pub struct Metrics {
    requests_total: AtomicU64,
    errors_total: AtomicU64,
    requests_in_flight: AtomicI64,
    status_2xx: AtomicU64,
    status_3xx: AtomicU64,
    status_4xx: AtomicU64,
    status_5xx: AtomicU64,
    bucket_counts: [AtomicU64; LATENCY_BUCKETS_SECONDS.len()],
    duration_micros_total: AtomicU64,
    storefront_data_failures_total: AtomicU64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    /// Creates an empty collector.
    #[must_use]
    pub fn new() -> Self {
        Self {
            requests_total: AtomicU64::new(0),
            errors_total: AtomicU64::new(0),
            requests_in_flight: AtomicI64::new(0),
            status_2xx: AtomicU64::new(0),
            status_3xx: AtomicU64::new(0),
            status_4xx: AtomicU64::new(0),
            status_5xx: AtomicU64::new(0),
            bucket_counts: std::array::from_fn(|_| AtomicU64::new(0)),
            duration_micros_total: AtomicU64::new(0),
            storefront_data_failures_total: AtomicU64::new(0),
        }
    }

    /// Records a storefront data resolution failure that degraded a render.
    pub fn record_storefront_data_failure(&self) {
        self.storefront_data_failures_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Marks a request as started.
    pub fn begin_request(&self) {
        self.requests_in_flight.fetch_add(1, Ordering::Relaxed);
    }

    /// Marks a request as finished and records its status and duration.
    pub fn end_request(&self, status: u16, duration: Duration) {
        self.requests_in_flight.fetch_sub(1, Ordering::Relaxed);
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        if status >= 400 {
            self.errors_total.fetch_add(1, Ordering::Relaxed);
        }
        let status_counter = match status {
            200..=299 => &self.status_2xx,
            300..=399 => &self.status_3xx,
            400..=499 => &self.status_4xx,
            _ => &self.status_5xx,
        };
        status_counter.fetch_add(1, Ordering::Relaxed);

        let seconds = duration.as_secs_f64();
        if let Some(index) = LATENCY_BUCKETS_SECONDS
            .iter()
            .position(|upper| seconds <= *upper)
        {
            self.bucket_counts[index].fetch_add(1, Ordering::Relaxed);
        }
        self.duration_micros_total.fetch_add(
            u64::try_from(duration.as_micros()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }

    /// Renders metrics in Prometheus text exposition format.
    #[must_use]
    pub fn render(&self) -> String {
        let requests = self.requests_total.load(Ordering::Relaxed);
        let errors = self.errors_total.load(Ordering::Relaxed);
        let in_flight = self.requests_in_flight.load(Ordering::Relaxed).max(0);
        let sum_seconds = self.duration_micros_total.load(Ordering::Relaxed) as f64 / 1_000_000.0;

        let mut output = String::new();
        let _ = writeln!(
            output,
            "# HELP fastcart_http_requests_total Total HTTP requests handled."
        );
        let _ = writeln!(output, "# TYPE fastcart_http_requests_total counter");
        let _ = writeln!(output, "fastcart_http_requests_total {requests}");
        let _ = writeln!(
            output,
            "# HELP fastcart_http_errors_total Total HTTP responses with status >= 400."
        );
        let _ = writeln!(output, "# TYPE fastcart_http_errors_total counter");
        let _ = writeln!(output, "fastcart_http_errors_total {errors}");
        let _ = writeln!(
            output,
            "# HELP fastcart_http_requests_in_flight Requests currently being handled."
        );
        let _ = writeln!(output, "# TYPE fastcart_http_requests_in_flight gauge");
        let _ = writeln!(output, "fastcart_http_requests_in_flight {in_flight}");
        let _ = writeln!(
            output,
            "# HELP fastcart_storefront_data_failures_total Storefront renders that degraded because binding resolution failed."
        );
        let _ = writeln!(
            output,
            "# TYPE fastcart_storefront_data_failures_total counter"
        );
        let _ = writeln!(
            output,
            "fastcart_storefront_data_failures_total {}",
            self.storefront_data_failures_total.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            output,
            "# HELP fastcart_http_response_status_total HTTP responses by status class."
        );
        let _ = writeln!(output, "# TYPE fastcart_http_response_status_total counter");
        for (class, counter) in [
            ("2xx", &self.status_2xx),
            ("3xx", &self.status_3xx),
            ("4xx", &self.status_4xx),
            ("5xx", &self.status_5xx),
        ] {
            let _ = writeln!(
                output,
                "fastcart_http_response_status_total{{class=\"{class}\"}} {}",
                counter.load(Ordering::Relaxed)
            );
        }
        let _ = writeln!(
            output,
            "# HELP fastcart_http_request_duration_seconds HTTP request latency."
        );
        let _ = writeln!(
            output,
            "# TYPE fastcart_http_request_duration_seconds histogram"
        );
        let mut cumulative = 0_u64;
        for (index, upper) in LATENCY_BUCKETS_SECONDS.iter().enumerate() {
            cumulative =
                cumulative.saturating_add(self.bucket_counts[index].load(Ordering::Relaxed));
            let _ = writeln!(
                output,
                "fastcart_http_request_duration_seconds_bucket{{le=\"{upper}\"}} {cumulative}"
            );
        }
        let _ = writeln!(
            output,
            "fastcart_http_request_duration_seconds_bucket{{le=\"+Inf\"}} {requests}"
        );
        let _ = writeln!(
            output,
            "fastcart_http_request_duration_seconds_sum {sum_seconds:.6}"
        );
        let _ = writeln!(
            output,
            "fastcart_http_request_duration_seconds_count {requests}"
        );
        output
    }
}

/// Axum middleware recording request count, status and latency.
pub async fn metrics_middleware(
    State(metrics): State<Arc<Metrics>>,
    request: Request,
    next: Next,
) -> Response {
    let started = Instant::now();
    metrics.begin_request();
    let response = next.run(request).await;
    metrics.end_request(response.status().as_u16(), started.elapsed());
    response
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware;
    use axum::routing::get;
    use tower::ServiceExt;

    use super::{Metrics, metrics_middleware};

    #[test]
    fn renders_counters_and_cumulative_buckets() {
        let metrics = Metrics::new();
        metrics.begin_request();
        metrics.end_request(200, Duration::from_millis(4));
        metrics.end_request(201, Duration::from_millis(60));
        metrics.end_request(404, Duration::from_millis(200));
        metrics.end_request(500, Duration::from_secs(40));
        metrics.record_storefront_data_failure();

        let rendered = metrics.render();
        assert!(rendered.contains("fastcart_http_requests_total 4"));
        assert!(rendered.contains("fastcart_storefront_data_failures_total 1"));
        assert!(rendered.contains("fastcart_http_errors_total 2"));
        assert!(rendered.contains("fastcart_http_response_status_total{class=\"2xx\"} 2"));
        assert!(rendered.contains("fastcart_http_response_status_total{class=\"4xx\"} 1"));
        assert!(rendered.contains("fastcart_http_response_status_total{class=\"5xx\"} 1"));
        assert!(rendered.contains("fastcart_http_requests_in_flight 0"));
        assert!(rendered.contains("fastcart_http_request_duration_seconds_bucket{le=\"0.005\"} 1"));
        assert!(rendered.contains("fastcart_http_request_duration_seconds_bucket{le=\"0.1\"} 2"));
        assert!(rendered.contains("fastcart_http_request_duration_seconds_bucket{le=\"0.25\"} 3"));
        assert!(rendered.contains("fastcart_http_request_duration_seconds_bucket{le=\"+Inf\"} 4"));
        assert!(rendered.contains("fastcart_http_request_duration_seconds_count 4"));
    }

    #[tokio::test]
    async fn middleware_records_real_responses() {
        let metrics = Arc::new(Metrics::new());
        let app = Router::new()
            .route("/ok", get(|| async { StatusCode::OK }))
            .route("/fail", get(|| async { StatusCode::INTERNAL_SERVER_ERROR }))
            .layer(middleware::from_fn_with_state(
                metrics.clone(),
                metrics_middleware,
            ));

        let response = app
            .clone()
            .oneshot(Request::builder().uri("/ok").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = app
            .oneshot(Request::builder().uri("/fail").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let rendered = metrics.render();
        assert!(rendered.contains("fastcart_http_requests_total 2"));
        assert!(rendered.contains("fastcart_http_errors_total 1"));
        assert!(rendered.contains("fastcart_http_response_status_total{class=\"2xx\"} 1"));
        assert!(rendered.contains("fastcart_http_response_status_total{class=\"5xx\"} 1"));
    }
}
