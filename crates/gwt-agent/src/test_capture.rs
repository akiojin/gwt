//! Shared tracing capture helpers for the gwt-agent test modules.
//!
//! Tests that want to assert on emitted `tracing` events install a
//! [`CaptureLayer`] over the registry and inspect the snapshot afterwards.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tracing::{field::Visit, Event, Level, Subscriber};
use tracing_subscriber::{
    layer::{Context, Layer},
    registry::LookupSpan,
};

#[derive(Clone, Debug)]
pub struct CapturedEvent {
    pub level: Level,
    pub target: String,
    pub fields: HashMap<String, String>,
}

#[derive(Clone, Default)]
pub struct CapturedEvents(Arc<Mutex<Vec<CapturedEvent>>>);

impl CapturedEvents {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Vec<CapturedEvent> {
        self.0.lock().expect("captured events").clone()
    }
}

pub struct CaptureLayer {
    events: CapturedEvents,
}

impl CaptureLayer {
    pub fn new(events: CapturedEvents) -> Self {
        Self { events }
    }
}

struct CaptureVisitor<'a>(&'a mut CapturedEvent);

impl<'a> Visit for CaptureVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .fields
            .insert(field.name().to_string(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0
            .fields
            .insert(field.name().to_string(), value.to_string());
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0
            .fields
            .insert(field.name().to_string(), value.to_string());
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0
            .fields
            .insert(field.name().to_string(), value.to_string());
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0
            .fields
            .insert(field.name().to_string(), value.to_string());
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut captured = CapturedEvent {
            level: *event.metadata().level(),
            target: event.metadata().target().to_string(),
            fields: HashMap::new(),
        };
        event.record(&mut CaptureVisitor(&mut captured));
        self.events
            .0
            .lock()
            .expect("captured events")
            .push(captured);
    }
}
