//! 配对本身：扫码到同意成立、改名传过去、扫码方离线时问结果、重名拒绝、查找只认绑定的 endpoint。

use super::*;

/// 全流程：出码 → 扫码（等对方同意）→ 出码方主人同意 → 两边记成已配对、名单更新、主人身份记上。
/// 同一张长效码还能被别的服务器扫。
#[tokio::test]
async fn scan_then_approval_pairs_both_sides() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;

    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();

    // 扫码方先看到“等对方同意”。
    let row = b.row("a.example.internal").expect("扫码后应有这条记录");
    assert_eq!(row.status, PeerStatus::AwaitingApproval);
    assert_eq!(row.owner_display_name, "Alex");
    assert!(a.peers.lookup("b.example.internal").is_none(), "还没同意");

    // 出码方的请求里是扫码方自报的内容，私聊里来了一条带链接的提醒。
    let request = a.request_now("b.example.internal");
    assert_eq!(request.owner_display_name, "Alice");
    assert_eq!(request.owner_user_id, "@owner:b.example.internal");
    assert!(
        a.peers.lookup("b.example.internal").is_none(),
        "同意之前不成立"
    );
    a.wait_notices(1).await;
    let notice = a.notices().remove(0)["body"].as_str().unwrap().to_owned();
    assert!(notice.contains("wonton://settings/peers"), "{notice}");
    assert!(notice.contains("Alice"), "{notice}");

    // 主人同意：两边记成已配对。
    a.approve("b.example.internal").await.unwrap();
    let a_bound = a.wait_paired("b.example.internal").await;
    let b_bound = b.wait_paired("a.example.internal").await;
    let code_id = wonton_tunnel::peer_pair::PeerPairingCode::parse(code.code.expose())
        .unwrap()
        .server_id;
    assert_eq!(a_bound, b.id_endpoint());
    assert_eq!(b_bound, code_id);

    // 名单跟着更新（结果送出去以后），行带上主人的身份。
    a.wait_paired_names("b.example.internal").await;
    b.wait_paired_names("a.example.internal").await;
    let row = a.row("b.example.internal").unwrap();
    assert_eq!(row.owner_display_name, "Alice");
    assert_eq!(row.owner_user_id, "@owner:b.example.internal");
    assert_eq!(
        row.status,
        PeerStatus::Checking,
        "没接联邦转发组件时状态还在确认"
    );
    assert!(
        a.peers.view(&langs()).requests.is_empty(),
        "处理完的请求不再列出"
    );

    // 已经配对过：再扫同一个服务器的码，直接说清。
    b.scan_code(&code, &a).await.unwrap_err();
    assert!(a.peers.view(&langs()).requests.is_empty());
}

/// 主人改了资料再出码，对方扫到的是新名字，列表里显示的也是新名字（不是核心启动时的旧名字）。
#[tokio::test]
async fn an_owner_rename_reaches_the_peer_as_the_current_name() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;

    a.set_owner_name("改名了");
    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    let row = b.row("a.example.internal").expect("扫码后应有这条记录");
    assert_eq!(row.owner_display_name, "改名了");

    let request = a.request_now("b.example.internal");
    assert_eq!(request.owner_display_name, "Alice");
    a.approve("b.example.internal").await.unwrap();
    b.wait_paired("a.example.internal").await;
    assert_eq!(
        b.row("a.example.internal").unwrap().owner_display_name,
        "改名了"
    );
}

/// 扫码方离线时同意：上线后来问一次就成立。
#[tokio::test]
async fn a_decision_sent_while_the_scanner_is_offline_is_picked_up_by_the_poll() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;

    let code = a.peers.issue_code();
    // b 不带回连地址：a 送结果送不到，只能等 b 来问。
    b.scan_as(
        &code,
        &a,
        PeerIdentity {
            addrs: Vec::new(),
            ..b.peers.own_identity()
        },
    )
    .await
    .unwrap();
    a.request_now("b.example.internal");
    a.approve("b.example.internal").await.unwrap();

    a.wait_paired("b.example.internal").await;
    assert_eq!(
        b.row("a.example.internal").unwrap().status,
        PeerStatus::AwaitingApproval,
        "结果没送到，扫码方还停在等"
    );

    b.poll("a.example.internal").await;
    b.wait_paired("a.example.internal").await;
    assert!(b.peers.server_allowlist().contains(&a.id_endpoint()));
}

/// 同名不同 endpoint：本机还留着这个名字的记录（包括“对方已断开连接”）就回“已经配对过”。
#[tokio::test]
async fn a_taken_server_name_is_refused() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    pair(&a, &b).await;

    // 另一个 endpoint 冒充同一个服务器名。
    let impostor = TestCore::start("Fake", "b.example.internal").await;
    let code = a.peers.issue_code();
    let error = impostor.scan_code(&code, &a).await.unwrap_err();
    assert_eq!(error, PeerError::AlreadyPaired);

    // 对方删了以后，冒充的还是配不进来：这个名字的记录还在（对方已断开连接）。
    b.peers.remove("a.example.internal").await.unwrap();
    a.wait_removed("b.example.internal").await;
    let code = a.peers.issue_code();
    let error = impostor.scan_code(&code, &a).await.unwrap_err();
    assert_eq!(error, PeerError::AlreadyPaired);
}
/// 查找函数的规则：一个服务器名只会解析到配对时绑定的 endpoint ID，没配对和删掉之后都查不到。
#[tokio::test]
async fn a_server_name_resolves_only_to_its_bound_endpoint() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;

    assert!(a.peers.lookup("b.example.internal").is_none());
    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    a.request_now("b.example.internal");
    a.approve("b.example.internal").await.unwrap();
    let bound = a.wait_paired("b.example.internal").await;
    assert_eq!(
        a.peers.lookup("b.example.internal").unwrap().0,
        bound,
        "绑定的就是扫码时的 endpoint ID"
    );
    a.peers.remove("b.example.internal").await.unwrap();
    assert!(a.peers.lookup("b.example.internal").is_none());
}
