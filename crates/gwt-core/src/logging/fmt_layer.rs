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

/// Preserve tracing's JSON shape while selecting one registered sink per record.
pub(crate) fn build_routed<S>(router: super::ProjectLogRouter) -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fmt::layer()
        .fmt_fields(fmt::format::JsonFields::new())
        .event_format(RoutedJson { router })
        .with_writer(std::io::sink)
}

struct RoutedJson {
    router: super::ProjectLogRouter,
}

impl<S> fmt::FormatEvent<S, fmt::format::JsonFields> for RoutedJson
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn format_event(
        &self,
        ctx: &fmt::FmtContext<'_, S, fmt::format::JsonFields>,
        _writer: fmt::format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let mut line = String::new();
        fmt::format()
            .json()
            .with_current_span(true)
            .with_span_list(false)
            .with_timer(ChronoLocal::rfc_3339())
            .format_event(ctx, fmt::format::Writer::new(&mut line), event)?;
        let scope = self.router.resolve(event, ctx.event_scope());
        if let Some(token) = &scope {
            // Add resolved provenance without changing the standard fields/span shape.
            let mut json: serde_json::Value =
                serde_json::from_str(&line).map_err(|_| std::fmt::Error)?;
            json["project_scope"] = token.clone().into();
            line = serde_json::to_string(&json).map_err(|_| std::fmt::Error)?;
            line.push('\n');
        }
        self.router
            .write(scope.as_deref(), line.as_bytes())
            .map_err(|_| std::fmt::Error)
    }
}
