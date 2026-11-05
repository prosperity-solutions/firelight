//! Logs, traces and metrics (§17.2).
//!
//! Logs go to stdout (JSON or pretty). Traces and metrics are exported over
//! OTLP/HTTP only when the standard `OTEL_EXPORTER_OTLP_ENDPOINT` (or a
//! signal-specific `…_TRACES_ENDPOINT` / `…_METRICS_ENDPOINT`) is set; every
//! other standard `OTEL_*` variable (headers, protocol, service name,
//! resource attributes, sampling) is honoured by the SDK. Metric names start
//! with `firelight.` (see firelight_server::metrics).

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// Flushes and stops the exporters when dropped (graceful shutdown).
#[derive(Default)]
pub struct Guard {
    traces: Option<SdkTracerProvider>,
    metrics: Option<SdkMeterProvider>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(t) = self.traces.take() {
            let _ = t.shutdown();
        }
        if let Some(m) = self.metrics.take() {
            let _ = m.shutdown();
        }
    }
}

fn set(var: &str) -> bool {
    std::env::var(var).is_ok_and(|v| !v.trim().is_empty())
}

/// Is OTLP export on, for traces and for metrics?
pub fn wanted() -> (bool, bool) {
    if std::env::var("OTEL_SDK_DISABLED").is_ok_and(|v| v.trim().eq_ignore_ascii_case("true")) {
        return (false, false);
    }
    let any = set("OTEL_EXPORTER_OTLP_ENDPOINT");
    (
        any || set("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"),
        any || set("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT"),
    )
}

fn resource() -> Resource {
    let mut b = Resource::builder();
    if !set("OTEL_SERVICE_NAME") {
        b = b.with_service_name("firelight");
    }
    b.with_attribute(opentelemetry::KeyValue::new(
        "service.version",
        crate::VERSION,
    ))
    .build()
}

/// Logging always; traces and metrics when configured. Keep the guard for the
/// life of the process.
pub fn init(level: &str, format: &str) -> Guard {
    let filter =
        tracing_subscriber::EnvFilter::new(format!("{level},hyper=warn,opentelemetry=warn"));
    let fmt = if format == "pretty" {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stdout)
            .boxed()
    } else {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stdout)
            .json()
            .flatten_event(true)
            .with_current_span(false)
            .boxed()
    };
    let (want_traces, want_metrics) = wanted();
    let mut guard = Guard::default();
    // The OTLP client is reqwest with rustls; Firelight's TLS provider is ring.
    let _ = rustls_ring();
    let mut problems = Vec::new();
    let otel_layer = if want_traces {
        match opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .build()
        {
            Ok(exporter) => {
                let provider = SdkTracerProvider::builder()
                    .with_batch_exporter(exporter)
                    .with_resource(resource())
                    .build();
                let tracer = provider.tracer("firelight");
                opentelemetry::global::set_tracer_provider(provider.clone());
                guard.traces = Some(provider);
                Some(tracing_opentelemetry::layer().with_tracer(tracer))
            }
            Err(e) => {
                problems.push(format!("traces: {e}"));
                None
            }
        }
    } else {
        None
    };
    if want_metrics {
        match opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .build()
        {
            Ok(exporter) => {
                let provider = SdkMeterProvider::builder()
                    .with_periodic_exporter(exporter)
                    .with_resource(resource())
                    .build();
                opentelemetry::global::set_meter_provider(provider.clone());
                guard.metrics = Some(provider);
            }
            Err(e) => problems.push(format!("metrics: {e}")),
        }
    }
    // Logs follow the log level; traces have their own filter, so a quiet
    // log level doesn't empty them and debug logs (rmcp prints whole MCP
    // requests, query text included) never become span events.
    let otel_filter = tracing_subscriber::EnvFilter::new(
        "info,rmcp=warn,hyper=warn,h2=warn,reqwest=warn,opentelemetry=warn",
    );
    tracing_subscriber::registry()
        .with(fmt.with_filter(filter))
        .with(otel_layer.map(|l| l.with_filter(otel_filter)))
        .init();
    if std::env::var("OTEL_EXPORTER_OTLP_PROTOCOL")
        .or_else(|_| std::env::var("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL"))
        .is_ok_and(|p| p.trim() == "grpc")
    {
        tracing::warn!(
            "OTEL_EXPORTER_OTLP_PROTOCOL=grpc isn't supported; Firelight exports OTLP over HTTP (http/protobuf). Point the endpoint at the collector's HTTP port (4318)"
        );
    }
    for p in problems {
        tracing::warn!("OpenTelemetry export is off: {p}");
    }
    if guard.traces.is_some() || guard.metrics.is_some() {
        tracing::info!(
            traces = guard.traces.is_some(),
            metrics = guard.metrics.is_some(),
            "OpenTelemetry export on"
        );
    }
    guard
}

fn rustls_ring() -> bool {
    // Same as firelight_source::http_client: reqwest is built without a
    // default provider, so install ring once before any client exists.
    let _ = firelight_source::http_client();
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn off_without_an_endpoint() {
        // The test environment sets no OTEL_* endpoint.
        if std::env::var_os("OTEL_EXPORTER_OTLP_ENDPOINT").is_none()
            && std::env::var_os("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT").is_none()
            && std::env::var_os("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT").is_none()
        {
            assert_eq!(super::wanted(), (false, false));
        }
    }
}
