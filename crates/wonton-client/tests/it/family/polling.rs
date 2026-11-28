//! 轮询：离开页面停掉，回来又接上。

use super::*;

#[tokio::test]
async fn leaving_the_page_stops_the_refresh() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.app
        .dispatch(Action::Family {
            action: FamilyAction::LeaveFamily,
        })
        .unwrap();
    // 让走之前已经发出、和定时器已经排上的一次取完。
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    let before = rig.core.seen(&Method::GET, "/_wonton/api/members").len();
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    let after = rig.core.seen(&Method::GET, "/_wonton/api/members").len();
    assert_eq!(before, after, "离开页面后不该再取列表");
}

/// 离开页面再回来：轮询要跟着恢复，不能只靠手动刷新那一次。
#[tokio::test]
async fn coming_back_to_the_page_resumes_the_polling() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.app
        .dispatch(Action::Family {
            action: FamilyAction::LeaveFamily,
        })
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    refresh(&rig);
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let before = rig.core.seen(&Method::GET, "/_wonton/api/members").len();
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    let after = rig.core.seen(&Method::GET, "/_wonton/api/members").len();
    assert!(after > before, "回到页面后定时重取要恢复");
}
