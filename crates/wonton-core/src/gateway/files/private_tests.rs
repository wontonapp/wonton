use crate::files::test_support::any_langs;
use crate::files::{new_upload_id, record::Accessor, uploads::NewUpload};
use crate::gateway::TunnelPeer;
use crate::gateway::files_tests::{Home, Reply, home};
use crate::members::MemberRecord;
use axum::body::Body;
use hyper::{Method, StatusCode};
use iroh::SecretKey;

fn member(h: &Home, username: &str) -> TunnelPeer {
    let mut record = MemberRecord::for_test(username);
    record.role_id = "family".into();
    h.members.invite(record).unwrap();
    h.members.mark_joined(username).unwrap();
    let id = SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(id, None, Some(username.into()), |_| true)
        .unwrap();
    h.f.host.people.lock().unwrap().insert(
        id.to_string(),
        (username.into(), Accessor::User(username.into())),
    );
    TunnelPeer(id)
}

async fn call(h: &Home, peer: TunnelPeer, method: Method, path: &str) -> Reply {
    h.send(
        h.request(method, path)
            .extension(peer)
            .header("tus-resumable", "1.0.0")
            .body(Body::from(r#"{"room":"!room"}"#))
            .unwrap(),
    )
    .await
}

async fn upload(h: &Home, peer: TunnelPeer, text: &str) -> wonton_protocol::FileId {
    let id = new_upload_id();
    let device = peer.0.to_string();
    h.f.files
        .create_upload(
            NewUpload {
                id: &id,
                name: Some("same.txt"),
                mime_type: None,
                size: Some(text.len() as u64),
                device: &device,
            },
            &any_langs(),
        )
        .await
        .unwrap();
    std::fs::write(h.f.uploads_dir().join(id.as_str()), text).unwrap();
    h.f.files.finish_upload(&id, &any_langs()).await.unwrap();
    id
}

async fn assert_foreign_denied(h: &Home, peer: TunnelPeer, id: &wonton_protocol::FileId) {
    for (method, suffix) in [
        (Method::GET, ""),
        (Method::GET, "/content"),
        (Method::GET, "/thumbnail"),
        (Method::POST, "/grants"),
        (Method::DELETE, "/grants/bot"),
        (Method::POST, "/send-to-devices"),
        (Method::DELETE, ""),
    ] {
        let reply = call(h, peer, method, &format!("/_wonton/files/{id}{suffix}")).await;
        assert_eq!(reply.error_kind(), "not_shared");
    }
    let reply = call(
        h,
        peer,
        Method::HEAD,
        &format!("/_wonton/files/uploads/{id}"),
    )
    .await;
    assert_eq!(reply.error_kind(), "not_shared");
    assert_eq!(reply.header("upload-length"), "");
}

async fn assert_own_mutations(h: &Home, peer: TunnelPeer, id: &wonton_protocol::FileId) {
    let sent = call(
        h,
        peer,
        Method::POST,
        &format!("/_wonton/files/{id}/send-to-devices"),
    )
    .await;
    assert_eq!(sent.status, StatusCode::NO_CONTENT);
    h.f.host
        .rooms
        .lock()
        .unwrap()
        .insert("!room".into(), vec!["helper".into()]);
    assert_eq!(
        call(
            h,
            peer,
            Method::POST,
            &format!("/_wonton/files/{id}/grants")
        )
        .await
        .status,
        StatusCode::OK
    );
    assert!(
        h.f.files
            .authorize(id, &Accessor::Bot("helper".into()), &any_langs())
            .is_ok()
    );
    assert_eq!(
        call(
            h,
            peer,
            Method::DELETE,
            &format!("/_wonton/files/{id}/grants/helper")
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    assert!(
        h.f.files
            .authorize(id, &Accessor::Bot("helper".into()), &any_langs())
            .is_err()
    );
    assert_eq!(
        call(h, peer, Method::DELETE, &format!("/_wonton/files/{id}"))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn members_and_owner_have_isolated_uploads_and_all_http_operations() {
    let h = home().await;
    let alice = member(&h, "alice");
    let bob = member(&h, "bob");
    let identities = [(alice, "alice"), (bob, "bob"), (h.device, "owner")];
    for (peer, username) in identities {
        let id = upload(&h, peer, username).await;
        let path =
            h.f.dir
                .path()
                .join(format!("core/home/{username}/transfers/same.txt"));
        let completed = call(
            &h,
            peer,
            Method::HEAD,
            &format!("/_wonton/files/uploads/{id}"),
        )
        .await;
        assert_eq!(
            completed.header("upload-length"),
            username.len().to_string()
        );
        assert_eq!(
            completed.header("upload-offset"),
            username.len().to_string()
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), username);
        assert_eq!(
            call(
                &h,
                peer,
                Method::GET,
                &format!("/_wonton/files/{id}/content")
            )
            .await
            .body,
            username.as_bytes()
        );
        for (other, _) in identities
            .into_iter()
            .filter(|(other, _)| other.0 != peer.0)
        {
            assert_foreign_denied(&h, other, &id).await;
        }
        assert_own_mutations(&h, peer, &id).await;
        assert!(
            !h.f.dir
                .path()
                .join(format!("core/home/{username}/transfers/same.txt"))
                .exists()
        );
    }
    assert!(!h.f.shares_dir.join("transfers").exists());
    let notices = h.f.host.person_notices.lock().unwrap();
    assert_eq!(
        notices
            .iter()
            .map(|(who, _)| who.clone())
            .collect::<Vec<_>>(),
        [
            Accessor::User("alice".into()),
            Accessor::User("bob".into()),
            Accessor::Owner
        ]
    );
}

#[tokio::test]
async fn private_home_permission_and_removal_deny_previously_owned_files() {
    let h = home().await;
    let alice = member(&h, "alice");
    let id = upload(&h, alice, "private").await;
    h.members.set_role("alice", "friend").unwrap();
    assert_eq!(
        call(&h, alice, Method::POST, "/_wonton/files/uploads/")
            .await
            .error_kind(),
        "not_shared"
    );
    for path in [
        format!("/_wonton/files/{id}"),
        format!("/_wonton/files/{id}/thumbnail"),
        format!("/_wonton/files/uploads/{id}"),
    ] {
        assert_eq!(
            call(&h, alice, Method::HEAD, &path).await.error_kind(),
            "not_shared"
        );
    }
    h.members.set_role("alice", "family").unwrap();
    h.members.remove("alice", false, true).unwrap();
    assert_ne!(
        call(
            &h,
            alice,
            Method::GET,
            &format!("/_wonton/files/{id}/content")
        )
        .await
        .status,
        StatusCode::OK
    );
}
