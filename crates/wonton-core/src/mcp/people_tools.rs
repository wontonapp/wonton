//! 某人用的语言（ADR-0043）：插件发聊天消息、写提示时按这个人的语言来。只读，不收窄到核心支持的语言，
//! 默认语言由插件自己定。

use rmcp::model::{JsonObject, Tool};
use serde_json::{Value, json};
use wonton_protocol::MemberStatus;

use super::core_tools::tool;
use super::{Bus, Caller};
use crate::plugin::languages::accept_language_tags;

pub(super) const USER_LOCALE: &str = "user_locale";

pub(super) fn definitions() -> Vec<Tool> {
    vec![tool(
        USER_LOCALE,
        "Read which languages a person uses, to reply or send messages in their language. `user_id` is a Matrix ID \
         on this server; leave it out for the owner. When replying or posting on behalf of a specific person, \
         always pass their `user_id`; omitting it returns the owner's languages. Returns `locale` (their most \
         preferred BCP 47 tag) and `languages` ordered by preference; `locale` is null and `languages` empty when \
         nothing is recorded. The core does not pick a default: choose one yourself.",
        json!({"type": "object", "properties": {
            "user_id": {"type": "string", "description": "Matrix ID like @alice:example.org; the owner when left out"},
        }, "additionalProperties": false}),
        true,
    )]
}

pub(super) fn call(bus: &Bus, caller: &Caller, args: &JsonObject) -> Result<Value, String> {
    let people = bus.people()?;
    let owner = bus.plugins.owner_name();
    let wanted = match args.get("user_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) => Some(id.as_str()),
        Some(_) => return Err("invalid_input: user_id must be a string".to_owned()),
    };
    let localpart = match wanted {
        Some(id) => local_part(id, &people.server_name)?,
        None => owner.to_owned(),
    };
    if localpart != owner {
        // 先判权限再找人：没有权限的插件不能拿 ID 探出家里有谁（是存在的家人还是陌生人都是同一个错）。
        if !bus.plugins.gets_owner_identity(&caller.id) {
            return Err(format!(
                "permission_denied: reading someone else's languages needs the owner.identity permission \
                 ({localpart} is not the owner)"
            ));
        }
        known(people, &localpart)?;
    }
    let languages: Vec<String> = bus
        .plugins
        .person_languages(&localpart)
        .map(|header| accept_language_tags(&header))
        .unwrap_or_default();
    let locale = languages.first().cloned();
    Ok(json!({
        "user_id": format!("@{localpart}:{}", people.server_name),
        "locale": locale,
        "languages": languages,
    }))
}

/// `@<用户名>:<服务器名>` 里本服务器的用户名。
fn local_part(user_id: &str, server_name: &str) -> Result<String, String> {
    let invalid = || format!("invalid_input: {user_id} is not a Matrix ID like @alice:example.org");
    let (localpart, server) = user_id
        .strip_prefix('@')
        .and_then(|rest| rest.split_once(':'))
        .ok_or_else(invalid)?;
    if localpart.is_empty() || server.is_empty() {
        return Err(invalid());
    }
    if server != server_name {
        return Err(format!(
            "not_this_server: {user_id} is not on this server ({server_name})"
        ));
    }
    Ok(localpart.to_owned())
}

/// 本服务器上认识的人：主人之外只有家人记录里、还没被移除的人。Bot 和陌生人不算。
fn known(people: &super::People, localpart: &str) -> Result<(), String> {
    match people.members.get(localpart) {
        Some(record) if record.status != MemberStatus::Removed => Ok(()),
        _ => Err(format!(
            "not_found: no person @{localpart}:{} on this server",
            people.server_name
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_ids_must_be_matrix_ids_on_this_server() {
        assert_eq!(
            local_part("@alice:example.org", "example.org").unwrap(),
            "alice"
        );
        assert_eq!(
            local_part("@alice:elsewhere.org", "example.org")
                .unwrap_err()
                .split(':')
                .next(),
            Some("not_this_server")
        );
        for bad in ["alice", "@alice", "@:example.org", "@alice:"] {
            let error = local_part(bad, "example.org").unwrap_err();
            assert!(error.starts_with("invalid_input:"), "{bad}: {error}");
        }
    }
}
