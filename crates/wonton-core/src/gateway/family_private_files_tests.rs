use super::*;
use crate::files::record::{Accessor, Origin};
use crate::files::uploads::NewUpload;
use crate::files::{CoreHost, Files, FilesConfig, new_upload_id};
use crate::mcp::audit::Audit;
use wonton_protocol::FileId;

pub(super) fn service(gateway: &Gateway) -> Arc<Files> {
    let root = gateway.plugins.data_dir();
    let host = CoreHost::new(
        gateway.plugins.clone(),
        Arc::new(crate::mcp::upstream::Upstreams::new()),
        None,
    )
    .with_people(
        gateway.members.clone(),
        gateway.roles.clone(),
        gateway.tunnel.devices.clone(),
        "owner".into(),
        gateway.matrix.clone(),
        root.join("core"),
    );
    Arc::new(
        Files::open(
            &FilesConfig {
                data_dir: root.clone(),
                dir: root.join("core/files"),
                shares_dir: root.join("shares"),
                owner: gateway.plugins.private_home_owner(),
            },
            Arc::new(host),
            HttpClient::new(),
            Arc::new(Audit::new(root.join("core/audit.jsonl"))),
            Arc::new(ImageWorker::in_process()),
        )
        .unwrap(),
    )
}

async fn upload(bed: &Bed, device: iroh::EndpointId, text: &str) -> FileId {
    let files = bed.gateway.files.as_ref().unwrap();
    let id = new_upload_id();
    let langs = Languages::default();
    files
        .create_upload(
            NewUpload {
                id: &id,
                name: Some("shared.txt"),
                mime_type: Some("text/plain"),
                size: Some(text.len() as u64),
                device: &device.to_string(),
            },
            &langs,
        )
        .await
        .unwrap();
    fs::write(
        bed.dir.path().join("core/files/uploads").join(id.as_str()),
        text,
    )
    .unwrap();
    files.finish_upload(&id, &langs).await.unwrap();
    id
}

async fn shared_upload(bed: &Bed, username: &str, text: &str) -> FileId {
    let (request, device) = bed.pending(username, false);
    let reply = bed.approve(&request).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    let id = upload(bed, device, text).await;
    let files = bed.gateway.files.as_ref().unwrap();
    let langs = Languages::default();
    let bot = bed
        .gateway
        .plugins
        .bot_account(&PluginId::parse(AGENT).unwrap(), "default")
        .unwrap();
    files
        .grant_room_for(Accessor::User(username.into()), &id, &bot.home_room, &langs)
        .await
        .unwrap();
    let localpart = bot
        .user_id
        .strip_prefix('@')
        .unwrap()
        .split_once(':')
        .unwrap()
        .0;
    files
        .share_with_owner(&id, localpart, &langs)
        .await
        .unwrap();
    id
}

async fn file_reply(bed: &Bed, device: iroh::EndpointId, id: &FileId, suffix: &str) -> Reply {
    send_as(
        &router(bed.gateway.clone()),
        Some(device),
        Method::GET,
        CORE,
        &format!("/_wonton/files/{id}{suffix}"),
        &[("x-wonton-files", "1")],
        "",
    )
    .await
}

async fn assert_content(bed: &Bed, device: iroh::EndpointId, id: &FileId, text: &str) {
    let reply = file_reply(bed, device, id, "").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let reply = file_reply(bed, device, id, "/content").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.text(), text);
}

async fn assert_unknown(bed: &Bed, device: iroh::EndpointId, id: &FileId) {
    for suffix in ["", "/content", "/thumbnail"] {
        let reply = file_reply(bed, device, id, suffix).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{}", reply.text());
        assert_eq!(reply.error_kind(), Some("file_unknown"), "{}", reply.text());
    }
}

async fn remove(bed: &Bed, delete_home: bool) -> Reply {
    bed.call(
        Method::POST,
        &wonton_protocol::member_removal_path("alice"),
        json!({"delete_home": delete_home, "keep_bot_data": true}),
    )
    .await
}

#[tokio::test]
async fn deleting_home_forgets_shared_uploads_but_retaining_home_keeps_links_usable() {
    let mut bed = Bed::start().await;
    let owner = SecretKey::generate().public();
    bed.gateway.tunnel.devices.add(owner, None).unwrap();
    let alice = shared_upload(&bed, "alice", "alice shared content").await;
    let bob = shared_upload(&bed, "bob", "bob shared content").await;
    assert_content(&bed, owner, &alice, "alice shared content").await;
    let reply = remove(&bed, false).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    bed.reopen().await;
    assert_content(&bed, owner, &alice, "alice shared content").await;
    assert_content(&bed, owner, &bob, "bob shared content").await;
    let reply = remove(&bed, true).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_unknown(&bed, owner, &alice).await;
    assert_content(&bed, owner, &bob, "bob shared content").await;
    bed.reopen().await;
    assert_unknown(&bed, owner, &alice).await;
    assert_content(&bed, owner, &bob, "bob shared content").await;
}

#[tokio::test]
async fn failed_file_record_cleanup_is_visible_and_resumes_after_restart() {
    let mut bed = Bed::start().await;
    let owner = SecretKey::generate().public();
    bed.gateway.tunnel.devices.add(owner, None).unwrap();
    let id = shared_upload(&bed, "alice", "shared before removal").await;
    assert_content(&bed, owner, &id, "shared before removal").await;
    let path = bed
        .dir
        .path()
        .join("core/files/records")
        .join(format!("{id}.json"));
    let record = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let reply = remove(&bed, true).await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    fs::remove_dir(&path).unwrap();
    fs::write(&path, record).unwrap();
    bed.reopen().await;
    let reply = bed.retry().await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    assert_unknown(&bed, owner, &id).await;
    bed.reopen().await;
    assert_unknown(&bed, owner, &id).await;
}

const FAMILY_ROOM: &str = "!family:test";

fn open_room(bed: &Bed, room: &str, members: &[&str], with_core: bool) {
    let mut people: std::collections::BTreeSet<String> = members
        .iter()
        .map(|name| bed.matrix.user_id(name))
        .collect();
    if with_core {
        people.insert(bed.matrix.admin_user_id());
    }
    bed.fake.state().rooms.insert(
        room.into(),
        fake::Room {
            members: people,
            ..Default::default()
        },
    );
}

fn open_family_room(bed: &Bed, members: &[&str]) {
    open_room(bed, FAMILY_ROOM, members, true);
}

async fn family_device(bed: &Bed, username: &str) -> iroh::EndpointId {
    let (request, device) = bed.pending(username, false);
    let reply = bed.approve(&request).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    device
}

async fn grant_to_room(bed: &Bed, id: &FileId, room: &str) {
    bed.gateway
        .files
        .as_ref()
        .unwrap()
        .grant_room_for(
            Accessor::User("alice".into()),
            id,
            room,
            &Languages::default(),
        )
        .await
        .unwrap();
}

async fn grant_to_family_room(bed: &Bed, id: &FileId) {
    grant_to_room(bed, id, FAMILY_ROOM).await;
}

#[tokio::test]
async fn family_room_members_open_posted_files_and_removed_ones_do_not() {
    let bed = Bed::start().await;
    let alice = family_device(&bed, "alice").await;
    let bob = family_device(&bed, "bob").await;
    let carol = family_device(&bed, "carol").await;
    let owner = SecretKey::generate().public();
    bed.gateway.tunnel.devices.add(owner, None).unwrap();
    open_family_room(&bed, &["alice", "bob", "owner"]);
    let id = upload(&bed, alice, "family bill").await;
    grant_to_family_room(&bed, &id).await;

    let direct = bed
        .gateway
        .files
        .as_ref()
        .unwrap()
        .authorize(&id, &Accessor::User("bob".into()), &Languages::default())
        .await;
    assert!(direct.is_ok(), "{:?}", direct.err());
    assert_content(&bed, bob, &id, "family bill").await;
    assert_content(&bed, owner, &id, "family bill").await;

    let reply = file_reply(&bed, carol, &id, "").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("not_shared"));

    bed.fake
        .state()
        .rooms
        .get_mut(FAMILY_ROOM)
        .unwrap()
        .members
        .remove(&bed.matrix.user_id("bob"));
    let reply = file_reply(&bed, bob, &id, "").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("not_shared"));
}

#[tokio::test]
async fn room_membership_that_cannot_be_read_is_service_unavailable() {
    let bed = Bed::start().await;
    let alice = family_device(&bed, "alice").await;
    let bob = family_device(&bed, "bob").await;
    open_family_room(&bed, &["alice", "bob"]);
    let id = upload(&bed, alice, "family bill").await;
    grant_to_family_room(&bed, &id).await;

    bed.fake.state().refuse_room_reads = true;
    let reply = file_reply(&bed, bob, &id, "").await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    assert_eq!(reply.error_kind(), Some("service_unavailable"));
    assert!(
        reply
            .text()
            .contains("Can't check who's in the room right now"),
        "{}",
        reply.text()
    );
}

const OTHER_ROOM: &str = "!other:test";

#[tokio::test]
async fn a_room_without_the_core_is_skipped_not_unknown() {
    let bed = Bed::start().await;
    let alice = family_device(&bed, "alice").await;
    let bob = family_device(&bed, "bob").await;
    open_room(&bed, FAMILY_ROOM, &["bob"], false);
    let id = upload(&bed, alice, "family bill").await;
    grant_to_family_room(&bed, &id).await;

    let reply = file_reply(&bed, bob, &id, "").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("not_shared"));
}

#[tokio::test]
async fn a_deleted_room_is_skipped_not_unknown() {
    let bed = Bed::start().await;
    let alice = family_device(&bed, "alice").await;
    let bob = family_device(&bed, "bob").await;
    open_family_room(&bed, &["alice", "bob"]);
    let id = upload(&bed, alice, "family bill").await;
    grant_to_family_room(&bed, &id).await;
    assert_content(&bed, bob, &id, "family bill").await;

    bed.fake.state().rooms.remove(FAMILY_ROOM);
    let reply = file_reply(&bed, bob, &id, "").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("not_shared"));
}

#[tokio::test]
async fn a_later_room_member_still_opens_when_an_earlier_room_is_unreadable() {
    // 房间按 BTreeSet 的字典序查：`!family:test` 在前（查不动），`!other:test` 在后（有答案）。
    let bed = Bed::start().await;
    let alice = family_device(&bed, "alice").await;
    let bob = family_device(&bed, "bob").await;
    open_family_room(&bed, &["alice", "bob"]);
    open_room(&bed, OTHER_ROOM, &["alice", "bob"], true);
    bed.fake.state().broken_rooms.insert(FAMILY_ROOM.into());
    let id = upload(&bed, alice, "family bill").await;
    grant_to_family_room(&bed, &id).await;
    grant_to_room(&bed, &id, OTHER_ROOM).await;

    assert_content(&bed, bob, &id, "family bill").await;
}

#[tokio::test]
async fn room_membership_survives_restart() {
    let mut bed = Bed::start().await;
    let alice = family_device(&bed, "alice").await;
    let bob = family_device(&bed, "bob").await;
    open_family_room(&bed, &["alice", "bob"]);
    let id = upload(&bed, alice, "family bill").await;
    grant_to_family_room(&bed, &id).await;

    // bob 只在房间成员里、不在显式 access：重启后只能靠落盘的 rooms 放行。
    // 若 rooms 没落盘（比如 serde skip），这里会静默退回 403。
    bed.reopen().await;
    assert_content(&bed, bob, &id, "family bill").await;
}

#[tokio::test]
async fn a_removed_member_loses_room_access() {
    let mut bed = Bed::start().await;
    let alice = family_device(&bed, "alice").await;
    let bob = family_device(&bed, "bob").await;
    open_family_room(&bed, &["alice", "bob"]);
    let id = upload(&bed, alice, "family bill").await;
    grant_to_family_room(&bed, &id).await;
    assert_content(&bed, bob, &id, "family bill").await;

    // 只改记录状态，不吊销设备：这里测的是成员身份，不是设备没了。
    let path = bed.dir.path().join("members.json");
    let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for member in stored["members"].as_array_mut().unwrap() {
        if member["username"] == "bob" {
            member["status"] = json!("Removed");
        }
    }
    fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    bed.reopen().await;
    let reply = file_reply(&bed, bob, &id, "").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("not_shared"));
}

#[tokio::test]
async fn a_room_member_deletes_the_file_for_everyone() {
    let bed = Bed::start().await;
    let alice = family_device(&bed, "alice").await;
    let bob = family_device(&bed, "bob").await;
    open_family_room(&bed, &["alice", "bob"]);
    let id = upload(&bed, alice, "family bill").await;
    grant_to_family_room(&bed, &id).await;
    assert_content(&bed, bob, &id, "family bill").await;
    let record = bed
        .gateway
        .files
        .as_ref()
        .unwrap()
        .authorize(&id, &Accessor::User("bob".into()), &Languages::default())
        .await
        .unwrap();
    let Origin::Upload { reference, .. } = &record.origin else {
        panic!("应是上传的文件");
    };
    let placed = bed
        .home("alice")
        .join("transfers")
        .join(reference.path.last().unwrap());
    assert!(placed.exists());

    let reply = send_as(
        &router(bed.gateway.clone()),
        Some(bob),
        Method::DELETE,
        CORE,
        &format!("/_wonton/files/{id}"),
        &[("x-wonton-files", "1")],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    assert_unknown(&bed, alice, &id).await;
    assert_unknown(&bed, bob, &id).await;
    assert!(!placed.exists(), "文件也一起删了");
}
