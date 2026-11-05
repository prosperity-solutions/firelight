//! Firelight's metrics (§17.2), named `firelight.*`. Without an OTLP endpoint
//! the global meter is a no-op, so recording costs nothing.

use std::sync::OnceLock;

use opentelemetry::metrics::{Counter, Gauge, Histogram};

pub struct Metrics {
    /// Seconds per source pull.
    pub pull_duration: Histogram<f64>,
    pub pull_errors: Counter<u64>,
    /// Seconds to embed what a re-index left missing.
    pub index_duration: Histogram<f64>,
    pub chunks_pending: Gauge<u64>,
    /// Seconds per search.
    pub search_duration: Histogram<f64>,
    pub search_results: Histogram<u64>,
    /// Searches that found nothing: the "content gaps" signal.
    pub search_zero_results: Counter<u64>,
    /// MCP tool calls, by `tool`.
    pub mcp_calls: Counter<u64>,
    pub golden_failures: Counter<u64>,
}

/// The instruments, made on first use (after telemetry is set up).
pub fn get() -> &'static Metrics {
    static M: OnceLock<Metrics> = OnceLock::new();
    M.get_or_init(|| {
        let m = opentelemetry::global::meter("firelight");
        Metrics {
            pull_duration: m
                .f64_histogram("firelight.pull.duration")
                .with_unit("s")
                .build(),
            pull_errors: m.u64_counter("firelight.pull.errors").build(),
            index_duration: m
                .f64_histogram("firelight.index.duration")
                .with_unit("s")
                .build(),
            chunks_pending: m.u64_gauge("firelight.chunks.pending").build(),
            search_duration: m
                .f64_histogram("firelight.search.duration")
                .with_unit("s")
                .build(),
            search_results: m.u64_histogram("firelight.search.results").build(),
            search_zero_results: m.u64_counter("firelight.search.zero_results").build(),
            mcp_calls: m.u64_counter("firelight.mcp.calls").build(),
            golden_failures: m.u64_counter("firelight.golden.failures").build(),
        }
    })
}
