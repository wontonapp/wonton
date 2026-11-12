use std::time::Duration;

use axum::{Router, routing::get};
use serde_json::json;
use tokio::net::TcpListener;
use wonton_protocol::{Answer, CardLine};

use super::agent_install_tests::{Bench, INSTALL, bench, plain};
use super::tests::{failed, text};
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, PluginPackage};

const ID: &str = "app.agent.isolation";

struct WebApp {
    b: Bench,
    port: u16,
}

impl WebApp {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/", get(|| async { "ok" })))
                .await
                .unwrap();
        });
        Self {
            b: bench().await,
            port,
        }
    }

    fn write(&self, version: &str, isolation: Option<bool>, permissions: &str) {
        let (services, mut manifest) = plain(ID, version, permissions);
        if let Some(isolation) = isolation {
            manifest.push_str(&format!(
                "  webui: {{service: web, port: {}, cross_origin_isolated: {isolation}}}\n",
                self.port,
            ));
        }
        self.b.write_package("page", services, &manifest);
    }

    fn record(&self) -> crate::plugin::store::InstalledRecord {
        self.b
            .s
            .manager
            .installed_record(&PluginId::parse(ID).unwrap())
            .unwrap()
            .unwrap()
    }

    async fn rejected_card(&self, nth: usize) -> Vec<CardLine> {
        let call = self.b.s.spawn(INSTALL, json!({"dir": "page"}));
        let card = self.b.s.cards.wait_for(nth).await;
        self.b.s.answer(&card.id, Answer::Reject).unwrap();
        assert!(failed(&call.await.unwrap()));
        card.lines
    }
}

#[tokio::test]
async fn first_install_adds_an_emphasized_localized_isolation_warning() {
    let app = WebApp::new().await;
    let mut notices = Vec::new();
    for (index, language) in ["zh-Hans", "en-US"].into_iter().enumerate() {
        app.b.s.manager.note_owner_languages(Some(language));
        app.write("0.1.0", Some(false), "");
        let baseline = app.rejected_card(index * 2 + 1).await;
        app.write("0.1.0", Some(true), "");
        let lines = app.rejected_card(index * 2 + 2).await;
        let added: Vec<_> = lines
            .into_iter()
            .filter(|line| !baseline.contains(line))
            .collect();
        let [notice] = added.as_slice() else {
            panic!("{added:?}")
        };
        assert!(notice.emphasis);
        notices.push(notice.text.clone());
        assert!(!app.b.installed(ID));
    }
    assert_ne!(notices[0], notices[1]);
    assert!(!notices[0].is_ascii());
    assert!(notices[1].is_ascii());
}

#[tokio::test]
async fn enabling_isolation_alone_requires_consent_and_rejection_keeps_the_old_version() {
    for previous in [None, Some(false)] {
        let app = WebApp::new().await;
        app.write("0.1.0", previous, "");
        app.b
            .installed_with_answer("page", 1, Answer::Approve)
            .await;
        assert!(app.b.installed(ID));
        app.write("0.2.0", Some(true), "");
        let lines = app.rejected_card(2).await;
        assert_eq!(app.record().version, "0.1.0");
        let call = app.b.s.spawn(INSTALL, json!({"dir": "page"}));
        let card = app.b.s.cards.wait_for(3).await;
        assert_eq!(card.lines, lines);
        assert_eq!(app.record().version, "0.1.0");
        app.b.s.answer(&card.id, Answer::Approve).unwrap();
        let result = call.await.unwrap();
        assert!(!failed(&result), "{}", text(&result));
        assert_eq!(app.record().version, "0.2.0");
        let detail = app
            .b
            .s
            .manager
            .installed_detail(
                &PluginId::parse(ID).unwrap(),
                &Languages::from_header(Some("zh")),
            )
            .unwrap();
        let [notice] = detail.notices.as_slice() else {
            panic!("{detail:?}")
        };
        assert!(notice.emphasis);
        assert_eq!(card.lines.iter().filter(|line| *line == notice).count(), 1);
    }
}

#[tokio::test]
async fn isolation_is_listed_alongside_new_permissions() {
    let app = WebApp::new().await;
    app.write("0.1.0", Some(false), "");
    app.b
        .installed_with_answer("page", 1, Answer::Approve)
        .await;
    app.write("0.2.0", Some(false), "storage.share.documents:read");
    let permissions_only = app.rejected_card(2).await;
    app.write("0.2.0", Some(true), "storage.share.documents:read");
    let with_isolation = app.rejected_card(3).await;
    assert!(
        permissions_only
            .iter()
            .all(|line| with_isolation.contains(line))
    );
    let added: Vec<_> = with_isolation
        .iter()
        .filter(|line| !permissions_only.contains(line))
        .collect();
    let [notice] = added.as_slice() else {
        panic!("{added:?}")
    };
    assert!(notice.emphasis);
    assert_eq!(app.record().version, "0.1.0");
}

#[tokio::test]
async fn unchanged_or_removed_isolation_updates_quietly_but_owner_can_require_a_card() {
    let app = WebApp::new().await;
    app.write("0.1.0", Some(true), "");
    app.b
        .installed_with_answer("page", 1, Answer::Approve)
        .await;
    assert!(app.b.installed(ID));
    let id = PluginId::parse(ID).unwrap();
    let langs = Languages::from_header(Some("zh"));
    let notice = app
        .b
        .s
        .manager
        .installed_detail(&id, &langs)
        .unwrap()
        .notices[0]
        .clone();
    for (version, isolation) in [("0.2.0", true), ("0.3.0", false)] {
        app.write(version, Some(isolation), "");
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            app.b.s.call(INSTALL, json!({"dir": "page"})),
        )
        .await
        .unwrap();
        assert!(!failed(&result), "{}", text(&result));
        assert_eq!(app.b.s.cards.posted.lock().unwrap().len(), 1);
        let record = app.record();
        assert_eq!(record.version, version);
        let manifest =
            PluginPackage::manifest_of(&record.source, &Languages::from_header(Some("zh-Hans")))
                .unwrap();
        assert_eq!(manifest.webui.unwrap().cross_origin_isolated, isolation);
        assert_eq!(app.b.s.audit().last().unwrap()["decision"], "allowed");
    }
    app.b
        .s
        .manager
        .set_ask_every_update(&id, true, &langs)
        .unwrap();
    app.write("0.4.0", Some(true), "");
    app.b
        .installed_with_answer("page", 2, Answer::Approve)
        .await;
    assert_eq!(app.record().version, "0.4.0");
    app.write("0.5.0", Some(true), "");
    let lines = app.rejected_card(3).await;
    assert_eq!(lines.iter().filter(|line| **line == notice).count(), 1);
    assert_eq!(app.record().version, "0.4.0");
}
