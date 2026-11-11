use super::*;
use crate::gateway::sharing::{AccessGuard, StreamAccess, guard_request};
use wonton_protocol::GatewayRefusal;

fn request(f: &Fixture, stream: Option<StreamAccess>) -> Request {
    let mut req = Request::builder().uri("/").body(Body::empty()).unwrap();
    req.extensions_mut().insert(f.peer.clone());
    if let Some(stream) = stream {
        req.extensions_mut().insert(stream);
    }
    req
}

fn guard(f: &Fixture) -> AccessGuard {
    let mut req = request(f, None);
    guard_request(&f.gateway, &mut req, "app.whoami").unwrap();
    req.extensions().get::<AccessGuard>().unwrap().clone()
}

fn grouped(f: &Fixture) -> String {
    let group = f.sharing.friends.save_group(None, "Friends").unwrap();
    f.sharing
        .friends
        .set_groups(&f.peer.user_id, std::slice::from_ref(&group.id))
        .unwrap();
    f.sharing
        .shares
        .create(
            &ShareGrantUpdate {
                plugin_id: "app.whoami".into(),
                recipient: ShareRecipient::Group {
                    group_id: group.id.clone(),
                },
                expires_at_ms: None,
                acknowledge_risk: true,
            },
            &f.h.manager
                .share_offer(&PluginId::parse("app.whoami").unwrap()),
        )
        .unwrap();
    group.id
}

#[tokio::test]
async fn coalesced_group_remove_readd_cannot_revive_an_upgrade() {
    let f = Fixture::new().await;
    let group = grouped(&f);
    let client = upgrade(&f).await;
    f.sharing.friends.set_groups(&f.peer.user_id, &[]).unwrap();
    f.sharing
        .friends
        .set_groups(&f.peer.user_id, &[group])
        .unwrap();
    closed(client).await;
    assert_eq!(f.get(PLUGIN, "/").await.status, StatusCode::OK);
}

#[tokio::test]
async fn an_original_direct_overlap_survives_group_epoch_changes() {
    let f = Fixture::new().await;
    let group = grouped(&f);
    let direct = f.grant(None);
    let guard = guard(&f);
    f.sharing.friends.set_groups(&f.peer.user_id, &[]).unwrap();
    f.sharing
        .friends
        .set_groups(&f.peer.user_id, &[group])
        .unwrap();
    tokio::task::yield_now().await;
    assert!(
        guard.allows(
            &f.sharing
                .authorize(&f.peer, "app.whoami", &f.h.manager)
                .unwrap()
        )
    );
    assert!(futures_util::poll!(Box::pin(guard.cancelled())).is_pending());
    f.sharing.shares.revoke(&direct.id).unwrap();
    tokio::time::timeout(Duration::from_secs(2), guard.cancelled())
        .await
        .unwrap();
    assert_eq!(f.get(PLUGIN, "/").await.status, StatusCode::OK);
}

#[tokio::test]
async fn updating_the_original_grant_resets_its_existing_upgrade() {
    let f = Fixture::new().await;
    let grant = f.grant(None);
    let client = upgrade(&f).await;
    f.sharing
        .shares
        .update(
            &grant.id,
            &ShareGrantUpdate {
                plugin_id: "app.whoami".into(),
                recipient: grant.recipient,
                expires_at_ms: None,
                acknowledge_risk: true,
            },
            &f.h.manager
                .share_offer(&PluginId::parse("app.whoami").unwrap()),
        )
        .unwrap();
    closed(client).await;
    assert_eq!(f.get(PLUGIN, "/").await.status, StatusCode::OK);
}

#[tokio::test]
async fn a_keepalive_stream_reuses_its_first_plugin_guard() {
    let f = Fixture::new().await;
    let original = f.grant(None);
    let stream = StreamAccess::new();
    let mut first = request(&f, Some(stream.clone()));
    guard_request(&f.gateway, &mut first, "app.whoami").unwrap();
    f.grant(None);
    let mut second = request(&f, Some(stream.clone()));
    guard_request(&f.gateway, &mut second, "app.whoami").unwrap();
    f.sharing.shares.revoke(&original.id).unwrap();
    let mut third = request(&f, Some(stream));
    assert_eq!(
        guard_request(&f.gateway, &mut third, "app.whoami"),
        Err(GatewayRefusal::ShareRevoked)
    );
    tokio::time::timeout(
        Duration::from_secs(2),
        second
            .extensions()
            .get::<AccessGuard>()
            .unwrap()
            .cancelled(),
    )
    .await
    .unwrap();
    assert_eq!(f.get(PLUGIN, "/").await.status, StatusCode::OK);
}

#[tokio::test]
async fn a_new_unbounded_grant_cannot_hide_expiry_of_the_last_original_permit() {
    let f = Fixture::new().await;
    let start = f.sharing.now();
    let unbounded = f.grant(None);
    f.grant(Some(start + 60_000));
    tokio::time::pause();
    let guard = guard(&f);
    f.grant(None);
    f.sharing.shares.revoke(&unbounded.id).unwrap();
    tokio::task::yield_now().await;
    f.clock
        .store(start + 59_999, std::sync::atomic::Ordering::SeqCst);
    tokio::time::advance(Duration::from_millis(59_999)).await;
    assert!(
        guard.allows(
            &f.sharing
                .authorize(&f.peer, "app.whoami", &f.h.manager)
                .unwrap()
        )
    );
    assert!(futures_util::poll!(Box::pin(guard.cancelled())).is_pending());
    f.clock
        .store(start + 60_000, std::sync::atomic::Ordering::SeqCst);
    tokio::time::advance(Duration::from_millis(1)).await;
    guard.cancelled().await;
    // The newly granted access is valid only for a new stream.
    assert!(
        f.sharing
            .authorize(&f.peer, "app.whoami", &f.h.manager)
            .is_ok()
    );
}

#[tokio::test]
async fn friend_deletion_and_plugin_uninstall_reset_existing_upgrades() {
    for uninstall in [false, true] {
        let f = Fixture::new().await;
        f.grant(None);
        let client = upgrade(&f).await;
        if uninstall {
            f.sharing.shares.revoke_plugin("app.whoami").unwrap();
        } else {
            f.sharing.friends.remove(&f.peer.user_id, 0).unwrap();
        }
        closed(client).await;
        assert_eq!(f.get(PLUGIN, "/").await.error_kind(), Some("share_revoked"));
    }
}

#[tokio::test]
async fn expiry_deletion_and_uninstall_reset_http_bodies() {
    for reason in 0..3 {
        let f = Fixture::new().await;
        let grant = f.grant(Some(f.sharing.now() + 60_000));
        let (client, hold) = streaming_http(&f).await;
        match reason {
            0 => {
                f.clock.store(
                    grant.expires_at_ms.unwrap(),
                    std::sync::atomic::Ordering::SeqCst,
                );
                f.sharing
                    .friends
                    .update_display_name(&f.peer.user_id, Some("Updated"))
                    .unwrap();
            }
            1 => {
                f.sharing.friends.remove(&f.peer.user_id, 0).unwrap();
            }
            _ => {
                f.sharing.shares.revoke_plugin("app.whoami").unwrap();
            }
        }
        closed(client).await;
        hold.notify_one();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_inflight_grant_cannot_survive_uninstall_and_reinstall() {
    let f = Fixture::new().await;
    f.h.manager
        .attach_sharing(f.sharing.friends.clone(), f.sharing.shares.clone());
    let update = ShareGrantUpdate {
        plugin_id: "app.whoami".into(),
        recipient: ShareRecipient::Friend {
            user_id: f.peer.user_id.clone(),
        },
        expires_at_ms: None,
        acknowledge_risk: true,
    };
    let manager = f.h.manager.clone();
    let shares = f.sharing.shares.clone();
    let (validated, validation) = tokio::sync::oneshot::channel();
    let (commit, commit_requested) = std::sync::mpsc::channel();
    let creating = tokio::task::spawn_blocking(move || {
        manager.with_share_offer(&PluginId::parse("app.whoami").unwrap(), |offer| {
            validated.send(()).unwrap();
            commit_requested
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            shares.create(&update, &offer)
        })
    });
    validation.await.unwrap();
    let manager = f.h.manager.clone();
    let runtime = tokio::runtime::Handle::current();
    let (started, starting) = tokio::sync::oneshot::channel();
    let mut removing = tokio::task::spawn_blocking(move || {
        started.send(()).unwrap();
        runtime.block_on(manager.uninstall(
            &PluginId::parse("app.whoami").unwrap(),
            false,
            &crate::plugin::languages::Languages::from_header(None),
        ))
    });
    starting.await.unwrap();
    let early = tokio::time::timeout(Duration::from_millis(100), &mut removing).await;
    let waited_for_commit = early.is_err();
    commit.send(()).unwrap();
    let grant = creating.await.unwrap().unwrap();
    match early {
        Ok(result) => result,
        Err(_) => removing.await,
    }
    .unwrap()
    .unwrap();
    assert!(waited_for_commit, "uninstall overtook an uncommitted grant");
    assert!(f.sharing.shares.get(&grant.id).is_none());
    f.h.manager.register_for_test(
        crate::plugin::PluginPackage::manifest_of(
            crate::gateway::tests::WHOAMI,
            &crate::plugin::languages::Languages::from_header(None),
        )
        .unwrap(),
        crate::plugin::PluginStatus::Running,
        Some(f.h.upstream.to_string()),
    );
    assert_eq!(f.get(PLUGIN, "/").await.error_kind(), Some("share_revoked"));
}
