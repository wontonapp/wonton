use super::*;
use crate::bot_web_registration;

fn bot(plugin_id: &str, name: &str) -> BotRef {
    BotRef {
        plugin_id: plugin_id.into(),
        bot: name.into(),
    }
}

#[test]
fn bot_web_labels_bind_the_complete_plugin_and_bot_identity() {
    // SHA-256 向量由 Python hashlib 独立计算，包含分隔的 NUL 字节。
    assert_eq!(
        bot_web_label(&bot("com.example.agent", "alice")),
        "bot--alice-com-example-agent-f4debdbf"
    );
    assert_eq!(
        bot_web_label(&bot("com.example.agent", "bob")),
        "bot--bob-com-example-agent-a17ffd67"
    );
    assert_eq!(
        bot_web_label(&bot("org.example.agent", "alice")),
        "bot--alice-org-example-agent-0abb0138"
    );
    assert_eq!(
        bot_web_label(&bot("com.example.agent", "default")),
        "com-example-agent"
    );
}

#[test]
fn bot_web_labels_truncate_only_the_slug_at_the_exact_dns_boundary() {
    let name = "b";
    for (slug_len, expected_len) in [(46, 62), (47, 63), (48, 63)] {
        let plugin_id = format!("a.{}", "x".repeat(slug_len - 2));
        assert_eq!(plugin_id.len(), slug_len);
        let label = bot_web_label(&bot(&plugin_id, name));
        assert_eq!(label.len(), expected_len);
        let readable = label.strip_prefix("bot--b-").unwrap();
        let (slug, digest) = readable.rsplit_once('-').unwrap();
        assert_eq!(slug, &plugin_id.replace('.', "-")[..slug_len.min(47)]);
        assert_eq!(digest.len(), 8);
    }
    let name = "b".repeat(32);
    let plugin_id = format!("a.{}", "x".repeat(63 - "a.".len()));
    assert_eq!(name.len(), 32);
    assert_eq!(plugin_id.len(), 63);
    assert_eq!(
        bot_web_label(&bot(&plugin_id, &name)),
        "bot--bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-a-xxxxxxxxxxxxxx-9af9ffcc"
    );
    assert_eq!(bot_web_label(&bot(&plugin_id, &name)).len(), 63);
    assert_eq!(
        bot_web_label(&bot(&plugin_id, "default")),
        plugin_id.replace('.', "-")
    );
}

#[test]
fn short_digest_collisions_refuse_registration_without_reassigning_the_label() {
    // 两个合法 ID 的前 8 位 SHA-256 相同，且可读 slug 截短后相同。
    let name = "b".repeat(32);
    let registered = bot("a.xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx0000eb1d", &name);
    let requested = bot("a.xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx00020c00", &name);
    let label = "bot--bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-a-xxxxxxxxxxxxxx-5d92ad5b";
    assert_eq!(bot_web_label(&registered), label);
    assert_eq!(bot_web_label(&requested), label);
    assert_eq!(bot_web_registration(&registered, None).unwrap(), label);
    assert_eq!(
        bot_web_registration(&registered, Some(&registered)).unwrap(),
        label
    );
    let error = bot_web_registration(&requested, Some(&registered)).unwrap_err();
    assert_eq!(error.label, label);
    assert_eq!(error.registered, registered);
    assert_eq!(error.requested, requested);
}

#[test]
fn unknown_web_status_or_failure_never_supplies_a_page() {
    assert_eq!(
        serde_json::from_str::<BotWebView>(
            r#"{"state":"future_ready","page":{"url":"https://example.com"}}"#
        )
        .unwrap(),
        BotWebView::Other
    );
    assert_eq!(
        serde_json::from_str::<BotWebView>(
            r#"{"state":"unavailable","reason":"future_permission"}"#
        )
        .unwrap(),
        BotWebView::Unavailable {
            reason: BotWebFailure::Other,
        }
    );
}
