use super::*;
use crate::files::record::Accessor;
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

fn open_family_room(bed: &Bed, members: &[&str]) {
    let mut people: std::collections::BTreeSet<String> = members
        .iter()
        .map(|name| bed.matrix.user_id(name))
        .collect();
    people.insert(bed.matrix.admin_user_id());
    bed.fake.state().rooms.insert(
        FAMILY_ROOM.into(),
        fake::Room {
            members: people,
            ..Default::default()
        },
    );
}

async fn family_device(bed: &Bed, username: &str) -> iroh::EndpointId {
    let (request, device) = bed.pending(username, false);
    let reply = bed.approve(&request).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
    device
}

async fn grant_to_family_room(bed: &Bed, id: &FileId) {
    bed.gateway
        .files
        .as_ref()
        .unwrap()
        .grant_room_for(
            Accessor::User("alice".into()),
            id,
            FAMILY_ROOM,
            &Languages::default(),
        )
        .await
        .unwrap();
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
}
