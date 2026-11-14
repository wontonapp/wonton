use serde_json::{Value, json};

#[derive(Default)]
pub(super) struct FakeFriends {
    pub(super) friends: Vec<Value>,
    pub(super) removed: Vec<Value>,
    pub(super) blocked: Vec<String>,
    pub(super) calls: Vec<(String, String)>,
}

/// 好友接口的假实现：只够客户端行为测试用，记账在 `FakeFriends`。
pub(super) fn respond(core: &super::Core, head: &str, body: &[u8]) -> Option<(u16, String)> {
    if !head.contains(" /_wonton/api/friends") && !head.contains(" /_wonton/api/federation-blocks")
    {
        return None;
    }
    let method = head.split(' ').next().unwrap_or_default().to_owned();
    let path = head.split(' ').nth(1).unwrap_or_default().to_owned();
    let mut state = core.friends.lock().unwrap();
    state.calls.push((method.clone(), path.clone()));
    let body: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    if method == "get" {
        return Some((
            200,
            json!({
                "friends": state.friends,
                "blocked_servers": state.blocked,
                "removed": state.removed,
                "can_block_federation": true,
                "owner_scope": true,
            })
            .to_string(),
        ));
    }
    if path.contains("federation-blocks") {
        let name = path.rsplit('/').next().unwrap_or_default().to_owned();
        if method == "post" {
            state.blocked.push(name);
        } else {
            state.blocked.retain(|blocked| blocked != &name);
        }
        return Some((204, String::new()));
    }
    if method == "put" {
        let encoded = path.rsplit('/').next().unwrap_or_default();
        let user = encoded.replace("%40", "@").replace("%3a", ":");
        state.friends.retain(|friend| friend["user_id"] != user);
        state.removed.retain(|removed| removed["user_id"] != user);
        state.friends.push(json!({
            "user_id": user,
            "display_name": body["display_name"],
            "shared_plugins": [],
            "last_opened_share_at_ms": null,
        }));
        return Some((204, String::new()));
    }
    if method == "delete" {
        let encoded = path.rsplit('/').next().unwrap_or_default();
        let user = encoded.replace("%40", "@").replace("%3a", ":");
        state.friends.retain(|friend| friend["user_id"] != user);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or_default();
        state
            .removed
            .push(json!({"user_id": user, "removed_at_ms": now}));
        return Some((204, String::new()));
    }
    Some((404, String::new()))
}
