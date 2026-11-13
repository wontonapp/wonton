use matrix_sdk::ruma::user_id;
use serde_json::json;

use super::*;

const CALL_MEMBER: &str = "org.matrix.msc3401.call.member";

fn requested(capabilities: &[&str]) -> Capabilities {
    serde_json::from_value(json!(capabilities)).unwrap()
}

fn granted(asked: &[&str], declared: &[&str]) -> Vec<String> {
    let declared: Vec<String> = declared.iter().map(|d| (*d).to_owned()).collect();
    let mut granted = strings(&grant(
        &requested(asked),
        &declared,
        user_id!("@me:home.example"),
    ));
    granted.sort();
    granted
}

fn send_state(key: Option<&str>) -> String {
    match key {
        Some(key) => format!("{SEND_STATE}:{CALL_MEMBER}#{key}"),
        None => format!("{SEND_STATE}:{CALL_MEMBER}"),
    }
}

/// 行为：页面只拿到请求 ∩ 声明，清单外的、SDK 不认识的都不给。
#[test]
fn only_requested_and_declared_capabilities_are_granted() {
    let transports = "org.matrix.msc4515.rtc_transports";
    let delayed = "org.matrix.msc4157.send.delayed_event";
    let messages = "org.matrix.msc2762.receive.event:m.room.message";
    let keys = "org.matrix.msc3819.send.to_device:io.element.call.encryption_keys";
    assert_eq!(
        granted(
            &[transports, delayed, messages, keys, "m.always_on_screen"],
            &[transports, keys, "m.always_on_screen"],
        ),
        vec![keys.to_owned(), transports.to_owned()]
    );
}

/// 行为：发状态事件不限 `state_key` 时收窄到自己；具体的只认自己的三种写法，别人的不给。
#[test]
fn state_keys_are_narrowed_to_the_own_user() {
    let declared = [send_state(None)];
    let declared: Vec<&str> = declared.iter().map(String::as_str).collect();
    let mine = [
        "@me:home.example",
        "@me:home.example_DEVICE",
        "_@me:home.example_DEVICE_m.call",
    ];
    let others = [
        "@other:home.example",
        "_@other:home.example_DEVICE",
        "@me:home.example.evil",
        "",
    ];
    let mut asked: Vec<String> = vec![send_state(None)];
    asked.extend(mine.iter().chain(&others).map(|k| send_state(Some(k))));
    let asked: Vec<&str> = asked.iter().map(String::as_str).collect();
    let mut expected: Vec<String> = mine.iter().map(|k| send_state(Some(k))).collect();
    expected.sort();
    expected.dedup();
    assert_eq!(granted(&asked, &declared), expected);
}

/// 行为：清单只声明了具体的 `state_key` 时，不限的请求收窄到声明的那几条，仍然只留自己的。
#[test]
fn a_declared_state_key_narrows_an_open_request() {
    let declared = [
        send_state(Some("@me:home.example")),
        send_state(Some("@other:home.example")),
    ];
    let declared: Vec<&str> = declared.iter().map(String::as_str).collect();
    assert_eq!(
        granted(&[&send_state(None)], &declared),
        vec![send_state(Some("@me:home.example"))]
    );
    assert!(granted(&[&send_state(Some("@me:home.example_D"))], &declared).is_empty());
}

/// 行为：读状态事件不收窄 `state_key`，清单不限就照请求给。
#[test]
fn reading_state_is_not_narrowed() {
    let read = format!("org.matrix.msc2762.receive.state_event:{CALL_MEMBER}");
    assert_eq!(granted(&[&read], &[&read]), vec![read.clone()]);
}

/// 行为：清单写 `m.call.member`、页面也这么请求时，SDK 两边都按 `org.matrix.msc3401.call.member` 算，照样批。
#[test]
fn the_declared_event_type_is_read_the_way_the_sdk_reads_it() {
    let short = "org.matrix.msc2762.receive.state_event:m.call.member";
    assert_eq!(
        granted(&[short], &[short]),
        vec![format!(
            "org.matrix.msc2762.receive.state_event:{CALL_MEMBER}"
        )]
    );
}
