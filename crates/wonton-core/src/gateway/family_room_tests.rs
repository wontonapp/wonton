use std::collections::BTreeSet;
use std::sync::Arc;

use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use wonton_protocol::{FAMILY_ROOM_EVENT, FAMILY_ROOM_SYNC_PATH, MemberStatus};

use crate::gateway::family_tests::{bearer, send_as};
use crate::gateway::tests::{CORE, Harness, Reply, TOKEN};
use crate::gateway::{Gateway, router};
use crate::http::HttpClient;
use crate::matrix::{Matrix, fake};
use crate::members::{JoinCheck, MemberRecord, Members};
use crate::roles::Roles;

struct Family {
    gateway: Arc<Gateway>,
    app: axum::Router,
    matrix: Arc<Matrix>,
    fake: fake::Fake,
    harness: Harness,
}

impl Family {
    async fn start() -> Self {
        let harness = Harness::start(None).await;
        let (fake, config) = fake::start().await;
        let matrix = Arc::new(
            Matrix::connect(&config, &harness.data_dir, HttpClient::new())
                .await
                .unwrap(),
        );
        let gateway = Arc::new(Self::gateway(
            &harness,
            matrix.clone(),
            harness.members.clone(),
            harness.roles.clone(),
        ));
        Self {
            app: router(gateway.clone()),
            gateway,
            matrix,
            fake,
            harness,
        }
    }

    fn gateway(
        h: &Harness,
        matrix: Arc<Matrix>,
        members: Arc<Members>,
        roles: Arc<Roles>,
    ) -> Gateway {
        Gateway::new(
            h.gateway.public.clone(),
            h.gateway.owner.clone(),
            TOKEN.to_owned(),
            members,
            h.manager.clone(),
            None,
            h.http.clone(),
            h.tunnel.clone(),
        )
        .with_matrix(Some(matrix))
        .with_roles(roles)
    }

    async fn call(&self, method: Method, path: &str, body: Value) -> Reply {
        send_as(
            &self.app,
            None,
            method,
            CORE,
            path,
            &[
                ("authorization", &bearer()),
                ("content-type", "application/json"),
            ],
            &body.to_string(),
        )
        .await
    }

    fn pending(&self, username: &str, role: &str) -> (String, iroh::EndpointId) {
        self.gateway
            .members
            .invite(MemberRecord {
                role_id: role.to_owned(),
                wants_private_bot: false,
                ..MemberRecord::for_test(username)
            })
            .unwrap();
        let code = self.gateway.members.issue_pairing_code(username).unwrap();
        let endpoint = SecretKey::generate().public();
        assert_eq!(
            self.gateway
                .members
                .request_join(
                    code.secret.expose(),
                    endpoint,
                    "Phone",
                    &self.gateway.tunnel.devices
                )
                .unwrap(),
            JoinCheck::Pending(role.to_owned())
        );
        let id = self
            .gateway
            .members
            .join_requests()
            .iter()
            .find(|request| request.username == username)
            .unwrap()
            .id
            .clone();
        (id, endpoint)
    }

    async fn approve(&self, id: &str) -> Reply {
        self.call(
            Method::POST,
            &wonton_protocol::join_request_approve_path(id),
            Value::Null,
        )
        .await
    }

    async fn join(&self, username: &str, role: &str) -> iroh::EndpointId {
        let (id, device) = self.pending(username, role);
        let reply = self.approve(&id).await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
        device
    }

    async fn retry(&self) -> Reply {
        self.call(Method::POST, FAMILY_ROOM_SYNC_PATH, Value::Null)
            .await
    }

    fn reopen(&self) -> Gateway {
        use crate::plugin::PluginManager;
        use crate::plugin::docker::Engine;
        use crate::plugin::image_worker::ImageWorker;
        use crate::plugin::lifecycle::LifecycleConfig;
        use crate::plugin::store::Store;

        let dir = &self.harness.data_dir;
        let config = LifecycleConfig {
            public: self.gateway.public.clone(),
            data_dir: dir.clone(),
            apps_dir: dir.join("apps"),
            shares_dir: dir.join("shares"),
            relay_dir: dir.join("relay"),
            core_container: None,
            tz: "UTC".into(),
            puid: 1000,
            pgid: 1000,
            install_timeout: std::time::Duration::from_secs(5),
            owner: "owner".into(),
            profile: self.gateway.owner.clone(),
            gateway_port: 8080,
            plugin_slice: None,
            instance: Default::default(),
        };
        let plugins = Arc::new(PluginManager::new(
            Engine::unreachable_for_test(),
            Store::open(dir.join("plugins")).unwrap(),
            self.harness.http.clone(),
            None,
            config,
            ImageWorker::in_process(),
        ));
        Gateway::new(
            self.gateway.public.clone(),
            self.gateway.owner.clone(),
            TOKEN.to_owned(),
            Arc::new(Members::open(dir.join("members.json")).unwrap()),
            plugins,
            None,
            self.harness.http.clone(),
            self.gateway.tunnel.clone(),
        )
        .with_matrix(Some(self.matrix.clone()))
        .with_roles(Arc::new(Roles::open(dir.join("roles.json")).unwrap()))
    }

    fn room(&self) -> String {
        let state = self.fake.state();
        let rooms: Vec<_> = state
            .rooms
            .iter()
            .filter(|(id, _)| {
                let room = &state.room_state[*id];
                room.creator == self.matrix.admin_user_id()
                    && room
                        .events
                        .contains_key(&(FAMILY_ROOM_EVENT.to_owned(), String::new()))
            })
            .map(|(id, _)| id.clone())
            .collect();
        assert_eq!(rooms.len(), 1);
        rooms[0].clone()
    }

    fn assert_people(&self, room: &str, people: &[(&str, u64)]) {
        let state = self.fake.state();
        let levels =
            &state.room_state[room].events[&("m.room.power_levels".to_owned(), String::new())];
        let room = &state.rooms[room];
        let expected = std::iter::once(("@owner:test".to_owned(), json!(100))).chain(
            people
                .iter()
                .map(|(user, level)| (format!("@{user}:test"), json!(level))),
        );
        let users: serde_json::Map<String, Value> = expected.collect();
        assert_eq!(levels["users"], json!(users));
        assert_eq!(levels["users_default"], 0);
        let mut joined: BTreeSet<_> = users.keys().cloned().collect();
        joined.insert(self.matrix.admin_user_id());
        assert_eq!(room.members, joined);
        assert!(room.invited.is_empty());
        for user in users.keys() {
            assert!(
                state.devices(user).is_empty(),
                "temporary session leaked: {user}"
            );
        }
    }
}

#[tokio::test]
async fn approved_family_joins_once_and_power_levels_follow_role_assignment_and_edits() {
    let f = Family::start().await;
    let (id, _) = f.pending("alice", "family");
    assert!(f.fake.state().rooms.is_empty());
    let reply = f.approve(&id).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    let room = f.room();
    {
        let state = f.fake.state();
        let created = &state.rooms[&room];
        assert!(created.encrypted);
        assert_eq!(
            state.room_state[&room].events[&("m.room.create".to_owned(), String::new())]["room_version"],
            "12"
        );
        assert_eq!(
            state.room_state[&room].events[&(FAMILY_ROOM_EVENT.to_owned(), String::new())],
            json!({})
        );
    }
    f.assert_people(&room, &[("alice", 0)]);
    for (role, level) in [("admin", 100), ("family", 0)] {
        let reply = f
            .call(
                Method::PUT,
                &wonton_protocol::member_role_path("alice"),
                json!({"role_id": role}),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        assert_eq!(f.room(), room);
        f.assert_people(&room, &[("alice", level)]);
    }
    f.join("bob", "family").await;
    for manage_members in [true, false] {
        let reply = f
            .call(
                Method::PUT,
                &wonton_protocol::role_path("family"),
                json!({
                    "name": "Family role", "permissions": {"manage_members": manage_members}
                }),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        let level = if manage_members { 100 } else { 0 };
        f.assert_people(&room, &[("alice", level), ("bob", level)]);
    }
}

#[tokio::test]
async fn friends_stay_out_even_with_manage_members_and_removal_kicks_family() {
    let f = Family::start().await;
    f.join("guest", "friend").await;
    assert!(f.fake.state().rooms.is_empty());
    f.join("alice", "family").await;
    let room = f.room();
    let reply = f
        .call(
            Method::PUT,
            &wonton_protocol::role_path("friend"),
            json!({
                "name": "Guest manager", "permissions": {"manage_members": true}
            }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    f.assert_people(&room, &[("alice", 0)]);
    let reply = f
        .call(
            Method::PUT,
            &wonton_protocol::member_role_path("guest"),
            json!({"role_id": "family"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    f.assert_people(&room, &[("alice", 0), ("guest", 0)]);
    let reply = f
        .call(
            Method::PUT,
            &wonton_protocol::member_role_path("guest"),
            json!({"role_id": "friend"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    f.assert_people(&room, &[("alice", 0)]);
    let reply = f
        .call(
            Method::POST,
            &wonton_protocol::member_removal_path("alice"),
            json!({"delete_home": false, "keep_bot_data": true}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    f.assert_people(&room, &[]);
    assert!(f.fake.state().rooms[&room].left.contains("@alice:test"));
}

#[tokio::test]
async fn failed_creation_is_reported_and_retry_uses_saved_approval_without_reinviting() {
    let f = Family::start().await;
    let (id, _) = f.pending("alice", "family");
    f.fake.state().refuse_rooms = true;
    let reply = f.approve(&id).await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    assert_eq!(reply.error_kind(), Some("matrix_unavailable"));
    assert_eq!(
        f.gateway.members.get("alice").unwrap().status,
        MemberStatus::Joined
    );
    assert!(f.fake.state().rooms.is_empty());
    f.fake.state().refuse_rooms = false;
    let reply = f.retry().await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    let room = f.room();
    f.assert_people(&room, &[("alice", 0)]);
    let reply = f.retry().await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    assert_eq!(f.room(), room);
}

#[tokio::test]
async fn failed_role_sync_does_not_look_successful_and_next_change_reconciles_everyone() {
    let f = Family::start().await;
    f.join("alice", "family").await;
    let room = f.room();
    f.fake.state().refuse_power_levels = true;
    let reply = f
        .call(
            Method::PUT,
            &wonton_protocol::member_role_path("alice"),
            json!({"role_id": "admin"}),
        )
        .await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    assert_eq!(f.gateway.members.get("alice").unwrap().role_id, "admin");
    f.assert_people(&room, &[("alice", 0)]);
    f.fake.state().refuse_power_levels = false;
    f.join("bob", "family").await;
    f.assert_people(&room, &[("alice", 100), ("bob", 0)]);
    f.fake.state().refuse_power_levels = true;
    let reply = f
        .call(
            Method::PUT,
            &wonton_protocol::role_path("admin"),
            json!({
                "name": "No member management", "permissions": {"manage_members": false}
            }),
        )
        .await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    f.fake.state().refuse_power_levels = false;
    let reply = f.retry().await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    f.assert_people(&room, &[("alice", 0), ("bob", 0)]);
}

#[tokio::test]
async fn restart_retries_failed_removal_and_only_a_missing_room_is_recreated() {
    let f = Family::start().await;
    f.join("alice", "family").await;
    f.join("bob", "family").await;
    let room = f.room();
    f.fake.state().refuse_kicks = true;
    let reply = f
        .call(
            Method::POST,
            &wonton_protocol::member_removal_path("alice"),
            json!({"delete_home": false, "keep_bot_data": true}),
        )
        .await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    assert!(f.fake.state().rooms[&room].members.contains("@alice:test"));
    f.fake.state().refuse_kicks = false;
    let reopened = f.reopen();
    reopened.sync_family_room().await.unwrap();
    assert_eq!(f.room(), room);
    f.assert_people(&room, &[("bob", 0)]);
    f.fake.state().refuse_room_reads = true;
    let reply = f.retry().await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    assert_eq!(f.room(), room);
    f.fake.state().refuse_room_reads = false;
    f.fake.state().rooms.remove(&room);
    reopened.sync_family_room().await.unwrap();
    let replacement = f.room();
    assert_ne!(replacement, room);
    f.assert_people(&replacement, &[("bob", 0)]);
}

#[tokio::test]
async fn concurrent_approvals_share_one_room_and_retry_requires_member_management() {
    let f = Family::start().await;
    let (alice, device) = f.pending("alice", "family");
    let (bob, _) = f.pending("bob", "admin");
    let (a, b) = tokio::join!(f.approve(&alice), f.approve(&bob));
    assert_eq!(a.status, StatusCode::NO_CONTENT, "{}", a.text());
    assert_eq!(b.status, StatusCode::NO_CONTENT, "{}", b.text());
    f.assert_people(&f.room(), &[("alice", 0), ("bob", 100)]);
    let reply = send_as(
        &f.app,
        Some(device),
        Method::POST,
        CORE,
        FAMILY_ROOM_SYNC_PATH,
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.json()["missing"], "manage_members");
    let admin = f.gateway.tunnel.devices.of_user("bob")[0].id;
    let reply = send_as(
        &f.app,
        Some(admin),
        Method::POST,
        CORE,
        FAMILY_ROOM_SYNC_PATH,
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
}

#[tokio::test]
async fn a_family_marker_in_someone_elses_room_cannot_receive_family_members() {
    let f = Family::start().await;
    let stranger = f.matrix.login("stranger", "Phone").await.unwrap();
    let session = f.matrix.session(&stranger);
    let room = session
        .create_family_room("Fake family", &serde_json::Map::new())
        .await
        .unwrap();
    session
        .invite(&room, &f.matrix.admin_user_id())
        .await
        .unwrap();
    f.matrix.core_session().join(&room).await.unwrap();
    session.logout().await.unwrap();
    f.join("alice", "family").await;
    assert_ne!(f.room(), room);
    f.assert_people(&f.room(), &[("alice", 0)]);
    assert!(!f.fake.state().rooms[&room].members.contains("@alice:test"));
    assert!(!f.fake.state().rooms[&room].members.contains("@owner:test"));
}
