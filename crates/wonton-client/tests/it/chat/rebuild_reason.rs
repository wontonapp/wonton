//! 时间线重建的原因：产品在决定重建时打一条带 `reason` 的 info，这里按房间收着，断言失败时带进 panic 消息。

use std::collections::VecDeque;
use std::fmt;
use std::sync::Mutex;

use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

const TARGETS: [&str; 2] = ["wonton_client::chat::timeline", "wonton_matrix::chat_room"];
const KEEP: usize = 64;

static REASONS: Mutex<VecDeque<(String, String)>> = Mutex::new(VecDeque::new());

pub struct Capture;

impl<S: Subscriber> Layer<S> for Capture {
    fn enabled(&self, metadata: &tracing::Metadata<'_>, _: Context<'_, S>) -> bool {
        TARGETS.contains(&metadata.target())
    }

    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        let (Some(room_id), Some(reason)) = (fields.room_id, fields.reason) else {
            return;
        };
        let mut reason = reason;
        if let Some(id) = &fields.event_id {
            reason.push_str(&format!("，事件 {id}"));
        }
        if let Some(absent) = &fields.absent {
            reason.push_str(&format!("，已缺席 {absent}"));
        }
        let mut reasons = REASONS.lock().expect("重建原因");
        reasons.push_back((room_id, reason));
        if reasons.len() > KEEP {
            reasons.pop_front();
        }
    }
}

#[derive(Default)]
struct Fields {
    room_id: Option<String>,
    reason: Option<String>,
    event_id: Option<String>,
    absent: Option<String>,
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record(field, format!("{value:?}"));
    }
}

impl Fields {
    fn record(&mut self, field: &Field, value: String) {
        match field.name() {
            "room_id" => self.room_id = Some(value),
            "reason" => self.reason = Some(value),
            "event_id" => self.event_id = Some(value),
            "absent" => self.absent = Some(value),
            _ => {}
        }
    }
}

/// 取出这个房间最近一条还没被取走的重建原因。
fn take(room_id: &str) -> Option<String> {
    let mut reasons = REASONS.lock().expect("重建原因");
    let index = reasons.iter().rposition(|(room, _)| room == room_id)?;
    reasons.remove(index).map(|(_, reason)| reason)
}

/// 取出这个房间最近一条还没被取走的清空原因，没记录时为“未记录”。
pub fn take_rebuild_reason(room_id: &str) -> String {
    take(room_id).unwrap_or_else(|| "未记录".to_owned())
}
