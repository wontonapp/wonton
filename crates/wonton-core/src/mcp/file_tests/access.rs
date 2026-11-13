//! 分享与授权：主人可见性、文件变化、收回和房间授权。

use wonton_protocol::{FileFailure, FileSource};

use super::helpers::{AGENT, NOTES, any_langs, bench, images, jpeg, starts_with_kind};
use crate::mcp::tests::{failed, text};

#[tokio::test]
async fn share_references_become_ids_that_the_owner_sees_only_after_sharing() {
    let b = bench().await;
    let id = b.found("photos/2025/IMG 1.jpg", &jpeg(16)).await;
    let result = b.share(&id).await;
    assert!(!failed(&result), "{}", text(&result));
    assert!(text(&result).contains(&id.link()), "{}", text(&result));
    let info = b.owner_info(&id).await.expect("分享后主人能看");
    assert_eq!(info.name, "IMG 1.jpg");
    assert_eq!(
        info.source,
        FileSource::Bot {
            name: AGENT.into(),
            plugin: NOTES.into(),
        }
    );
    let unshared = b.found("photos/2025/IMG 2.jpg", &jpeg(16)).await;
    assert_eq!(
        b.owner_info(&unshared).await.unwrap_err(),
        FileFailure::NotShared
    );
}

#[tokio::test]
async fn view_file_says_file_changed_before_showing_text() {
    let b = bench().await;
    let id = b.found("docs/note.txt", b"one").await;
    b.f.put("docs/note.txt", b"changed");
    starts_with_kind(&b.view(&id).await, FileFailure::FileChanged);
}

#[tokio::test]
async fn files_changed_after_registering_must_be_found_again() {
    let b = bench().await;
    let id = b.found("docs/a.png", b"one").await;
    b.f.put("docs/a.png", b"changed");
    starts_with_kind(&b.view(&id).await, FileFailure::FileChanged);
    starts_with_kind(&b.share(&id).await, FileFailure::FileChanged);

    let again = b.found("docs/a.png", b"changed").await;
    assert_eq!(again, id, "同一个引用还是原来的编号");
    assert!(!failed(&b.share(&id).await));

    b.f.put("docs/a.png", b"owner edited");
    assert!(
        b.owner_info(&id).await.is_ok(),
        "主人那边不受影响：改了自己的文档，链接不该失效"
    );
}

#[tokio::test]
async fn gone_files_and_revoked_grants_are_reported() {
    let b = bench().await;
    let id = b.found("docs/a.txt", b"a").await;
    std::fs::remove_file(b.f.shares_dir.join("docs/a.txt")).unwrap();
    starts_with_kind(&b.share(&id).await, FileFailure::FileGone);

    let id = b.found("photos/a.txt", b"a").await;
    assert!(!failed(&b.share(&id).await));
    b.f.host
        .revoked_shares
        .lock()
        .unwrap()
        .push("photos".into());
    starts_with_kind(&b.share(&id).await, FileFailure::PermissionDenied);
    assert!(
        b.owner_info(&id).await.is_ok(),
        "分享以后，主人下载不再依赖来源"
    );
}

#[tokio::test]
async fn bots_cannot_touch_files_outside_their_scope() {
    let b = bench().await;
    let others = b.f.shared_with_bot("docs/theirs.txt", "someone-else");
    starts_with_kind(&b.share(&others).await, FileFailure::FileUnknown);
    let owners = b.f.shared_with_owner("docs/mine.txt", b"a");
    starts_with_kind(&b.view(&owners).await, FileFailure::FileUnknown);
}

/// App 在 Bot 的房间里发链接时调 `grants`，Bot 才能看设备传的文件。
#[tokio::test]
async fn bots_see_device_uploads_once_granted() {
    let b = bench().await;
    let id = b.f.uploaded("a.jpg", &jpeg(32));
    starts_with_kind(&b.view(&id).await, FileFailure::FileUnknown);
    b.f.host
        .rooms
        .lock()
        .unwrap()
        .insert("!bot:home".into(), vec![AGENT.into()]);
    b.f.files
        .grant_room(&id, "!bot:home", &any_langs())
        .await
        .unwrap();
    let result = b.view(&id).await;
    assert!(!failed(&result), "{}", text(&result));
    assert_eq!(images(&result), [(32, 32)]);
}

/// 主人亲手授权之后，来源插件的授权收回也不影响这个 Bot；要收回就单独收。经插件直接拿到的 Bot 仍然每次核对。
#[tokio::test]
async fn owner_grants_outlive_the_source_plugin_grant() {
    let b = bench().await;
    let id = b.found("photos/a.txt", b"a").await;
    assert!(!failed(&b.share(&id).await));
    b.f.host
        .revoked_shares
        .lock()
        .unwrap()
        .push("photos".into());
    starts_with_kind(&b.share(&id).await, FileFailure::PermissionDenied);

    b.f.host
        .rooms
        .lock()
        .unwrap()
        .insert("!bot:home".into(), vec![AGENT.into()]);
    b.f.files
        .grant_room(&id, "!bot:home", &any_langs())
        .await
        .unwrap();
    assert!(!failed(&b.share(&id).await), "主人给的，插件的授权不再算数");
    b.f.put("photos/a.txt", b"owner edited");
    assert!(!failed(&b.share(&id).await), "主人给的，读得到就行");

    b.f.files
        .revoke_bot(&id, AGENT, &any_langs())
        .await
        .unwrap();
    starts_with_kind(&b.share(&id).await, FileFailure::FileUnknown);
    assert!(b.owner_info(&id).await.is_ok(), "主人自己不受影响");

    b.f.files
        .grant_room(&id, "!bot:home", &any_langs())
        .await
        .unwrap();
    std::fs::remove_file(b.f.shares_dir.join("photos/a.txt")).unwrap();
    starts_with_kind(&b.share(&id).await, FileFailure::FileGone);
}
