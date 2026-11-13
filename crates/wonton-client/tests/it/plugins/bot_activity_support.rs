use std::sync::Mutex;
use std::time::Duration;

use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use tokio::sync::watch;
use wonton_protocol::BotActivityReport;

use super::{Canned, CoreState, core_error};

#[derive(Default)]
pub(super) struct Activity(Mutex<Option<watch::Sender<Canned>>>);

impl CoreState {
    pub fn set_bot_activity(&self, report: BotActivityReport) {
        self.publish_bot_activity(Canned(200, serde_json::to_string(&report).unwrap()));
    }

    pub fn fail_bot_activity(&self) {
        self.publish_bot_activity(core_error(503, "upstream", "Activity unavailable"));
    }

    fn publish_bot_activity(&self, reply: Canned) {
        let mut activity = self.bot_activity.0.lock().unwrap();
        if let Some(activity) = activity.as_ref() {
            activity.send_replace(reply);
        } else {
            *activity = Some(watch::Sender::new(reply));
        }
    }
}

pub(super) async fn respond(core: &CoreState, method: &Method, uri: &Uri) -> Option<Response> {
    if method != Method::GET || uri.path() != "/_wonton/api/bots/activity" {
        return None;
    }
    let mut activity = core.bot_activity.0.lock().unwrap().as_ref()?.subscribe();
    let after = uri.query().and_then(|query| {
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key == "after")
            .and_then(|(_, value)| value.parse::<u64>().ok())
    });
    let current = activity.borrow_and_update().clone();
    let rev = serde_json::from_str::<BotActivityReport>(&current.1)
        .ok()
        .map(|report| report.rev);
    if current.0 == 200 && after.is_some() && after == rev {
        tokio::select! {
            result = activity.changed() => { result.ok()?; },
            () = tokio::time::sleep(Duration::from_secs(25)) => {},
        }
    }
    let Canned(status, body) = activity.borrow_and_update().clone();
    Some((StatusCode::from_u16(status).unwrap(), body).into_response())
}
