//! JSONL formatting layer used by `init` to persist events to disk.

use tracing_appender::non_blocking::NonBlocking;
use tracing_subscriber::fmt::{self, time::ChronoLocal};

/// Build the JSONL fmt layer that writes structured events to the
/// non-blocking appender.
///
/// Each event is serialised as a single JSON object per line with:
/// `timestamp` (RFC3339 local), `level`, `target`, `message`, and any
/// structured fields captured from the `tracing` call site.
pub fn build<S>(writer: NonBlocking) -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fmt::layer()
        .json()
        .with_current_span(true)
        .with_span_list(false)
        .with_target(true)
        .with_file(false)
        .with_line_number(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_timer(ChronoLocal::rfc_3339())
        .with_writer(writer)
}
