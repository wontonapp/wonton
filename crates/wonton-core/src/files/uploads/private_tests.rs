use super::{NewUpload, UPLOAD_TTL_SECS};
use crate::files::record::{Accessor, Lookup};
use crate::files::test_support::{Fixture, any_langs};
use crate::plugin::manager::now_secs;
use std::fs;

#[tokio::test]
async fn retry_and_expiry_keep_each_person_in_their_home() {
    let f = Fixture::new();
    let mut ids = Vec::new();
    for username in ["alice", "bob", "owner"] {
        let who = if username == "owner" {
            Accessor::Owner
        } else {
            Accessor::User(username.into())
        };
        f.host
            .people
            .lock()
            .unwrap()
            .insert(username.into(), (username.into(), who.clone()));
        let id = crate::files::new_upload_id();
        f.files
            .create_upload(
                NewUpload {
                    id: &id,
                    name: Some("same.txt"),
                    mime_type: None,
                    size: Some(5),
                    device: username,
                },
                &any_langs(),
            )
            .await
            .unwrap();
        fs::write(f.uploads_dir().join(id.as_str()), "hello").unwrap();
        let pending = f.files.uploads.load_pending(&id).unwrap().unwrap();
        f.files.place_marked(&pending, &any_langs()).unwrap();
        ids.push((id, username, who));
    }
    let files = f.reopen();
    for (id, username, who) in &ids {
        for _ in 0..2 {
            let record = files.finish_upload(id, &any_langs()).await.unwrap();
            assert_eq!(record.access, [who.clone()].into());
        }
        let home = f.dir.path().join(format!("core/home/{username}/transfers"));
        assert_eq!(fs::read_to_string(home.join("same.txt")).unwrap(), "hello");
        assert!(!home.join("same (1).txt").exists());
        assert!(!f.uploads_dir().join(id.as_str()).exists());
        assert!(files.uploads.load_pending(id).unwrap().is_none());
    }
    files.sweep(now_secs() + UPLOAD_TTL_SECS + 1).await;
    for (id, username, _) in ids {
        assert!(
            !f.dir
                .path()
                .join(format!("core/home/{username}/transfers/same.txt"))
                .exists()
        );
        assert!(matches!(
            files.registry.get(&id, now_secs()),
            Lookup::Unknown
        ));
    }
    assert!(!f.shares_dir.join("transfers").exists());
}

#[tokio::test]
async fn private_bot_share_resolves_keeper_and_never_grants_server_owner() {
    let f = Fixture::new();
    let id = f.shared_with_bot("photos/secret.txt", "private-bot");
    let alice = Accessor::User("alice".into());
    f.host
        .keepers
        .lock()
        .unwrap()
        .insert("private-bot".into(), alice.clone());
    let record = f
        .files
        .share_with_owner(&id, "private-bot", &any_langs())
        .await
        .unwrap();
    assert!(record.access.contains(&alice));
    assert!(!record.access.contains(&Accessor::Owner));
    assert_eq!(
        f.files
            .info(&id, &Accessor::Owner, &any_langs())
            .await
            .unwrap_err()
            .kind,
        wonton_protocol::FileFailure::NotShared
    );
    f.host.denied.lock().unwrap().push(alice.clone());
    assert_eq!(
        f.files
            .info(&id, &alice, &any_langs())
            .await
            .unwrap_err()
            .kind,
        wonton_protocol::FileFailure::NotShared
    );
}

#[tokio::test]
async fn revoked_person_cannot_create_or_finish_uploads_or_recreate_home() {
    let f = Fixture::new();
    let alice = Accessor::User("alice".into());
    f.host
        .people
        .lock()
        .unwrap()
        .insert("phone".into(), ("alice".into(), alice.clone()));
    let id = crate::files::new_upload_id();
    let request = NewUpload {
        id: &id,
        name: Some("private.txt"),
        mime_type: None,
        size: Some(5),
        device: "phone",
    };
    f.files.create_upload(request, &any_langs()).await.unwrap();
    fs::write(f.uploads_dir().join(id.as_str()), "hello").unwrap();
    f.host.denied.lock().unwrap().push(alice);
    crate::private_home::remove(f.dir.path(), "alice").unwrap();
    assert!(f.files.create_upload(request, &any_langs()).await.is_err());
    assert!(f.files.finish_upload(&id, &any_langs()).await.is_err());
    f.files.sweep(now_secs()).await;
    assert!(!f.dir.path().join("core/home/alice").exists());
    let turn = f.files.upload_turn(&id).await;
    f.files.upload_cancelled(&turn).await;
    assert!(f.files.uploads.load_pending(&id).unwrap().is_none());
    assert!(!f.uploads_dir().join(id.as_str()).exists());
}

#[tokio::test]
async fn invalid_trusted_username_cannot_escape_the_home_root() {
    let f = Fixture::new();
    f.host.people.lock().unwrap().insert(
        "phone".into(),
        ("../outside".into(), Accessor::User("../outside".into())),
    );
    let id = crate::files::new_upload_id();
    let request = NewUpload {
        id: &id,
        name: Some("private.txt"),
        mime_type: None,
        size: Some(5),
        device: "phone",
    };
    assert!(f.files.create_upload(request, &any_langs()).await.is_err());
    assert!(f.files.uploads.load_pending(&id).unwrap().is_none());
    assert!(!f.dir.path().join("core/outside").exists());
}
