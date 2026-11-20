//! 正文里的 `@显示名` 变成 `matrix.to` 链接（胶囊），选中的用户写进 `m.mentions`（ADR-0109 §6）。
//!
//! 只认输入框里选过的人：正文里打的名字没选中不算，选中了但正文里没写也照样写进 `m.mentions`（推送照响）。

use std::collections::HashSet;

use matrix_sdk::ruma::{
    OwnedUserId, UserId,
    events::Mentions,
    events::room::message::{MessageType, RoomMessageEventContent, TextMessageEventContent},
};

use crate::message::MemberNames;

/// 选中的人里真正在房间里的，去重后按原顺序返回；自己的、不是成员的、解析不了的都不算。
pub(crate) fn selected(ids: &[String], names: &MemberNames) -> Vec<OwnedUserId> {
    let mut seen = HashSet::new();
    ids.iter()
        .filter_map(|raw| raw.parse::<OwnedUserId>().ok())
        .filter(|user| names.contains(user) && !names.is_own(user))
        .filter(|user| seen.insert(user.to_owned()))
        .collect()
}

/// 文字事件：`body` 原样，`formatted_body` 里选中的名字换成 `matrix.to` 链接。正文里一处都没写时不出 HTML，
/// `m.mentions` 照写。
pub(crate) fn content(
    body: &str,
    users: &[OwnedUserId],
    names: &MemberNames,
) -> RoomMessageEventContent {
    let mut content = RoomMessageEventContent::text_plain(body);
    if let Some(html) = html(body, users, names) {
        content.msgtype = MessageType::Text(TextMessageEventContent::html(body, html));
    }
    if !users.is_empty() {
        content = content.add_mentions(Mentions::with_user_ids(users.iter().cloned()));
    }
    content
}

/// 正文转成 HTML：同一个位置多个名字取最长的，从左往右，一个名字里不再套别的。
fn html(body: &str, users: &[OwnedUserId], names: &MemberNames) -> Option<String> {
    if users.is_empty() {
        return None;
    }
    let mut pills: Vec<(String, &UserId)> = users
        .iter()
        .map(|user| (names.name(user), user.as_ref()))
        .filter(|(name, _)| !name.trim().is_empty())
        .collect();
    pills.sort_by_key(|(name, _)| std::cmp::Reverse(name.chars().count()));

    let mut out = String::new();
    let mut cursor = 0;
    let mut inserted = false;
    for (index, _) in body.char_indices() {
        if index < cursor || body.as_bytes()[index] != b'@' {
            continue;
        }
        let after = &body[index + 1..];
        let Some((name, user)) = pills
            .iter()
            .find(|(name, _)| after.starts_with(name.as_str()))
        else {
            continue;
        };
        out.push_str(&escape(&body[cursor..index]));
        out.push_str(&format!(
            "<a href=\"https://matrix.to/#/{}\">@{}</a>",
            encode(user.as_str()),
            escape(name)
        ));
        cursor = index + 1 + name.len();
        inserted = true;
    }
    if !inserted {
        return None;
    }
    out.push_str(&escape(&body[cursor..]));
    Some(out)
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// 用户 ID 按 RFC 3986 的路径片段编码，`@`、`:` 和别的非保留字符都转成 `%XX`。
fn encode(user: &str) -> String {
    let mut out = String::with_capacity(user.len());
    for byte in user.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte.into());
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::{owned_user_id, user_id};
    use serde_json::json;

    use super::*;

    fn names() -> MemberNames {
        MemberNames::new(
            [
                ("@hermes:example.org".parse().unwrap(), "Hermes".to_owned()),
                ("@tom:example.org".parse().unwrap(), "Tom".to_owned()),
                ("@tommy:example.org".parse().unwrap(), "Tommy".to_owned()),
            ]
            .into(),
            user_id!("@me:example.org").to_owned(),
            false,
        )
    }

    fn json(content: RoomMessageEventContent) -> serde_json::Value {
        serde_json::to_value(content).unwrap()
    }

    fn users(ids: &[&str], names: &MemberNames) -> Vec<OwnedUserId> {
        selected(
            &ids.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>(),
            names,
        )
    }

    #[test]
    fn pills_follow_the_longest_name_and_keep_the_body() {
        let names = names();
        let users = users(&["@tom:example.org", "@tommy:example.org"], &names);
        let content = content("@Tommy 你好 @Tom", &users, &names);
        let value = json(content);
        assert_eq!(value["body"], "@Tommy 你好 @Tom");
        let html = value["formatted_body"].as_str().unwrap();
        assert!(
            html.starts_with("<a href=\"https://matrix.to/#/%40tommy%3Aexample.org\">@Tommy</a>")
        );
        assert!(html.ends_with("<a href=\"https://matrix.to/#/%40tom%3Aexample.org\">@Tom</a>"));
        assert_eq!(
            value["m.mentions"]["user_ids"],
            json!(["@tom:example.org", "@tommy:example.org"])
        );
    }

    #[test]
    fn names_not_written_elsewhere_still_land_in_mentions() {
        let names = names();
        let users = users(&["@hermes:example.org"], &names);
        let value = json(content("你好", &users, &names));
        assert_eq!(value["body"], "你好");
        assert!(value.get("formatted_body").is_none());
        assert_eq!(
            value["m.mentions"]["user_ids"],
            json!(["@hermes:example.org"])
        );
    }

    #[test]
    fn plain_text_without_selection_has_no_html_and_no_mentions() {
        let names = names();
        let value = json(content("@Hermes你好", &[], &names));
        assert_eq!(value["body"], "@Hermes你好");
        assert!(value.get("formatted_body").is_none());
        assert!(value.get("m.mentions").is_none());
    }

    #[test]
    fn selection_skips_own_strangers_duplicates_and_junk() {
        let names = names();
        let ids = [
            "@me:example.org",
            "@stranger:example.org",
            "@hermes:example.org",
            "@hermes:example.org",
            "不是 ID",
        ];
        let ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
        assert_eq!(
            selected(&ids, &names),
            vec![owned_user_id!("@hermes:example.org")]
        );
    }

    #[test]
    fn body_characters_are_escaped() {
        let names = names();
        let users = users(&["@hermes:example.org"], &names);
        let value = json(content("a & b <c> @Hermes", &users, &names));
        assert_eq!(value["body"], "a & b <c> @Hermes");
        assert!(
            value["formatted_body"]
                .as_str()
                .unwrap()
                .contains("a &amp; b &lt;c&gt; ")
        );
    }
}
