use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use serde_json::json;

use super::{ATTACHMENT_LIMIT, BODY_LIMIT, Message, ReceivedError, received_ip};

fn message(text: &str, html: &str) -> Message {
    serde_json::from_value(json!({
        "ID": "message-1",
        "From": {"Name": "Application", "Address": "app@plugins.wonton.internal"},
        "Subject": "subject",
        "Text": text,
        "HTML": html,
        "To": null,
        "Cc": null,
        "Bcc": null,
        "Attachments": null,
        "Inline": null
    }))
    .unwrap()
}

#[test]
fn mailpit_pascal_case_fields_and_null_arrays_deserialize() {
    let mail = message("text", "<p>html</p>");
    assert_eq!(mail.id, "message-1");
    assert_eq!(
        mail.from.as_ref().unwrap().address,
        "app@plugins.wonton.internal"
    );
    assert_eq!(mail.subject, "subject");
    assert_eq!(mail.text, "text");
    assert_eq!(mail.html, "<p>html</p>");
    assert!(mail.to.is_empty());
    assert!(mail.cc.is_empty());
    assert!(mail.bcc.is_empty());
    assert!(mail.attachments.is_empty());
    assert!(mail.inline.is_empty());

    let missing: Message = serde_json::from_value(json!({"ID": "missing"})).unwrap();
    assert!(missing.from.is_none());
    assert!(missing.to.is_empty());
    assert!(missing.cc.is_empty());
    assert!(missing.bcc.is_empty());
    assert!(missing.attachments.is_empty());
    assert!(missing.inline.is_empty());
}

#[test]
fn received_uses_only_the_first_header_for_ipv4_and_ipv6() {
    for (remote, expected) in [
        ("172.17.0.1", IpAddr::V4(Ipv4Addr::new(172, 17, 0, 1))),
        (
            "2001:db8::7",
            IpAddr::V6("2001:db8::7".parse::<Ipv6Addr>().unwrap()),
        ),
    ] {
        let headers = json!({"Received": [
            format!("from ehlo (unknown [{remote}]) by host (Mailpit) with SMTP for <owner@wonton.internal>; Sun, 08 Nov 2026 10:00:00 +0000"),
            "from attacker (unknown [192.0.2.99]) by host (Mailpit) with SMTP; forged"
        ]});
        assert_eq!(received_ip(&headers), Ok(expected));
    }
}

#[test]
fn invalid_top_received_never_falls_back_to_attacker_header() {
    for top in [
        json!("unparseable"),
        json!("from ehlo (unknown [not-an-ip]) by host (Mailpit) with SMTP; date"),
        json!("from ehlo [192.0.2.2] (unknown [not-an-ip]) by host (Mailpit); date"),
        json!(null),
        json!(7),
    ] {
        let headers = json!({"Received": [
            top,
            "from attacker (unknown [192.0.2.99]) by host (Mailpit); forged"
        ]});
        assert_eq!(received_ip(&headers), Err(ReceivedError::Invalid));
    }
    for headers in [
        json!({}),
        json!({"Received": []}),
        json!({"Received": "forged"}),
    ] {
        assert_eq!(received_ip(&headers), Err(ReceivedError::Missing));
    }
}

#[test]
fn received_ignores_forged_ehlo_brackets_and_delimiters() {
    for ehlo in [
        "[192.0.2.99]",
        "fake (unknown [192.0.2.99]) by pretend",
        "[2001:db8::bad] [192.0.2.99]",
    ] {
        let headers = json!({"Received": [format!(
            "from {ehlo} (unknown [172.17.0.1]) by host (Mailpit) with SMTP for <owner@wonton.internal>; date"
        )]});
        assert_eq!(
            received_ip(&headers),
            Ok(IpAddr::V4(Ipv4Addr::new(172, 17, 0, 1)))
        );
    }
}

#[test]
fn recipients_union_headers_and_mailpit_envelope_bcc_once_per_existing_person() {
    // Mailpit 的 SMTP 层把信封独有的 carol 补进 Bcc；bob 只在头里，按批准的并集契约仍收到。
    let mail: Message = serde_json::from_value(json!({
        "ID": "union",
        "From": {"Address": "sender@wonton.internal"},
        "To": [
            {"Address": "alice@wonton.internal"},
            {"Address": "bob@wonton.internal"},
            {"Address": "removed@wonton.internal"},
            {"Address": "alice@example.test"}
        ],
        "Cc": [
            {"Address": "alice@wonton.internal"},
            {"Address": "app@plugins.wonton.internal"}
        ],
        "Bcc": [
            {"Address": "carol@wonton.internal"},
            {"Address": "bob@wonton.internal"},
            {"Address": "carol@wonton.internal"}
        ]
    }))
    .unwrap();
    let existing = BTreeMap::from([
        ("alice@wonton.internal".into(), "alice".into()),
        ("bob@wonton.internal".into(), "bob".into()),
        ("carol@wonton.internal".into(), "carol".into()),
        ("sender@wonton.internal".into(), "sender".into()),
    ]);
    assert_eq!(mail.recipients(&existing), vec!["alice", "bob", "carol"]);
    assert!(mail.recipients(&BTreeMap::new()).is_empty());
}

#[test]
fn html_only_retains_hyperlink_without_rendering_html() {
    let mail = message(
        "",
        "<p>Reset <a href=\"https://example.test/reset?token=abc\">password</a></p>",
    );
    let prepared = mail.prepare().unwrap();
    assert!(prepared.body.contains("password"));
    assert!(
        prepared
            .body
            .contains("https://example.test/reset?token=abc")
    );
    assert!(!prepared.body.contains("<a"));
    assert!(!prepared.body.contains("<p>"));
    assert!(prepared.html_attachment.is_none());
}

#[test]
fn actual_text_including_whitespace_takes_precedence() {
    for text in ["plain text", " \n\t"] {
        let mail = message(text, "<p>Must not replace text</p>");
        let prepared = mail.prepare().unwrap();
        assert_eq!(prepared.body, text);
        assert!(prepared.html_attachment.is_none());
    }
    assert_eq!(message("", "").prepare().unwrap().body, "");
}

#[test]
fn body_limit_counts_unicode_characters_at_exact_boundary() {
    assert_eq!(BODY_LIMIT, 4000);
    let at_limit = "云".repeat(4000);
    let over_limit = format!("{at_limit}🙂");
    assert_eq!(at_limit.chars().count(), 4000);
    assert_eq!(at_limit.len(), 12000);
    assert_eq!(over_limit.chars().count(), 4001);
    assert_eq!(over_limit.len(), 12004);
    let html = "<p>Original HTML</p>";
    let exact = message(&at_limit, html);
    let exact = exact.prepare().unwrap();
    assert_eq!(exact.body, at_limit);
    assert!(exact.html_attachment.is_none());
    let over = message(&over_limit, html);
    let over = over.prepare().unwrap();
    assert_eq!(over.body, at_limit);
    assert_eq!(over.body.chars().count(), 4000);
    assert_eq!(over.html_attachment, Some(html));
    let plain = message(&over_limit, "");
    let plain = plain.prepare().unwrap();
    assert_eq!(plain.body, at_limit);
    assert!(plain.html_attachment.is_none());
}

#[test]
fn long_html_only_attaches_original_html() {
    let html = format!("<p>{}</p>", "云".repeat(4001));
    let mail = message("", &html);
    let prepared = mail.prepare().unwrap();
    assert_eq!(prepared.body.chars().count(), 4000);
    assert_eq!(prepared.html_attachment, Some(html.as_str()));
}

#[test]
fn attachments_and_inline_use_exact_twenty_mib_upload_boundary() {
    assert_eq!(ATTACHMENT_LIMIT, 20 * 1024 * 1024);
    let at_limit = 20_u64 * 1024 * 1024;
    let over_limit = at_limit + 1;
    assert_eq!(at_limit, 20_971_520);
    assert_eq!(over_limit, 20_971_521);
    let mail: Message = serde_json::from_value(json!({
        "ID": "attachments",
        "Attachments": [
            {"PartID": "1.1", "FileName": "exact.bin", "ContentType": "application/octet-stream", "Size": at_limit},
            {"PartID": "1.2", "FileName": "large.bin", "ContentType": "application/octet-stream", "Size": over_limit}
        ],
        "Inline": [
            {"PartID": "1.3", "FileName": "exact.png", "ContentType": "image/png", "Size": at_limit},
            {"PartID": "1.4", "FileName": "large.png", "ContentType": "image/png", "Size": over_limit}
        ]
    })).unwrap();
    let prepared = mail.prepare().unwrap();
    assert_eq!(prepared.attachments.len(), 4);
    let expected = [
        (
            "1.1",
            "exact.bin",
            "application/octet-stream",
            at_limit,
            true,
        ),
        (
            "1.2",
            "large.bin",
            "application/octet-stream",
            over_limit,
            false,
        ),
        ("1.3", "exact.png", "image/png", at_limit, true),
        ("1.4", "large.png", "image/png", over_limit, false),
    ];
    for (prepared, (part_id, name, content_type, size, upload)) in
        prepared.attachments.iter().zip(expected)
    {
        assert_eq!(prepared.attachment.part_id, part_id);
        assert_eq!(prepared.attachment.file_name, name);
        assert_eq!(prepared.attachment.content_type, content_type);
        assert_eq!(prepared.attachment.size, size);
        assert_eq!(prepared.upload, upload);
    }
}
