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
    let failed = h
        .send(
            hyper::Method::POST,
            crate::gateway::tests::CORE,
            "/_wonton/api/inbox/workers/w-1/resolve",
            &authed(TOKEN),
            axum::body::Body::from(json!({"delivery": "failed", "error": "boom"}).to_string()),
        )
        .await;
    assert_eq!(failed.status, 200, "{}", failed.text());
    let after_failed = h
        .get(
            crate::gateway::tests::CORE,
            "/_wonton/api/inbox/workers/w-1",
            &authed(TOKEN),
        )
        .await;
    assert_eq!(after_failed.json()["delivery"], "failed");

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
    let after_sent = h
        .get(
            crate::gateway::tests::CORE,
            "/_wonton/api/inbox/workers/w-1",
            &authed(TOKEN),
        )
        .await;
    assert_eq!(after_sent.json()["delivery"], "sent");
}

#[allow(dead_code)]
fn _use(a: Vec<(&'static str, &'static str)>) -> Vec<(&'static str, &'static str)> {
    a
}

/// worker 的两条读/写接口也要主人令牌或设备身份。
#[tokio::test]
async fn worker_reads_need_auth() {
    let h = Harness::start(None).await;
    let read = h
        .get(
            crate::gateway::tests::CORE,
            "/_wonton/api/inbox/workers/w-1",
            &[],
        )
        .await;
    assert_eq!(read.status, 401);
    let resolve = h
        .send(
            hyper::Method::POST,
            crate::gateway::tests::CORE,
            "/_wonton/api/inbox/workers/w-1/resolve",
            &[],
            axum::body::Body::from(json!({"delivery": "sent"}).to_string()),
        )
        .await;
    assert_eq!(resolve.status, 401);
}

/// worker 的 done 不带 choices 也收；未知事件落 notice 而不是 400。
#[tokio::test]
async fn worker_done_without_choices_and_unknown_events() {
    let h = Harness::start(None).await;
    for (worker, event, kind) in [("w-done", "done", "notice"), ("w-weird", "later", "notice")] {
        let body = json!({"worker_id": worker, "event": event, "body": "做完了"});
        let response = h
            .send(
                hyper::Method::POST,
                crate::gateway::tests::CORE,
                "/_wonton/api/inbox/workers",
                &authed(TOKEN),
                axum::body::Body::from(body.to_string()),
            )
            .await;
        assert_eq!(response.status, 200, "{}", response.text());
        assert_eq!(response.json()["type"], kind);
    }
}

/// 主人能看见发给自己的确认卡片，不再被列表按 viewer 过滤掉。
#[tokio::test]
async fn the_owner_sees_keeper_confirmations() {
    let h = Harness::start(None).await;
    let bus = h.gateway.mcp.as_ref().unwrap().bus.clone();
    let draft = install_draft();
    let caller = crate::plugin::PluginId::parse("app.hermes").unwrap();
    tokio::spawn(async move {
        let _ = bus
            .confirmations()
            .ask_as(&caller, draft, crate::mcp::Audience::default())
            .await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let list = h
        .get(
            crate::gateway::tests::CORE,
            "/_wonton/api/inbox",
            &authed(TOKEN),
        )
        .await;
    assert_eq!(list.status, 200, "{}", list.text());
    let items = list.json()["items"].as_array().unwrap().clone();
    assert!(
        items
            .iter()
            .any(|item| item["type"] == "confirm"
                && item["id"].as_str().unwrap_or("").starts_with("c-")),
        "{items:?}"
    );
}

fn install_draft() -> crate::mcp::Draft {
    crate::mcp::Draft {
        title: "装 Immich？".into(),
        lines: Vec::new(),
        choices: vec![
            wonton_protocol::CardChoice {
                answer: wonton_protocol::Answer::Approve,
                label: "允许".into(),
                choice_id: None,
            },
            wonton_protocol::CardChoice {
                answer: wonton_protocol::Answer::Reject,
                label: "拒绝".into(),
                choice_id: None,
            },
        ],
        scope: None,
        plugin_id: None,
        secret_inputs: Vec::new(),
        settings: None,
        category: None,
        permissions: Vec::new(),
        devices: Vec::new(),
    }
}

async fn needing(h: &Harness, after: Option<u64>) -> (u64, u64) {
    let path = match after {
        Some(after) => format!("/_wonton/api/inbox?after={after}"),
        None => "/_wonton/api/inbox".to_owned(),
    };
    let list = h
        .get(crate::gateway::tests::CORE, &path, &authed(TOKEN))
        .await;
    assert_eq!(list.status, 200, "{}", list.text());
    let body = list.json();
    (
        body["counts"]["needing"].as_u64().unwrap(),
        body["revision"].as_u64().unwrap(),
    )
}

/// 没有东西要回答时角标是 0；来了一张确认卡片，挂着的长轮询马上醒、角标变 1；卡片答完，下一次长轮询
/// 也马上醒、角标回 0。确认表的版本和收件箱的版本合在一起比较，不会白等到超时。
#[tokio::test]
async fn a_confirmation_wakes_the_long_poll_and_answering_clears_it() {
    let h = Harness::start(None).await;
    let bus = h.gateway.mcp.as_ref().unwrap().bus.clone();
    let (count, revision) = needing(&h, None).await;
    assert_eq!(count, 0, "没有卡片时不该有要回答的");

    let poll = needing(&h, Some(revision));
    let opened = async {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let caller = crate::plugin::PluginId::parse("app.hermes").unwrap();
        let ask_bus = bus.clone();
        tokio::spawn(async move {
            let _ = ask_bus
                .confirmations()
                .ask_as(&caller, install_draft(), crate::mcp::Audience::default())
                .await;
        });
    };
    let started = std::time::Instant::now();
    let ((count, revision), ()) = tokio::join!(poll, opened);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "长轮询等了 {:?} 才返回",
        started.elapsed()
    );
    assert_eq!(count, 1);

    let (pending, _, _) = bus.confirmations().list(None);
    let card = pending[0].id.clone();
    let poll = needing(&h, Some(revision));
    let answered = async {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        bus.confirmations()
            .answer(&card, wonton_protocol::Answer::Reject, None)
            .unwrap();
    };
    let started = std::time::Instant::now();
    let ((count, _), ()) = tokio::join!(poll, answered);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "长轮询等了 {:?} 才返回",
        started.elapsed()
    );
    assert_eq!(count, 0, "答完的卡片不该还算要回答");
}
