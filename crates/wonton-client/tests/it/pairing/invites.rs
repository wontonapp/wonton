use serde_json::json;
use wonton_client::{Action, CoreError, DeviceInvite, HomeFailure, HomeStatus};

use super::support::{Home, Phone, paired};

#[tokio::test]
async fn a_paired_phone_gets_a_code_that_adds_another_device() {
    let home = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    assert!(
        matches!(
            phone.dispatch(Action::Home {
                action: wonton_protocol::HomeAction::InviteDevice
            }),
            Err(CoreError::NotReady { .. })
        ),
        "还没配对时不能添加设备"
    );
    phone.pair(&home.code(None)).unwrap();
    phone.wait_for(paired).await;

    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::InviteDevice,
        })
        .unwrap();
    let view = phone
        .wait_for(|view| matches!(view.invite, Some(DeviceInvite::Ready { .. })))
        .await;
    let Some(DeviceInvite::Ready {
        code,
        expires_at_ms,
    }) = view.invite
    else {
        unreachable!()
    };
    let now_ms = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    assert!(expires_at_ms > now_ms + 590_000, "码 10 分钟后过期");
    // 直连地址由核心算，本设备不再把自己的连接信息传过去：请求体是空的。
    let request: serde_json::Value =
        serde_json::from_str(&home.codes.invites.lock().unwrap()[0]).unwrap();
    assert_eq!(request, json!({}));
    assert!(
        home.codes.direct_addrs[0].port() != 0,
        "假核心的码里仍有地址"
    );

    // 新设备扫这个码配上。
    let other = tempfile::tempdir().unwrap();
    let tablet = Phone::open(other.path());
    tablet.pair(code.expose()).unwrap();
    tablet.wait_for(paired).await;
}

#[tokio::test]
async fn a_refused_invite_says_why() {
    let home = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.pair(&home.code(None)).unwrap();
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    for (status, failure) in [
        (403, HomeFailure::Unauthorized),
        (503, HomeFailure::UpstreamUnavailable),
        (404, HomeFailure::Protocol),
    ] {
        *home.codes.refuse.lock().unwrap() = Some(status);
        phone
            .dispatch(Action::Home {
                action: wonton_protocol::HomeAction::InviteDevice,
            })
            .unwrap();
        let view = phone
            .wait_for(|view| matches!(view.invite, Some(DeviceInvite::Failed { .. })))
            .await;
        match view.invite {
            Some(DeviceInvite::Failed { reason, .. }) => assert_eq!(reason, failure, "{status}"),
            _ => unreachable!(),
        }
    }
}
