//! JSONL formatting layer used by `init` to persist events to disk.

use time::{OffsetDateTime, UtcOffset};
use tracing_appender::non_blocking::NonBlocking;
use tracing_subscriber::fmt;

struct LocalRfc3339;

impl fmt::time::FormatTime for LocalRfc3339 {
    fn format_time(&self, writer: &mut fmt::format::Writer<'_>) -> std::fmt::Result {
        let now = OffsetDateTime::now_utc();
        let offset = UtcOffset::local_offset_at(now).unwrap_or(UtcOffset::UTC);
        write_log_timestamp(writer, now.to_offset(offset))
    }
}

/// Preserve the timestamp bytes written by the previous JSONL timer.
fn write_log_timestamp(writer: &mut impl std::fmt::Write, at: OffsetDateTime) -> std::fmt::Result {
    write!(
        writer,
        "{}T{:02}:{:02}:{:02}",
        at.date(),
        at.hour(),
        at.minute(),
        at.second()
    )?;

    let nanos = at.nanosecond();
    if nanos != 0 {
        if nanos.is_multiple_of(1_000_000) {
            write!(writer, ".{:03}", nanos / 1_000_000)?;
        } else if nanos.is_multiple_of(1_000) {
            write!(writer, ".{:06}", nanos / 1_000)?;
        } else {
            write!(writer, ".{nanos:09}")?;
        }
    }

    let offset_seconds = at.offset().whole_seconds();
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let minutes = (offset_seconds.unsigned_abs() + 30) / 60;
    write!(writer, "{sign}{:02}:{:02}", minutes / 60, minutes % 60)
}

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
        .with_timer(LocalRfc3339)
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
            .with_timer(LocalRfc3339)
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

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    #[test]
    fn log_timestamp_preserves_legacy_offset_and_subsecond_precision() {
        for (instant, expected) in [
            (
                datetime!(2026-11-17 18:43:12 UTC),
                "2026-11-17T18:43:12+00:00",
            ),
            (
                datetime!(2026-11-17 18:43:12.007 +09:00),
                "2026-11-17T18:43:12.007+09:00",
            ),
            (
                datetime!(2026-11-17 18:43:12.007980 -03:30),
                "2026-11-17T18:43:12.007980-03:30",
            ),
            (
                datetime!(2026-11-17 18:43:12.007987654 +05:45:31),
                "2026-11-17T18:43:12.007987654+05:46",
            ),
        ] {
            let mut output = String::new();
            write_log_timestamp(&mut output, instant).unwrap();
            assert_eq!(output, expected);
        }
    }
}
