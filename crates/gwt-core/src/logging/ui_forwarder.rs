//! `tracing_subscriber::Layer` that forwards events to the UI thread.
//!
//! For every `tracing::Event` observed at or above `Info`, this layer
//! builds a `LogEvent` and sends it over an `UnboundedSender<LogEvent>`.
//! The TUI consumes the receiver and drives toasts / error modal from
//! the events. `Debug` events are silently skipped — they are
//! important for the file log but not for the UI.
//!
//! The sender is unbounded on purpose (FR-015, NFR-005): `tracing`
//! calls must never block background threads (git, docker, index worker)
//! even during bursts. The file writer is the authoritative record; the
//! UI forwarder only drives ephemeral surfaces.

use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;
use tracing::{field::Visit, Subscriber};
use tracing_subscriber::{layer::Context, Layer};

use super::{LogEvent, LogLevel};

pub type UiEventSender = Arc<UnboundedSender<LogEvent>>;

/// Layer that forwards tracing events into an `UnboundedSender<LogEvent>`.
pub struct UiForwarderLayer {
    sender: UiEventSender,
}

impl UiForwarderLayer {
    pub fn new(sender: UnboundedSender<LogEvent>) -> Self {
        Self {
            sender: Arc::new(sender),
        }
    }
}

impl<S> Layer<S> for UiForwarderLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let severity = LogLevel::from_tracing(*event.metadata().level());
        if severity == LogLevel::Debug {
            // Debug events are only persisted to the file; they are not
            // shown on any UI surface.
            return;
        }

        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);
        let EventVisitor {
            message,
            detail,
            fields,
        } = visitor;

        // Tracing allows events without an explicit `message = "..."`.
        // Use an empty string rather than skipping so the UI at least
        // shows a bubble that can be expanded via the Logs tab.
        let message = message.unwrap_or_default();
        let source = event.metadata().target().to_string();

        let mut log_event = LogEvent::new(severity, source, message);
        if let Some(detail) = detail {
            log_event = log_event.with_detail(detail);
        }
        log_event.fields = fields;

        // Ignore send errors: during shutdown the receiver may have been
        // dropped and we do not want `tracing::error!` in a `Drop` to
        // panic.
        let _ = self.sender.send(log_event);
    }
}

#[derive(Default)]
struct EventVisitor {
    message: Option<String>,
    detail: Option<String>,
    fields: serde_json::Map<String, serde_json::Value>,
}

impl Visit for EventVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let rendered = format!("{value:?}");
        match field.name() {
            "message" => self.message = Some(strip_quotes(rendered)),
            "detail" => self.detail = Some(strip_quotes(rendered)),
            name => {
                self.fields
                    .insert(name.to_string(), serde_json::Value::String(rendered));
            }
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "message" => self.message = Some(value.to_string()),
            "detail" => self.detail = Some(value.to_string()),
            name => {
                self.fields.insert(
                    name.to_string(),
                    serde_json::Value::String(value.to_string()),
                );
            }
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.insert(
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        );
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.insert(
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        );
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields
            .insert(field.name().to_string(), serde_json::Value::Bool(value));
    }
}

/// `record_debug` formats strings with surrounding quotes. Strip them so
/// UI messages do not render as `"hello"` instead of `hello`.
fn strip_quotes(s: String) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_quotes_removes_wrapping_quotes() {
        assert_eq!(strip_quotes("\"hello\"".into()), "hello");
        assert_eq!(strip_quotes("plain".into()), "plain");
        assert_eq!(strip_quotes("\"".into()), "\""); // single quote untouched
    }
}
