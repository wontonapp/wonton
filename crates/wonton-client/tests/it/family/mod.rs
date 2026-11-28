//! 成员页（ADR-0088）的行为测试：只经导出给原生壳的接口驱动，隧道的对面是本机假扮的核心。
//!
//! 假核心按真核心的规矩记账：邀请记成员和码，批准把设备登记到人名下，拒绝连同码作废，移除改状态。
//! 断言看 ViewModel，也看假核心收到的请求。

mod access;
mod failures;
mod invites;
mod members;
mod requests;
mod roles;

use axum::http::Method;
use wonton_client::{Action, FamilyFailure, FamilyProblem, FamilyViewModel, MemberStatus};
use wonton_protocol::{FamilyAction, MemberInvite, PermissionItem, RoleDraft, RoleTemplate};

use crate::plugins::rig::{Headered, Rig, WAIT};

async fn family(
    rig: &Rig,
    what: &str,
    mut check: impl FnMut(&FamilyViewModel) -> bool,
) -> FamilyViewModel {
    let subscription = rig.app.subscribe_family();
    let waiting = async {
        while let Some(view) = subscription.next().await {
            if check(&view) {
                return view;
            }
        }
        panic!("订阅提前结束");
    };
    tokio::time::timeout(WAIT, waiting)
        .await
        .unwrap_or_else(|_| panic!("{what}：超时，当前：{:#?}", rig.app.family()))
}

fn refresh(rig: &Rig) {
    rig.app
        .dispatch(Action::Family {
            action: FamilyAction::RefreshFamily,
        })
        .unwrap();
}

fn send(rig: &Rig, action: FamilyAction) {
    rig.app.dispatch(Action::Family { action }).unwrap();
}

fn invite(rig: &Rig, name: &str, role: Option<&str>) {
    rig.app
        .dispatch(Action::Family {
            action: FamilyAction::InviteMember {
                display_name: name.to_owned(),
                username: None,
                private_bot: false,
                role_id: role.map(str::to_owned),
            },
        })
        .unwrap();
}

async fn owner_ready(rig: &Rig) {
    rig.core.family.owner();
    rig.connected().await;
    refresh(rig);
    family(rig, "成员页就绪", |view| {
        view.access.is_owner && !view.roles.roles.is_empty()
    })
    .await;
}

fn storage_failure() -> Headered {
    Headered(
        500,
        vec![("x-wonton-error".to_owned(), "storage".to_owned())],
        "{\"error\":{\"kind\":\"storage\",\"message\":\"盘满了\"}}".to_owned(),
    )
}

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
