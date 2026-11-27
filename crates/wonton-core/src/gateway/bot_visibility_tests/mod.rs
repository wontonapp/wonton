//! `bot_visibility` 的分类和事实读取测试。

use std::path::PathBuf;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use hyper::HeaderMap;
use hyper::header::{AUTHORIZATION, ORIGIN};
use serde_json::{Value, json};
use wonton_protocol::{
    AppScope, BotAccess, BotKeeper, ContactEvent, ContactState, DEFAULT_INVITE_ROLE, RoleDraft,
};

use super::*;
use crate::contacts::tests::{FakeDirectory, FakeEffects, FakeInvites};
use crate::contacts::{ContactFailure, Contacts, PeerLink, PeerReach, Ports};
use crate::gateway::tests::{Harness, TOKEN};
use crate::http::HttpClient;
use crate::matrix::{BotAccount, Matrix};
use crate::members::MemberRecord;
use crate::plugin::bots::{BotEntry, BotStore, PluginBots};
use crate::plugin::manifest::PluginId;

const PLUGIN: &str = "ai.nous.hermes";

struct NoLink;

impl PeerLink for NoLink {
    fn reach(&self, _: &str) -> PeerReach {
        unreachable!("原始事实读取不查询联邦连接")
    }

    fn send(&self, _: &str, _: ContactEvent) -> BoxFuture<'_, Result<(), ContactFailure>> {
        unreachable!("原始事实读取不发送联邦事件")
    }
}

struct Fixture {
    gateway: Gateway,
    harness: Harness,
}

impl Fixture {
    async fn new() -> Self {
        let harness = Harness::start(None).await;
        let gateway = Gateway::new(
            harness.gateway.public.clone(),
            harness.gateway.owner.clone(),
            TOKEN.into(),
            harness.members.clone(),
            harness.manager.clone(),
            None,
            harness.http.clone(),
            harness.tunnel.clone(),
        );
        open_all_apps(&gateway);
        Self { gateway, harness }
    }

    fn peer(&self, username: Option<&str>) -> TunnelPeer {
        let id = iroh::SecretKey::generate().public();
        self.gateway
            .tunnel
            .devices
            .add_if(id, None, username.map(str::to_owned), |_| true)
            .unwrap();
        TunnelPeer(id)
    }

    fn member(&self, username: &str, user_id: &str) -> TunnelPeer {
        let mut record = MemberRecord::for_test(username);
        record.user_id = user_id.into();
        self.gateway.members.invite(record).unwrap();
        self.gateway.members.mark_joined(username).unwrap();
        self.peer(Some(username))
    }

    fn seed(&self, entries: &[(&str, &str, BotKeeper, &str)]) {
        let bots = entries
            .iter()
            .map(|(name, user_id, keeper, room)| {
                (
                    name.to_string(),
                    BotEntry {
                        account: BotAccount {
                            user_id: user_id.to_string(),
                            home_room: room.to_string(),
                        },
                        device_id: None,
                        keeper: keeper.clone(),
                        bus_token: "isolated-test-bus-token".into(),
                    },
                )
            })
            .collect();
        BotStore::new(self.harness.data_dir.join("plugins/bots"))
            .save(&PluginId::parse(PLUGIN).unwrap(), &PluginBots { bots })
            .unwrap();
    }

    fn record_path(&self, plugin: &str) -> PathBuf {
        self.harness
            .data_dir
            .join("plugins/bots")
            .join(format!("{plugin}.json"))
    }

    fn contacts(&mut self, rows: Value) -> (Arc<FakeInvites>, Arc<FakeEffects>, PathBuf) {
        let path = self.harness.data_dir.join("raw-contact-facts.json");
        std::fs::write(&path, serde_json::to_vec(&json!({"bots": rows})).unwrap()).unwrap();
        let invites = Arc::new(FakeInvites::default());
        let receipts = Arc::new(crate::contacts::ApplyReceipts::default());
        let effects = Arc::new(FakeEffects::new(receipts.clone()));
        self.gateway.contacts = Some(
            Contacts::open(
                &path,
                "test",
                Ports {
                    directory: Arc::new(FakeDirectory::default()),
                    link: Arc::new(NoLink),
                    invites: invites.clone(),
                    effects: effects.clone(),
                    receipts,
                    open_federation: false,
                },
            )
            .unwrap(),
        );
        (invites, effects, path)
    }

    fn facts(&self, peer: Option<TunnelPeer>, bot: &BotRef) -> BotIdentityAndContactFacts {
        bot_identity_and_contact_facts(&self.gateway, peer, &HeaderMap::new(), bot).unwrap()
    }
}

/// 种子角色默认一个应用都不开放；本文件的用例只测 Bot 归属和联系人，让角色能打开插件。
fn open_all_apps(gateway: &Gateway) {
    for id in ["admin", "family", "child", DEFAULT_INVITE_ROLE] {
        let Some(role) = gateway.roles.get(id) else {
            continue;
        };
        let mut permissions = role.permissions.clone();
        permissions.open_apps = AppScope::All;
        gateway
            .roles
            .update(
                &role.id,
                &RoleDraft {
                    name: id.to_owned(),
                    permissions,
                },
            )
            .unwrap();
    }
}

fn bot(name: &str) -> BotRef {
    BotRef {
        plugin_id: PLUGIN.into(),
        bot: name.into(),
    }
}

fn contact_rows(bot: &BotRef, contacts: Value) -> Value {
    json!({
        (format!("{}/{}", bot.plugin_id, bot.bot)): {"bot": bot, "contacts": contacts}
    })
}

mod access;
