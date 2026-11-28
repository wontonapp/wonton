use super::*;

#[tokio::test]
pub(crate) async fn more_permissions_need_a_new_confirmation_and_a_moved_tag_a_new_preview() {
    let (github, docker, m, _dir) = with_github(true).await;
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    let wanting = hello("1.1.0", "", "  permissions: [network.egress]\n");
    release(&github, &wanting);
    m.check_upgrades_now().await;
    let offer = m.installed_plugin(&id(), &zh()).unwrap().upgrade.unwrap();
    assert!(
        offer.permissions_changed,
        "offered {} installed {}",
        offer.version,
        version(&m)
    );

    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Offered, None, &zh())
        .await
        .unwrap();
    assert!(preview.permissions.iter().all(|p| p.added));
    let not_confirmed = UpgradeRequest {
        devices: Vec::new(),
        source: None,
        digest: preview.digest.clone(),
        grant: Vec::new(),
        enabled_permissions: Vec::new(),
        secrets: Default::default(),
        settings: Default::default(),
        lan_webui: false,
        connect_plugins: Vec::new(),
    };
    let err = m
        .upgrade_source(&id(), &not_confirmed, &super::langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "permission_denied", "{err}");

    // 确认页开着的时候，作者把 tag 挪到了别的内容上。
    let confirmed = UpgradeRequest {
        grant: vec!["network.egress".to_owned()],
        ..not_confirmed
    };
    release(
        &github,
        &hello("1.1.0", "", "  permissions: [network.egress, models]\n"),
    );
    let err = m
        .upgrade_source(&id(), &confirmed, &super::langs())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "changed", "{err}");
    assert_eq!(version(&m), "1.0.0");

    // 能不能出网是建网络时定的：新版本要出网，网络跟着重建，核心接回去。
    release(&github, &wanting);
    upgrade(&m).await.unwrap();
    let network = "wonton-app-hello";
    assert_eq!(
        m.engine.network_reach(network).await.unwrap(),
        Some(super::super::docker::NetworkReach::Egress)
    );
    assert!(docker.endpoint(network, "wonton-core").is_some());
    assert!(m.status_of(&id()) == Some(PluginStatus::Running));
}

/// 新版本的网页端口要开关了：开关算关，确认页说清这些端口会关（ADR-0023）。
#[tokio::test]
pub(crate) async fn ports_that_become_switched_close_and_the_preview_says_so() {
    let (_docker, m, _host, _dir) = on(office()).await;
    let compose = |version: &str, webui: &str| {
        format!(
            "services:\n  web:\n    image: web:{version}\nx-wonton:\n  spec: '0.4'\n  id: app.hello\n  version: {version}\n  \
             kind: app\n  title: {{zh: 你好, en: Hello}}\n  description: {{zh: 打个招呼, en: Say hi}}\n  \
             webui: {{service: web, port: 1, {webui}}}\n  lan:\n    ports: [{{service: web, port: 1}}]\n  \
             permissions: [network.lan]\n"
        )
    };
    let port = serve_any().await;
    let v1 = compose("1.0.0", "sso: proxy, proxy: {headers: {X-Host: 'static'}}")
        .replace("port: 1", &format!("port: {port}"));
    let granted = permissions(&v1);
    m.install(&v1, &granted, super::langs()).await.unwrap();

    let v2 = compose("1.1.0", "sso: none").replace("port: 1", &format!("port: {port}"));
    let preview = m
        .preview_upgrade(
            &id(),
            UpgradeTo::Upload(Box::new(
                PluginPackage::parse(&v2, &super::langs()).unwrap(),
            )),
            None,
            &zh(),
        )
        .await
        .unwrap();
    assert!(!preview.lan_webui.unwrap().enabled);
    let closing = crate::l10n::text(
        &zh(),
        "plugin-settings-upgrade-lan-ports-closing",
        &[("ports", format!("{port}/tcp").into())],
    );
    let notice = preview.notices.iter().find(|n| n.text == closing);
    assert!(notice.is_some_and(|n| n.emphasis), "{:?}", preview.notices);
}

pub(crate) async fn serve_any() -> u16 {
    use axum::Router;
    use axum::routing::get;
    super::egress::serve(Router::new().route("/", get(|| async { "ok" })))
        .await
        .port()
}

#[tokio::test]
pub(crate) async fn always_allow_stays_through_an_upgrade_and_the_preview_lists_it() {
    let (github, _docker, m, _dir) = with_github(true).await;
    let bot = "  permissions: [tools]\n";
    release(&github, &hello("1.0.0", "", bot));
    install(&m, SOURCE).await;
    let policy = m.tool_policy();
    policy.set_always("app.hello", "app.hello", true).unwrap();
    release(&github, &hello("1.1.0", "", bot));
    m.check_upgrades_now().await;

    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Offered, None, &zh())
        .await
        .unwrap();
    let listed: Vec<_> = preview.always_allowed.iter().map(|b| &b.caller).collect();
    assert_eq!(listed, ["app.hello"]);

    upgrade(&m).await.unwrap();
    assert_eq!(version(&m), "1.1.0");
    assert!(policy.is_always("app.hello", "app.hello"));

    policy.set_always("app.hello", "app.hello", false).unwrap();
    release(&github, &hello("1.2.0", "", bot));
    m.check_upgrades_now().await;
    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Offered, None, &zh())
        .await
        .unwrap();
    assert!(preview.always_allowed.is_empty());
}

/// widget 能力变多：新版本的提示要主人重新确认（和权限变多是同一条路），预览里标出多出来的那项。
#[tokio::test]
pub(crate) async fn more_widget_capabilities_ask_the_owner_again() {
    // 网页入口的健康检查真的连这个端口，要有东西听着。
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, axum::Router::new().fallback(|| async { "ok" }))
            .await
            .unwrap();
    });
    let widget = |capabilities: &[&str]| {
        let mut yaml = format!(
            "  permissions: []\n  webui: {{service: web, port: {port}}}\n  matrix_widget:\n    url: /room\n    capabilities:\n"
        );
        for capability in capabilities {
            yaml.push_str(&format!("      - {capability}\n"));
        }
        yaml
    };
    let transports = "org.matrix.msc4515.rtc_transports";
    let messages = "org.matrix.msc2762.receive.event:m.room.message";
    let (github, _docker, m, _dir) = with_github(false).await;
    release(&github, &hello("1.0.0", "", &widget(&[transports])));
    install(&m, SOURCE).await;
    // App 按已装列表里的声明在房间里打开 widget、批能力。
    assert_eq!(
        m.installed_plugin(&id(), &zh()).unwrap().matrix_widget,
        Some(wonton_protocol::PluginWidget {
            url: "/room".to_owned(),
            entry: wonton_protocol::WidgetEntry::Other,
            capabilities: vec![transports.to_owned()],
        })
    );

    release(&github, &hello("1.1.0", "", &widget(&[transports])));
    assert_eq!(m.check_upgrades_now().await, None);
    assert!(
        !m.installed_plugin(&id(), &zh())
            .unwrap()
            .upgrade
            .unwrap()
            .permissions_changed,
        "能力没变不用再确认"
    );

    release(
        &github,
        &hello("1.2.0", "", &widget(&[transports, messages])),
    );
    assert_eq!(m.check_upgrades_now().await, None);
    let offer = m.installed_plugin(&id(), &zh()).unwrap().upgrade.unwrap();
    assert!(
        offer.permissions_changed,
        "offered {} installed {}",
        offer.version,
        version(&m)
    );

    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Offered, None, &zh())
        .await
        .unwrap();
    let added: Vec<_> = preview
        .widget_capabilities
        .iter()
        .filter(|capability| capability.added)
        .collect();
    assert_eq!(added.len(), 1, "{:?}", preview.widget_capabilities);
    assert_eq!(added[0].capability, messages);
    assert_eq!(added[0].level, wonton_protocol::WidgetCapabilityLevel::High);
    assert!(
        preview.notices.iter().any(|notice| notice.emphasis),
        "{:?}",
        preview.notices
    );

    upgrade(&m).await.unwrap();
    assert_eq!(version(&m), "1.2.0");
}
