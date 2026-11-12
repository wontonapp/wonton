use serde_json::json;

use super::*;
use crate::{MessageSummary, Rgb};

fn colors(base: u8) -> ThemeColors {
    let rgb = |offset: u8| Rgb {
        red: base,
        green: offset,
        blue: 0,
    };
    ThemeColors {
        canvas: rgb(1),
        surface: rgb(2),
        surface_subtle: rgb(3),
        text_primary: rgb(4),
        text_secondary: rgb(5),
        separator: rgb(6),
        primary: rgb(7),
        on_primary: rgb(8),
        success: rgb(9),
        warning: rgb(10),
        danger: rgb(11),
        accent_fill: rgb(12),
        accent_ink: rgb(13),
        accent_ink_on_fill: rgb(14),
        text_on_accent_fill: rgb(15),
        accent_fill_border: None,
    }
}

fn colors_json(base: u8) -> serde_json::Value {
    let rgb = |offset: u8| json!({ "red": base, "green": offset, "blue": 0 });
    json!({
        "canvas": rgb(1),
        "surface": rgb(2),
        "surface_subtle": rgb(3),
        "text_primary": rgb(4),
        "text_secondary": rgb(5),
        "separator": rgb(6),
        "primary": rgb(7),
        "on_primary": rgb(8),
        "success": rgb(9),
        "warning": rgb(10),
        "danger": rgb(11),
        "accent_fill": rgb(12),
        "accent_ink": rgb(13),
        "accent_ink_on_fill": rgb(14),
        "text_on_accent_fill": rgb(15),
        "accent_fill_border": null,
    })
}

fn snapshot() -> WidgetSnapshot {
    WidgetSnapshot {
        version: WIDGET_SNAPSHOT_VERSION,
        updated_at_ms: 1_700_000_000_000,
        account: WidgetAccount::SignedIn,
        unread_total: 5,
        rooms: vec![WidgetRoom {
            room_id: "!family:home.example".into(),
            name: "家里".into(),
            is_direct: false,
            unread_count: 2,
            preview: Some(RoomPreview {
                sender_name: "妈妈".into(),
                is_own: false,
                body: MessageSummary::Text {
                    text: "晚饭好了".into(),
                },
                timestamp_ms: 1_699_999_990_000,
            }),
        }],
        default_agent: Some(WidgetAgent {
            user_id: "@hermes:home.example".into(),
            name: "Hermes".into(),
            activity: Some(BotActivity::Waiting),
            unread_count: 3,
        }),
        contacts: vec![WidgetContact {
            user_id: "@hermes:home.example".into(),
            name: "Hermes".into(),
            is_bot: true,
            room_id: "!hermes:home.example".into(),
        }],
        plugins: vec![WidgetPlugin {
            plugin_id: "immich".into(),
            title: "Immich".into(),
        }],
        light: colors(1),
        dark: colors(2),
    }
}

/// 契约：iOS 的小组件扩展按这些字段名解 JSON，名字改了它就读不出来。
#[test]
fn snapshot_field_names_are_the_contract() {
    let expected = json!({
        "version": 1,
        "updated_at_ms": 1_700_000_000_000_u64,
        "account": "signed_in",
        "unread_total": 5,
        "rooms": [{
            "room_id": "!family:home.example",
            "name": "家里",
            "is_direct": false,
            "unread_count": 2,
            "preview": {
                "sender_name": "妈妈",
                "is_own": false,
                "body": { "type": "text", "text": "晚饭好了" },
                "timestamp_ms": 1_699_999_990_000_u64,
            },
        }],
        "default_agent": {
            "user_id": "@hermes:home.example",
            "name": "Hermes",
            "activity": "waiting",
            "unread_count": 3,
        },
        "contacts": [{
            "user_id": "@hermes:home.example",
            "name": "Hermes",
            "is_bot": true,
            "room_id": "!hermes:home.example",
        }],
        "plugins": [{ "plugin_id": "immich", "title": "Immich" }],
        "light": colors_json(1),
        "dark": colors_json(2),
    });
    assert_eq!(serde_json::to_value(snapshot()).unwrap(), expected);
}

#[test]
fn every_account_state_has_a_fixed_name() {
    for (account, name) in [
        (WidgetAccount::NotPaired, "not_paired"),
        (WidgetAccount::SignedOut, "signed_out"),
        (WidgetAccount::Switching, "switching"),
        (WidgetAccount::SignedIn, "signed_in"),
    ] {
        assert_eq!(serde_json::to_value(account).unwrap(), json!(name));
    }
}

#[test]
fn a_written_snapshot_reads_back() {
    let bytes = serde_json::to_vec(&snapshot()).unwrap();
    assert_eq!(
        WidgetSnapshotRead::from_json(Some(&bytes)),
        WidgetSnapshotRead::Ready {
            snapshot: snapshot()
        }
    );
}

#[test]
fn a_missing_file_is_not_a_broken_one() {
    assert_eq!(
        WidgetSnapshotRead::from_json(None),
        WidgetSnapshotRead::Missing
    );
}

#[test]
fn other_versions_and_broken_files_are_unreadable() {
    let mut newer = serde_json::to_value(snapshot()).unwrap();
    newer["version"] = json!(WIDGET_SNAPSHOT_VERSION + 1);
    // 新版本改了别的字段也只按版本拒绝。
    newer["rooms"] = json!("changed");
    let newer = serde_json::to_vec(&newer).unwrap();
    for bytes in [&newer[..], b"{", b"{\"version\":1}"] {
        assert!(
            matches!(
                WidgetSnapshotRead::from_json(Some(bytes)),
                WidgetSnapshotRead::Unreadable { .. }
            ),
            "{}",
            String::from_utf8_lossy(bytes)
        );
    }
}
