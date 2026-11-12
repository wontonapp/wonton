//! 每个房间的草稿：只在本机，换房间、重启核心都还在，发出去以后才清掉。

use std::time::Duration;

use wonton_client::Action;

use super::accept_the_bots_invite;
use super::support::{Home, Phone, Timeline, WAIT};
use super::text;

/// 读一次草稿。房间还没同步到本机时核心回错，按空处理，交给调用方的循环等。
async fn draft(phone: &Phone, room_id: &str) -> Option<String> {
    phone.draft(room_id.to_owned()).await.ok().flatten()
}

/// 等到草稿是 `want`。写入在后台排队，读到想要的为止。
async fn until_draft(phone: &Phone, room_id: &str, want: Option<&str>) {
    let waiting = async {
        loop {
            if draft(phone, room_id).await.as_deref() == want {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    if tokio::time::timeout(WAIT, waiting).await.is_err() {
        let now = draft(phone, room_id).await;
        panic!("草稿一直不是 {want:?}，现在是 {now:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn drafts_stay_per_room_and_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let first_bot = home.bot("hermes", "Hermes").await;
    let first = accept_the_bots_invite(&home, &phone, &first_bot)
        .await
        .to_string();
    let second_bot = home.bot("other", "Hermes").await;
    let second = accept_the_bots_invite(&home, &phone, &second_bot)
        .await
        .to_string();

    // 两个房间各有一份，换房间不互相盖。
    let set = |room: &str, body: &str| Action::SetDraft {
        room_id: room.to_owned(),
        body: body.to_owned(),
    };
    phone.dispatch(set(&first, "写给第一个房间")).unwrap();
    until_draft(&phone, &first, Some("写给第一个房间")).await;
    phone.dispatch(set(&second, "写给第二个房间")).unwrap();
    until_draft(&phone, &second, Some("写给第二个房间")).await;
    assert_eq!(
        draft(&phone, &first).await.as_deref(),
        Some("写给第一个房间")
    );

    // 连着敲很多次，先敲的内容不会盖掉后敲的。
    for count in 0..20 {
        phone
            .dispatch(set(&first, &format!("第一间第 {count} 个字")))
            .unwrap();
    }
    until_draft(&phone, &first, Some("第一间第 19 个字")).await;

    // 重启核心：两份草稿都还在。刚登录时房间列表可能还没同步回来，等它到。
    drop(phone);
    let phone = Phone::open(dir.path());
    phone.connect(&home);
    phone.signed_in().await;
    until_draft(&phone, &first, Some("第一间第 19 个字")).await;
    until_draft(&phone, &second, Some("写给第二个房间")).await;

    // 发出去以后清掉；别的房间不受影响。
    let mut timeline = Timeline::open(&phone, &first).await;
    phone
        .dispatch(Action::SendText {
            room_id: first.clone(),
            body: "第一间第 19 个字".to_owned(),
        })
        .unwrap();
    timeline
        .until("发出去的那句出现", |item| {
            text(item, "第一间第 19 个字")
        })
        .await;
    until_draft(&phone, &first, None).await;
    assert_eq!(
        draft(&phone, &second).await.as_deref(),
        Some("写给第二个房间"),
        "发别的房间不影响这个房间的草稿"
    );

    // 发送途中又打了字：草稿换成新的，发出去以后不能把它清掉。
    phone
        .dispatch(Action::SendText {
            room_id: first.clone(),
            body: "再发一句".to_owned(),
        })
        .unwrap();
    phone.dispatch(set(&first, "发送途中又打的")).unwrap();
    timeline
        .until("再发的那句出现", |item| text(item, "再发一句"))
        .await;
    assert_eq!(
        draft(&phone, &first).await.as_deref(),
        Some("发送途中又打的")
    );
}
