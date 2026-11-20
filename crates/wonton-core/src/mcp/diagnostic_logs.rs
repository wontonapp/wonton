//! rmcp keeps raw SSE IDs for resumption and logs them from independently spawned tasks.
//! Intercept only those diagnostic fields; protocol IDs never pass through this layer.

use std::borrow::Cow;
use std::fmt;
use std::sync::Mutex;

use tracing::field::{DisplayValue, Field, Value, Visit, display};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

use crate::plugin::remote_redaction::{Redaction, WeakRedaction};

static LIVE: Mutex<Vec<WeakRedaction>> = Mutex::new(Vec::new());

pub(super) fn register(redaction: &Redaction) {
    let mut live = LIVE.lock().expect("MCP diagnostic redactions poisoned");
    live.retain(|entry| entry.upgrade().is_some());
    live.push(redaction.downgrade());
}

/// Keep SDK event IDs out of every downstream core log sink without altering SSE resumption.
#[derive(Clone, Copy)]
pub struct SdkDiagnostics;

impl<S: Subscriber> Layer<S> for SdkDiagnostics {
    fn event_enabled(&self, event: &Event<'_>, context: Context<'_, S>) -> bool {
        if event.metadata().target() != "rmcp::transport::common::client_side_sse"
            || !event.fields().any(|field| is_event_id(&field))
        {
            return true;
        }
        let redactions: Vec<_> = LIVE
            .lock()
            .expect("MCP diagnostic redactions poisoned")
            .iter()
            .filter_map(WeakRedaction::upgrade)
            .collect();
        if redactions.is_empty() {
            return true;
        }
        let mut fields = Fields {
            redactions: &redactions,
            values: std::iter::repeat_with(|| None)
                .take(event.metadata().fields().len())
                .collect(),
            changed: false,
        };
        event.record(&mut fields);
        if !fields.changed {
            return true;
        }
        let values: Vec<_> = fields
            .values
            .iter()
            .map(|value| value.as_ref().map(|value| value as &dyn Value))
            .collect();
        let values = event.metadata().fields().value_set_all(&values);
        let redacted = if event.is_contextual() {
            Event::new(event.metadata(), &values)
        } else {
            Event::new_child_of(event.parent().cloned(), event.metadata(), &values)
        };
        // Forward exactly one event with the original metadata/parent. Only the
        // original credential-bearing copy is withheld from downstream sinks.
        context.event(&redacted);
        false
    }
}

fn is_event_id(field: &Field) -> bool {
    matches!(field.name(), "id" | "last_event_id")
}

struct Fields<'a> {
    redactions: &'a [Redaction],
    values: Vec<Option<DisplayValue<String>>>,
    changed: bool,
}

impl Visit for Fields<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let mut text = format!("{value:?}");
        if is_event_id(field)
            && let Cow::Owned(redacted) = Redaction::text_all(&text, self.redactions)
        {
            text = redacted;
            self.changed = true;
        }
        self.values[field.index()] = Some(display(text));
    }
}
