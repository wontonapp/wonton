//! 中继表设置的行为测试：经真实隧道和本地代理读写上游的 `/_wonton/api/relays`，只经由导出给原生壳的接口驱动。

use wonton_client::{Action, HomeStatus};
use wonton_protocol::{HomeAction, RelaySettingsAction, RelaySettingsPhase};

use super::support::{Home, Phone};

fn entry(url: &str, tier: u8) -> serde_json::Value {
    serde_json::json!({
        "url": url,
        "tier": tier,
        "source": "signed",
        "state": {"state": "connected"},
        "editable": true,
    })
}

fn snapshot(entries: Vec<serde_json::Value>, n0: bool) -> String {
    serde_json::json!({
        "entries": entries,
        "n0": n0,
        "locked_by_env": false,
        "warnings": [],
    })
    .to_string()
}

fn error_body(message: &str) -> String {
    serde_json::json!({"error": {"kind": "upstream", "message": message}}).to_string()
}

fn dispatch(phone: &Phone, action: RelaySettingsAction) {
    phone
        .dispatch(Action::Home {
            action: HomeAction::RelaySettings { action },
        })
        .unwrap();
}

fn add(phone: &Phone, url: &str, tier: u8) {
    dispatch(
        phone,
        RelaySettingsAction::Add {
            url: url.into(),
            token: None,
            quic_port: None,
            tier: Some(tier),
        },
    );
}

async fn connected(phone: &Phone) -> Home {
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    home
}

#[tokio::test]
async fn refresh_loads_the_table_and_save_puts_the_whole_table_back() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = connected(&phone).await;

    *home.relay_reply.lock().unwrap() = (
        200,
        snapshot(vec![entry("https://relay.example.org/", 0)], true),
    );
    dispatch(&phone, RelaySettingsAction::Refresh);
    let view = phone
        .wait_for(|view| {
            view.relays.phase == Some(RelaySettingsPhase::Ready) && view.relays.entries.len() == 1
        })
        .await;
    assert!(view.relays.n0);
    assert_eq!(view.relays.entries[0].url, "https://relay.example.org/");
    // 直连路径：没有正在用的中继。
    assert!(view.current_relay.is_none());

    add(&phone, "https://second.example.org", 1);
    dispatch(&phone, RelaySettingsAction::SetN0 { on: false });
    let view = phone
        .wait_for(|view| view.relays.dirty && view.relays.entries.len() == 2)
        .await;
    assert_eq!(view.relays.entries[1].url, "https://second.example.org/");

    // 核心合并后回一整份新的：保存成功，表回到核心给的那一份。
    *home.relay_reply.lock().unwrap() = (
        200,
        snapshot(
            vec![
                entry("https://relay.example.org/", 0),
                entry("https://second.example.org/", 1),
            ],
            false,
        ),
    );
    dispatch(&phone, RelaySettingsAction::Save);
    let view = phone
        .wait_for(|view| view.relays.phase == Some(RelaySettingsPhase::Ready) && !view.relays.dirty)
        .await;
    assert!(!view.relays.n0);
    assert_eq!(view.relays.entries.len(), 2);

    let puts = home.relay_puts.lock().unwrap().clone();
    assert_eq!(puts.len(), 1);
    let put: serde_json::Value = serde_json::from_str(&puts[0]).unwrap();
    assert_eq!(put["n0"], false);
    assert_eq!(put["entries"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn a_failed_refresh_shows_the_reason_and_keeps_the_table() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = connected(&phone).await;

    *home.relay_reply.lock().unwrap() = (
        200,
        snapshot(vec![entry("https://relay.example.org/", 0)], false),
    );
    dispatch(&phone, RelaySettingsAction::Refresh);
    phone
        .wait_for(|view| {
            view.relays.phase == Some(RelaySettingsPhase::Ready) && !view.relays.entries.is_empty()
        })
        .await;

    *home.relay_reply.lock().unwrap() = (503, error_body("relay store is down"));
    dispatch(&phone, RelaySettingsAction::Refresh);
    let view = phone
        .wait_for(|view| matches!(view.relays.phase, Some(RelaySettingsPhase::Failed { .. })))
        .await;
    assert!(
        view.relays
            .load_problem
            .as_deref()
            .is_some_and(|problem| problem.contains("relay store is down")),
        "{:?}",
        view.relays.load_problem
    );
    assert_eq!(view.relays.entries.len(), 1);
}

/// 表还在 Loading 时一次被拒的编辑不能把在途的读作废，否则表卡在 Loading 再也不回来。
#[tokio::test]
async fn an_invalid_edit_while_loading_keeps_the_in_flight_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = connected(&phone).await;

    *home.relay_delay.lock().unwrap() = std::time::Duration::from_millis(200);
    *home.relay_reply.lock().unwrap() = (
        200,
        snapshot(vec![entry("https://relay.example.org/", 0)], true),
    );
    dispatch(&phone, RelaySettingsAction::Refresh);

    // 表还在路上：加一条地址不合法的，当场被拒。
    let rejected = phone.dispatch(Action::Home {
        action: HomeAction::RelaySettings {
            action: RelaySettingsAction::Add {
                url: "http://relay.example.org".into(),
                token: None,
                quic_port: None,
                tier: None,
            },
        },
    });
    assert!(rejected.is_err(), "{rejected:?}");

    // 在途的 GET 没被作废，回来照样落表。
    let view = phone
        .wait_for(|view| view.relays.phase == Some(RelaySettingsPhase::Ready))
        .await;
    assert_eq!(view.relays.entries.len(), 1);
    assert_eq!(view.relays.entries[0].url, "https://relay.example.org/");
    assert!(!view.relays.dirty);
}

#[tokio::test]
async fn a_failed_save_keeps_the_edits_dirty_and_shows_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = connected(&phone).await;

    *home.relay_reply.lock().unwrap() = (
        200,
        snapshot(vec![entry("https://relay.example.org/", 0)], false),
    );
    dispatch(&phone, RelaySettingsAction::Refresh);
    phone
        .wait_for(|view| {
            view.relays.phase == Some(RelaySettingsPhase::Ready) && !view.relays.entries.is_empty()
        })
        .await;
    add(&phone, "https://second.example.org", 1);

    *home.relay_reply.lock().unwrap() = (500, error_body("could not write the relay table"));
    dispatch(&phone, RelaySettingsAction::Save);
    let view = phone
        .wait_for(|view| matches!(view.relays.phase, Some(RelaySettingsPhase::Failed { .. })))
        .await;
    assert!(
        view.relays
            .save_problem
            .as_deref()
            .is_some_and(|problem| problem.contains("could not write the relay table")),
        "{:?}",
        view.relays.save_problem
    );
    assert!(view.relays.dirty);
    assert_eq!(view.relays.entries.len(), 2);
}

#[tokio::test]
async fn discard_goes_back_to_the_last_table_the_core_gave() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = connected(&phone).await;

    *home.relay_reply.lock().unwrap() = (
        200,
        snapshot(vec![entry("https://relay.example.org/", 0)], false),
    );
    dispatch(&phone, RelaySettingsAction::Refresh);
    phone
        .wait_for(|view| {
            view.relays.phase == Some(RelaySettingsPhase::Ready) && !view.relays.entries.is_empty()
        })
        .await;
    add(&phone, "https://second.example.org", 1);

    dispatch(&phone, RelaySettingsAction::Discard);
    let view = phone
        .wait_for(|view| !view.relays.dirty && view.relays.entries.len() == 1)
        .await;
    assert_eq!(view.relays.entries[0].url, "https://relay.example.org/");
    assert_eq!(view.relays.phase, Some(RelaySettingsPhase::Ready));
}

#[tokio::test]
async fn a_write_to_a_locked_table_is_kept_for_the_caller_to_fix() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = connected(&phone).await;

    let locked = serde_json::json!({
        "entries": [{
            "url": "https://fixed.example.org/",
            "tier": 0,
            "source": "env",
            "state": {"state": "unknown"},
            "editable": false,
        }],
        "n0": false,
        "locked_by_env": true,
        "warnings": ["WONTON_RELAYS is set"],
    })
    .to_string();
    *home.relay_reply.lock().unwrap() = (200, locked);
    dispatch(&phone, RelaySettingsAction::Refresh);
    let view = phone
        .wait_for(|view| view.relays.phase == Some(RelaySettingsPhase::Ready))
        .await;
    assert!(view.relays.locked_by_env);
    assert_eq!(view.relays.warnings, ["WONTON_RELAYS is set"]);

    // 整张表只读：加一条当场被拒，表不动。
    let result = phone.dispatch(Action::Home {
        action: HomeAction::RelaySettings {
            action: RelaySettingsAction::Add {
                url: "https://second.example.org".into(),
                token: None,
                quic_port: None,
                tier: None,
            },
        },
    });
    assert!(matches!(
        result,
        Err(wonton_client::CoreError::NotReady { .. })
    ));
    assert_eq!(phone.home().relays.entries.len(), 1);
}
