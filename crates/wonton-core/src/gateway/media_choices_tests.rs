//! 给壳挑的两张名单：`GET /sources` 列的候选和选提供方的规则对得上，`GET /share-targets` 给任何已加入的家人。

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{StatusCode, header};
use hyper::{Method, Request};
use iroh::SecretKey;
use serde_json::{Value, json};
use wonton_protocol::media::{MediaKind, MediaOperation, WONTON_DEVICE_HEADER};
use wonton_protocol::{AppScope, RoleDraft};

use super::files_tests::paired_gateway;
use super::tests::CORE;
use super::{Gateway, TunnelPeer, router};
use crate::http::HttpClient;
use crate::media::Media;
use crate::media::provider::Candidate;
use crate::media::test_support::{FakeDirectory, candidate};
use crate::members::MemberRecord;
use crate::plugin::upstreams::Endpoint;

const PHOTOS: &str = "app.example.photos";
const ALBUMS: &str = "app.example.albums";
const MOVIES: &str = "app.example.movies";
const LIBRARY: &str = "/_wonton/api/media/sources?operation=library&kind=photo";
const SHARE_TARGETS: &str = "/_wonton/api/media/share-targets";

struct Bed {
    _dir: tempfile::TempDir,
    gateway: Arc<Gateway>,
    app: Router,
    owner: TunnelPeer,
}

fn photos(id: &str) -> Candidate {
    candidate(id, &[MediaOperation::Library], &[MediaKind::Photo])
}

fn movies() -> Candidate {
    candidate(
        MOVIES,
        &[MediaOperation::Playback, MediaOperation::Library],
        &[MediaKind::Video],
    )
}

/// 列名单不连提供方，地址随便给一个。
async fn bed(candidates: Vec<Candidate>) -> Bed {
    let dir = tempfile::TempDir::new().unwrap();
    let paired = paired_gateway(dir.path()).await;
    let unused = || Endpoint::Tcp("127.0.0.1:9".into());
    let (directory, _) =
        FakeDirectory::new(candidates.into_iter().map(|c| (c, unused())).collect());
    let media = Media::with_directory(
        &dir.path().join("media"),
        Arc::new(directory),
        HttpClient::new(),
    )
    .unwrap();
    let gateway = Arc::new(paired.gateway.with_media(Some(Arc::new(media))));
    Bed {
        app: router(gateway.clone()),
        gateway,
        owner: paired.device,
        _dir: dir,
    }
}

impl Bed {
    /// 一个角色是 `role` 的已加入家人和他的一台设备。
    fn joined(&self, username: &str, role: &str) -> TunnelPeer {
        self.invited(username, role);
        self.gateway.members.mark_joined(username).unwrap();
        let device = SecretKey::generate().public();
        self.gateway
            .tunnel
            .devices
            .add_if(device, None, Some(username.to_owned()), |_| true)
            .unwrap();
        TunnelPeer(device)
    }

    fn invited(&self, username: &str, role: &str) {
        let record = MemberRecord {
            role_id: role.to_owned(),
            ..MemberRecord::for_test(username)
        };
        self.gateway.members.invite(record).unwrap();
    }

    /// `role` 只能打开 `apps`，不能管理成员。
    fn restrict(&self, role: &str, apps: &[&str]) {
        let record = self.gateway.roles.get(role).unwrap();
        let mut permissions = record.permissions.clone();
        permissions.open_apps = AppScope::Selected {
            plugin_ids: apps.iter().map(|id| (*id).to_owned()).collect(),
        };
        permissions.manage_members = false;
        let draft = RoleDraft {
            name: role.to_owned(),
            permissions,
        };
        self.gateway.roles.update(role, &draft).unwrap();
    }

    async fn get(&self, peer: TunnelPeer, path: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(Method::GET)
            .uri(path)
            .header(header::HOST, format!("{CORE}:18480"))
            .header(header::ACCEPT_LANGUAGE, "en")
            .header(WONTON_DEVICE_HEADER, crate::files::device_label(&peer.0))
            .extension(peer)
            .body(Body::empty())
            .unwrap();
        let response = tower::ServiceExt::oneshot(self.app.clone(), request)
            .await
            .unwrap();
        let status = response.status();
        let body = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        (status, serde_json::from_slice(&body).unwrap())
    }

    async fn sources(&self, peer: TunnelPeer, path: &str) -> Vec<(String, bool)> {
        let (status, body) = self.get(peer, path).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                (
                    s["id"].as_str().unwrap().to_owned(),
                    s["is_default"] == true,
                )
            })
            .collect()
    }

    async fn people(&self, peer: TunnelPeer) -> Vec<String> {
        let (status, body) = self.get(peer, SHARE_TARGETS).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["people"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["user_id"].as_str().unwrap().to_owned())
            .collect()
    }
}

fn mxid(username: &str) -> String {
    format!("@{username}:{CORE}")
}

#[tokio::test]
async fn no_photo_source_is_an_empty_list_not_a_failure() {
    let bed = bed(vec![movies()]).await;
    assert!(bed.sources(bed.owner, LIBRARY).await.is_empty());
}

#[tokio::test]
async fn a_single_source_is_listed_with_its_title() {
    let bed = bed(vec![photos(PHOTOS), movies()]).await;
    let (status, body) = bed.get(bed.owner, LIBRARY).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"sources": [{"id": PHOTOS, "title": PHOTOS.to_uppercase(), "is_default": false}]})
    );
}

#[tokio::test]
async fn several_sources_are_the_ones_the_core_would_ask_to_choose_between() {
    let bed = bed(vec![photos(PHOTOS), photos(ALBUMS), movies()]).await;
    assert_eq!(
        bed.sources(bed.owner, LIBRARY).await,
        [(PHOTOS.to_owned(), false), (ALBUMS.to_owned(), false)]
    );
    // 同一组候选，没点名来源时核心要人挑。
    let (status, failure) = bed.get(bed.owner, "/_wonton/api/media/library").await;
    assert_eq!(
        (status, failure["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("source_required"))
    );
    // 种类跟着查询走：只有影视那家有视频。
    assert_eq!(
        bed.sources(
            bed.owner,
            "/_wonton/api/media/sources?operation=library&kind=video"
        )
        .await,
        [(MOVIES.to_owned(), false)]
    );
}

#[tokio::test]
async fn the_owners_default_is_marked() {
    let mut albums = photos(ALBUMS);
    albums.default = true;
    let bed = bed(vec![photos(PHOTOS), albums]).await;
    assert_eq!(
        bed.sources(bed.owner, LIBRARY).await,
        [(PHOTOS.to_owned(), false), (ALBUMS.to_owned(), true)]
    );
}

#[tokio::test]
async fn sources_a_person_cannot_open_are_not_listed() {
    let bed = bed(vec![photos(PHOTOS), photos(ALBUMS)]).await;
    bed.restrict("child", &[ALBUMS]);
    let kid = bed.joined("kid", "child");
    assert_eq!(
        bed.sources(kid, LIBRARY).await,
        [(ALBUMS.to_owned(), false)]
    );
    bed.restrict("child", &[]);
    assert!(bed.sources(kid, LIBRARY).await.is_empty());
}

#[tokio::test]
async fn unknown_operations_list_nothing_and_a_missing_one_is_a_protocol_failure() {
    let bed = bed(vec![photos(PHOTOS)]).await;
    assert!(
        bed.sources(bed.owner, "/_wonton/api/media/sources?operation=transcode")
            .await
            .is_empty()
    );
    let (status, failure) = bed
        .get(bed.owner, "/_wonton/api/media/sources?kind=photo")
        .await;
    assert_eq!(
        (status, failure["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("protocol"))
    );
}

#[tokio::test]
async fn share_targets_are_the_owner_and_joined_members_but_not_the_caller() {
    let bed = bed(Vec::new()).await;
    bed.restrict("child", &[]);
    let kid = bed.joined("kid", "child");
    bed.joined("mia", "family");
    bed.invited("ann", "family");
    bed.joined("leo", "family");
    bed.gateway.members.remove("leo", false, false).unwrap();

    // 不能管理成员、一个应用都打不开的家人也能取。
    assert_eq!(bed.people(kid).await, [mxid("owner"), mxid("mia")]);
    assert_eq!(bed.people(bed.owner).await, [mxid("kid"), mxid("mia")]);
    let (_, body) = bed.get(kid, SHARE_TARGETS).await;
    assert_eq!(
        body["people"][1],
        json!({"user_id": mxid("mia"), "display_name": "mia"})
    );
}
