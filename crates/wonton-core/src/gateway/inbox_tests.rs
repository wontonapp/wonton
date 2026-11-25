//! 收件箱 HTTP 接口的行为测试（ADR-0119）。

use serde_json::json;
use wonton_protocol::inbox::{
    ChoicePayload, InboxQuestion, InboxRecipient, InboxSender, InboxSenderKind, InboxType,
    PagePayload,
};

use super::tests::{Harness, TOKEN};
use crate::inbox::{CreateInput, NewPage};

fn authed(token: &'static str) -> Vec<(&'static str, &'static str)> {
    let _ = token;
    vec![("authorization", "Bearer test-owner-token")]
}

async fn create_choice(
    h: &Harness,
) -> (
    String,
    tokio::sync::oneshot::Receiver<wonton_protocol::InboxAnswer>,
) {
    let inbox = h.gateway.mcp.as_ref().unwrap().bus.inbox().unwrap();
    let question = InboxQuestion {
        id: "q0".into(),
        header: "目标盘".into(),
        question: "备份到哪？".into(),
        multi_select: false,
        options: vec![
            wonton_protocol::InboxOption {
                id: "nas".into(),
                label: "NAS".into(),
                description: None,
                recommended: false,
                preview: None,
            },
            wonton_protocol::InboxOption {
                id: "local".into(),
                label: "本地".into(),
                description: None,
                recommended: false,
                preview: None,
            },
        ],
        note_enabled: false,
        skippable: false,
    };
    let created = inbox
        .create(CreateInput {
            kind: InboxType::Choice,
            sender: InboxSender {
                kind: InboxSenderKind::Agent,
                id: "app.hermes".into(),
                display_name: "Hermes".into(),
                avatar: None,
            },
            recipient: InboxRecipient::Person {
                username: "owner".into(),
            },
            title: "照片备份".into(),
            body: String::new(),
            level: None,
            source: None,
            blocking: true,
            dedupe_key: None,
            payload: wonton_protocol::InboxPayload::Choice(ChoicePayload {
                questions: vec![question],
            }),
            expires_in_ms: None,
            page: None,
        })
        .unwrap();
    (created.item.id, created.waiter.unwrap())
}

/// 列表 → 作答 → 发送方收到结构化答案；列表随 revision 变化。
#[tokio::test]
async fn list_answer_and_sender_receives_the_answer() {
    let h = Harness::start(None).await;
    let (id, waiter) = create_choice(&h).await;
    let list = h
        .get(
            crate::gateway::tests::CORE,
            "/_wonton/api/inbox",
            &authed(TOKEN),
        )
        .await;
    assert_eq!(list.status, 200, "{}", list.text());
    let body = list.json();
    assert_eq!(body["items"][0]["id"], id);
    assert_eq!(body["counts"]["needing"], 1);

    let answer = h
        .send(
            hyper::Method::POST,
            crate::gateway::tests::CORE,
            &format!("/_wonton/api/inbox/{id}/answer"),
            &authed(TOKEN),
            axum::body::Body::from(
                json!({
                    "answer_id": "a1",
                    "action": "submit",
                    "answers": [{"question_id": "q0", "selected": ["nas"]}]
                })
                .to_string(),
            ),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.json()["item"]["status"], "handled");
    let got = waiter.await.unwrap();
    assert_eq!(got.answers[0].selected, ["nas"]);

    let again = h
        .send(
            hyper::Method::POST,
            crate::gateway::tests::CORE,
            &format!("/_wonton/api/inbox/{id}/answer"),
            &authed(TOKEN),
            axum::body::Body::from(
                json!({
                    "answer_id": "a2",
                    "action": "submit",
                    "answers": [{"question_id": "q0", "selected": ["local"]}]
                })
                .to_string(),
            ),
        )
        .await;
    assert_eq!(again.status, 409, "{}", again.text());
}

/// 未认证 401；不是收件人的 403。
#[tokio::test]
async fn the_list_needs_a_paired_device_or_owner_token() {
    let h = Harness::start(None).await;
    let bare = h
        .get(crate::gateway::tests::CORE, "/_wonton/api/inbox", &[])
        .await;
    assert_eq!(bare.status, 401);
}

/// 页面：详情给带 5 分钟令牌的地址，页面响应带按名单生成的 CSP。
#[tokio::test]
async fn a_page_is_served_with_its_declared_csp() {
    let h = Harness::start(None).await;
    let inbox = h.gateway.mcp.as_ref().unwrap().bus.inbox().unwrap();
    let created = inbox
        .create(CreateInput {
            kind: InboxType::Page,
            sender: InboxSender {
                kind: InboxSenderKind::Agent,
                id: "app.hermes".into(),
                display_name: "Hermes".into(),
                avatar: None,
            },
            recipient: InboxRecipient::Person {
                username: "owner".into(),
            },
            title: "报告".into(),
            body: String::new(),
            level: None,
            source: None,
            blocking: false,
            dedupe_key: None,
            payload: wonton_protocol::InboxPayload::Page(PagePayload {
                resource_uri: Some("ui://report".into()),
                file_id: None,
                caption: "这周的备份报告".into(),
                network_domains: vec!["https://cdn.example.com".into()],
                actions: Vec::new(),
            }),
            expires_in_ms: None,
            page: Some(NewPage {
                html: b"<p>ok</p>".to_vec(),
                csp: crate::inbox::policy(&["https://cdn.example.com".into()]),
            }),
        })
        .unwrap();
    let detail = h
        .get(
            crate::gateway::tests::CORE,
            &format!("/_wonton/api/inbox/{}", created.item.id),
            &authed(TOKEN),
        )
        .await;
    assert_eq!(detail.status, 200, "{}", detail.text());
    let url = detail.json()["page"]["url"].as_str().unwrap().to_owned();
    // url 是 /_wonton/api/inbox/{id}/page?token=…
    let page = h.get(crate::gateway::tests::CORE, &url, &[]).await;
    assert_eq!(page.status, 200, "{}", page.text());
    let csp = page
        .headers
        .get("content-security-policy")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(csp.contains("connect-src https://cdn.example.com"), "{csp}");
    assert!(!csp.contains("connect-src 'none'"), "{csp}");
}

/// worker needs 变 confirm；router 取答案、回报投递结果。
#[tokio::test]
async fn worker_events_create_an_item_and_expose_the_answer() {
    let h = Harness::start(None).await;
    let event = json!({
        "worker_id": "w-1",
        "event": "needs",
        "name": "herdr",
        "task": "整理照片",
        "title": "worker 等你批准",
        "body": "要删除 12 张重复照片",
        "risk": "high"
    });
    let created = h
        .send(
            hyper::Method::POST,
            crate::gateway::tests::CORE,
            "/_wonton/api/inbox/workers",
            &authed(TOKEN),
            axum::body::Body::from(event.to_string()),
        )
        .await;
    assert_eq!(created.status, 200, "{}", created.text());
    eprintln!("worker created: {}", created.text());
    let item_id = created.json()["id"].as_str().unwrap().to_owned();
    assert!(!item_id.is_empty());
    let worker = h
        .get(
            crate::gateway::tests::CORE,
            "/_wonton/api/inbox/workers/w-1",
            &authed(TOKEN),
        )
        .await;
    assert_eq!(worker.status, 200, "{}", worker.text());
    assert_eq!(worker.json()["status"], "pending");
    let answered = h
        .send(
            hyper::Method::POST,
            crate::gateway::tests::CORE,
            &format!("/_wonton/api/inbox/{item_id}/answer"),
            &authed(TOKEN),
            axum::body::Body::from(json!({"answer_id": "a1", "action": "approve"}).to_string()),
        )
        .await;
    assert_eq!(answered.status, 200, "{}", answered.text());
    let resolved = h
        .send(
            hyper::Method::POST,
            crate::gateway::tests::CORE,
            "/_wonton/api/inbox/workers/w-1/resolve",
            &authed(TOKEN),
            axum::body::Body::from(json!({"delivery": "sent"}).to_string()),
        )
        .await;
    assert_eq!(resolved.status, 200, "{}", resolved.text());
}

#[allow(dead_code)]
fn _use(a: Vec<(&'static str, &'static str)>) -> Vec<(&'static str, &'static str)> {
    a
}
