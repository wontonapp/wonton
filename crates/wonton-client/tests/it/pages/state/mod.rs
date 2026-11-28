mod layout_reconcile;
mod my_apps_persistence;
mod pins;
mod readiness;
mod reload_recovery;
mod urls;

use axum::http::Method;
use wonton_client::{BOT_PANEL_PINS_PATH, PagesFailure};

use crate::plugins::rig::{Canned, Headered};

use super::*;

fn pin_response(id: &str) -> String {
    serde_json::json!({
        "pins": [{
            "id": id,
            "scope": {"type": "main"},
            "tab": {"type": "url", "url": format!("https://example.com/{id}")},
            "title": id,
            "attention": false,
            "plugin_id": "app.example.agent",
            "bot": "agent",
            "name": "Agent"
        }],
        "consents": []
    })
    .to_string()
}
