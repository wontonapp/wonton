//! 新会话切到配置的工作模式。ACP 里模式是可选的：一种写法是 `session/new` 应答的 `modes`，用 `session/set_mode`
//! 切；另一种是 `configOptions` 里类别为 `mode` 的一项，用 `session/set_config_option` 切（dsh 0.2.2 只有这种，
//! 对 `session/set_mode` 回 Method not found）。两样都没有的 Agent 不分模式，跳过；Agent 报了模式却不认配置的值时，
//! 照样发请求，由 Agent 回错，桥不替它判断。

use serde_json::{Value, json};

/// 要发给 Agent 的切模式请求；`None` 是 Agent 不分模式。
pub(super) fn request(
    session: &Value,
    session_id: &str,
    mode: &str,
) -> Option<(&'static str, Value)> {
    if session.get("modes").is_some_and(|modes| !modes.is_null()) {
        return Some((
            "session/set_mode",
            json!({"sessionId": session_id, "modeId": mode}),
        ));
    }
    let option = session
        .get("configOptions")?
        .as_array()?
        .iter()
        .find(|option| option["category"] == "mode")?;
    let config_id = option["id"].as_str()?;
    Some((
        "session/set_config_option",
        json!({"sessionId": session_id, "configId": config_id, "value": mode}),
    ))
}

/// 话题想要“改前先问”的模式请求（ADR-0058）：Agent 自己报的出这个取值才切，否则返回
/// `None`，由调用方退回配置的默认模式并在话题里如实说明。
pub(super) fn topic_request(
    session: &Value,
    session_id: &str,
    mode: &str,
) -> Option<(&'static str, Value)> {
    if !offers(session, mode) {
        return None;
    }
    request(session, session_id, mode)
}

/// Agent 报出的模式取值里有没有 `mode`。`modes.availableModes` 和类别为 `mode` 的配置项
/// 的 `options[].value` 是两种写法；两样都没有可选值表时不认。
fn offers(session: &Value, mode: &str) -> bool {
    let listed = session["modes"]["availableModes"]
        .as_array()
        .map(|modes| {
            modes
                .iter()
                .any(|available| available["id"].as_str() == Some(mode))
        })
        .unwrap_or(false);
    if listed {
        return true;
    }
    session
        .get("configOptions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|option| option["category"] == "mode")
        .filter_map(|option| option["options"].as_array())
        .flatten()
        .any(|value| value["value"].as_str() == Some(mode))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_use_set_mode() {
        let session =
            json!({"sessionId": "s", "modes": {"currentModeId": "a", "availableModes": []}});
        let (method, params) = request(&session, "s", "plan").unwrap();
        assert_eq!(method, "session/set_mode");
        assert_eq!(params["modeId"], "plan");
    }

    #[test]
    fn mode_config_option_uses_set_config_option() {
        let session = json!({"sessionId": "s", "configOptions": [
            {"id": "model", "category": "model", "type": "select"},
            {"id": "preset", "category": "mode", "type": "select"}
        ]});
        let (method, params) = request(&session, "s", "workspace-write").unwrap();
        assert_eq!(method, "session/set_config_option");
        assert_eq!(params["configId"], "preset");
        assert_eq!(params["value"], "workspace-write");
    }

    #[test]
    fn agents_without_modes_are_left_alone() {
        assert!(request(&json!({"sessionId": "s"}), "s", "plan").is_none());
        let only_model = json!({"configOptions": [{"id": "model", "category": "model"}]});
        assert!(request(&only_model, "s", "plan").is_none());
    }
}
