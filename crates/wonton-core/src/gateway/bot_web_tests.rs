//! Bot 网页的目标、权限与完整主机登录边界；真实网关，独立回环上游。
//!
//! 各行为分在同名子模块，共用的插件上游和其他 Bot 夹具留在这里。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::ConnectInfo;
use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wonton_protocol::{BotKeeper, BotRef, bot_web_label, bot_web_path};

use super::Person;
use super::fakes::{echo_upstream, fake_hydra};
use super::family_tests::send_as;
use super::tests::{CORE, Harness, Reply, TOKEN, WHOAMI};
use super::{APP_COOKIE, CORE_COOKIE, TunnelPeer};
use crate::members::MemberRecord;
use crate::plugin::languages::Languages;
use crate::plugin::upstreams::Endpoint;
use crate::plugin::{PluginId, PluginPackage, PluginStatus};

mod availability;
mod hosts;
mod login;
mod metadata;

pub(super) fn bot(name: &str) -> BotRef {
    BotRef {
        plugin_id: "app.whoami".into(),
        bot: name.into(),
    }
}

pub(super) fn host(name: &str) -> String {
    format!("{}.{}", bot_web_label(&bot(name)), CORE)
}

pub(super) fn source() -> String {
    WHOAMI.replace("  icon: icon.svg\n", "")
        .replace("  permissions: []", "  permissions: [matrix.bot]\n  bots: {services: [web]}")
        .replace("    port: 8000", "    port: 8000\n    path: /panel?mode=bot\n    cross_origin_isolated: true\n    sso: oidc\n    oidc:\n      redirect_paths: [/cb]\n      admin_groups: [admin]")
}

pub(super) fn record(h: &Harness, family: bool, compose: &str) {
    std::fs::write(h.data_dir.join("plugins/app.whoami.json"), json!({
        "id": "app.whoami", "version": "0.1.0", "source": compose.replace("  icon: icon.svg\n", ""),
        "granted_permissions": ["matrix.bot"], "installed_at": 1, "family": family,
    }).to_string()).unwrap();
}

pub(super) async fn setup(h: &Harness) -> Arc<AtomicUsize> {
    let compose = source();
    record(h, false, &compose);
    h.manager.register_for_test(
        PluginPackage::manifest_of(&compose, &Languages::default()).unwrap(),
        PluginStatus::Running,
        Some(h.upstream.to_string()),
    );
    h.manager.bot_records_for_test(
        &PluginId::parse("app.whoami").unwrap(),
        &[
            (
                "default",
                "@whoami:test",
                BotKeeper::User {
                    username: "owner".into(),
                },
                "a",
            ),
            (
                "mia",
                "@whoami-mia:test",
                BotKeeper::User {
                    username: "mia".into(),
                },
                "b",
            ),
        ],
    );
    let other_hits = Arc::new(AtomicUsize::new(0));
    let other = echo_upstream(other_hits.clone()).await;
    h.manager
        .bot_web_route_for_test(bot("default"), Endpoint::Tcp(h.upstream.to_string()), true);
    h.manager
        .bot_web_route_for_test(bot("mia"), Endpoint::Tcp(other.to_string()), true);
    let mut registrations = serde_json::Map::new();
    for name in ["default", "mia"] {
        registrations.insert(
            bot_web_label(&bot(name)),
            json!({"bot": bot(name), "secret": "test-only"}),
        );
    }
    std::fs::create_dir_all(h.data_dir.join("plugins/bot-web")).unwrap();
    std::fs::write(
        h.data_dir.join("plugins/bot-web/registrations.json"),
        serde_json::to_string(&registrations).unwrap(),
    )
    .unwrap();
    other_hits
}

pub(super) fn member(h: &Harness) -> iroh::EndpointId {
    h.members
        .invite(MemberRecord {
            invited_by: None,
            username: "mia".into(),
            user_id: "@mia:test".into(),
            display_name: "Mia".into(),
            email: None,
            role_id: crate::roles::DEFAULT_INVITE_ROLE.to_owned(),
            status: wonton_protocol::MemberStatus::Joined,
            wants_private_bot: false,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        })
        .unwrap();
    let device = iroh::SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(device, None, Some("mia".into()), |_| true)
        .unwrap();
    device
}

/// #518 起可见性按角色 open_apps 算：给默认邀请角色开放这个插件，family 标记不再管事。
pub(super) fn open_whoami(h: &Harness) {
    let draft = wonton_protocol::RoleDraft {
        name: "朋友".into(),
        permissions: wonton_protocol::Permissions {
            open_apps: wonton_protocol::AppScope::Selected {
                plugin_ids: vec!["app.whoami".into()],
            },
            ..wonton_protocol::Permissions::strictest()
        },
    };
    h.roles.update("friend", &draft).unwrap();
}

pub(super) fn assert_failure(reply: &Reply, status: StatusCode, reason: &str) {
    assert_eq!(reply.status, status, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some(reason));
    assert!(reply.json().get("page").is_none());
}
