use super::*;

fn entry_id() -> crate::plugin::PluginId {
    crate::plugin::PluginId::parse("io.wonton.test.entry").unwrap()
}

fn app_container() -> String {
    crate::plugin::names::Instance::default().container(&entry_id().slug(), "app")
}

async fn reached(hold: &crate::plugin::fake_docker::Hold) {
    tokio::time::timeout(Duration::from_secs(5), hold.reached())
        .await
        .expect("the request must reach the controlled Docker barrier");
}

#[tokio::test]
async fn closing_during_setup_revokes_request_even_after_reopening() {
    for reopen in [false, true] {
        let home = Arc::new(Home::start().await);
        home.register(ENTRY_APP, "entryapp", false, true);
        let hold = home.docker.hold("GET /exec/exec1/json");
        let request_home = home.clone();
        let request = tokio::spawn(async move {
            request_home
                .get(&request_home.host("entryapp"), "/", &entry_public())
                .await
        });
        reached(&hold).await;
        home.manager
            .clients_entry_for_test(&entry_id(), "entryapp", false, false);
        if reopen {
            home.manager
                .clients_entry_for_test(&entry_id(), "entryapp", false, true);
        }
        hold.release();
        let reply = request.await.unwrap();
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "reopen={reopen}");
        assert_eq!(reply.error_kind(), Some("entry_closed"));
        assert!(home.upstream_hits().is_empty());
        if reopen {
            let fresh = home.get(&home.host("entryapp"), "/", &entry_public()).await;
            assert_eq!(fresh.status, StatusCode::OK);
        }
    }
}

#[tokio::test]
async fn closing_then_reopening_revokes_existing_websocket() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let (listener, addr) = bind().await;
    serve(listener, home.app.clone()).await;
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(format!(
        "GET /ws HTTP/1.1\r\nHost: {}\r\nConnection: Upgrade\r\nUpgrade: echo\r\nx-wonton-entry: public\r\n\r\n",
        home.host("entryapp"),
    ).as_bytes()).await.unwrap();
    let mut head = Vec::new();
    let mut byte = [0; 1];
    while !head.ends_with(b"\r\n\r\n") {
        tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut byte))
            .await
            .unwrap()
            .unwrap();
        head.push(byte[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 101"));
    stream.write_all(b"ping").await.unwrap();
    let mut echoed = [0; 4];
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut echoed))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&echoed, b"ping");
    home.manager
        .clients_entry_for_test(&entry_id(), "entryapp", false, false);
    home.manager
        .clients_entry_for_test(&entry_id(), "entryapp", false, true);
    let mut after_close = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut after_close))
        .await
        .expect("reopening must never restore an old connection")
        .unwrap();
    let fresh = home.get(&home.host("entryapp"), "/", &entry_public()).await;
    assert_eq!(fresh.status, StatusCode::OK);
}

#[tokio::test]
async fn restarted_container_must_pass_setup_again_before_forwarding() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    let first = home.get(&host, "/", &entry_public()).await;
    assert_eq!(first.status, StatusCode::OK);
    // 容器重启换了启动代：查不出（7）时状态不确定，暂停转发回 503；查到 1 且注册过管理员的
    // 场景由 `cleared_data_after_setup_closes_and_notifies` 覆盖（关闭）。
    home.docker.on_exec(|_, _, _| (7, String::new()));
    home.docker.set_state(
        &app_container(),
        serde_json::json!({
            "Status": "running", "Running": true,
            "StartedAt": "2026-01-02T00:00:00Z", "Health": {"Status": "healthy"},
        }),
    );
    let restarted = home.get(&host, "/", &entry_public()).await;
    assert_eq!(restarted.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(restarted.error_kind(), Some("plugin_unavailable"));
    assert_eq!(home.upstream_hits().len(), 1);
}

#[tokio::test]
async fn first_pending_keeps_entries_open() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", true, true);
    let host = home.host("entryapp");
    home.docker.on_exec(|_, _, _| (1, String::new()));
    let pending = home.get(&host, "/", &entry_public()).await;
    // 从没注册过管理员的得 1 只是主人还没注册：409，入口保持开，让主人去注册（ADR-0022）。
    assert_eq!(pending.status, StatusCode::CONFLICT);
    assert_eq!(pending.error_kind(), Some("setup_pending"));
    let record = home.manager.record_for_test(&entry_id()).unwrap();
    assert!(record.clients_entries.public);
    assert!(!record.setup_done_ever);
    let again = home.get(&host, "/", &entry_public()).await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    let lan = home
        .get(&home.lan_host("entryapp"), "/", &entry_lan())
        .await;
    assert_eq!(lan.status, StatusCode::CONFLICT);
    assert!(home._fake.state().sent.is_empty());
}

#[tokio::test]
async fn cleared_data_after_setup_closes_and_notifies() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", true, true);
    let host = home.host("entryapp");
    home.docker.on_exec(|_, _, _| (0, String::new()));
    let ok = home.get(&host, "/", &entry_public()).await;
    assert_eq!(ok.status, StatusCode::OK);
    // 注册过管理员（setup_done_ever 落盘）；之后应用数据被清空、容器重启，命令回到 1：
    // 关闭两个入口并通知主人，之后命令回到 0 也不自动重开（C7 的界面才能重开）。
    let record = home.manager.record_for_test(&entry_id()).unwrap();
    assert!(record.setup_done_ever);
    home.docker.set_state(
        &app_container(),
        serde_json::json!({
            "Status": "running", "Running": true,
            "StartedAt": "2026-01-02T00:00:00Z", "Health": {"Status": "healthy"},
        }),
    );
    home.docker.on_exec(|_, _, _| (1, String::new()));
    // 检查得 1 的关闭在这条请求期间生效；守卫撤销优先于检查结果，请求按已关闭如实回答。
    let after = home.get(&host, "/", &entry_public()).await;
    assert_eq!(after.status, StatusCode::FORBIDDEN);
    assert_eq!(after.error_kind(), Some("entry_closed"));
    let closed = home.get(&host, "/", &entry_public()).await;
    assert_eq!(closed.status, StatusCode::FORBIDDEN);
    assert_eq!(closed.error_kind(), Some("entry_closed"));
    let lan = home
        .get(&home.lan_host("entryapp"), "/", &entry_lan())
        .await;
    assert_eq!(lan.status, StatusCode::FORBIDDEN);
    let record = home.manager.record_for_test(&entry_id()).unwrap();
    assert!(!record.clients_entries.public && !record.clients_entries.lan);
    assert!(record.setup_done_ever);
    // 通知从检查任务里异步发出：守卫撤销让请求先返回，等通知落地再断言。
    let notified = async {
        loop {
            let sent = home._fake.state().sent.iter().any(|(_, content)| {
                content["body"]
                    .as_str()
                    .is_some_and(|body| body.contains("io.wonton.test.entry"))
            });
            if sent {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(5), notified)
        .await
        .expect("the owner must be told about the closed entries");
}

#[tokio::test]
async fn closed_entries_do_not_reopen_on_zero() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", true, true);
    home.docker.on_exec(|_, _, _| (0, String::new()));
    home.get(&home.host("entryapp"), "/", &entry_public()).await;
    home.docker.set_state(
        &app_container(),
        serde_json::json!({
            "Status": "running", "Running": true,
            "StartedAt": "2026-01-02T00:00:00Z", "Health": {"Status": "healthy"},
        }),
    );
    home.docker.on_exec(|_, _, _| (1, String::new()));
    home.get(&home.host("entryapp"), "/", &entry_public()).await;
    home.docker.set_state(
        &app_container(),
        serde_json::json!({
            "Status": "running", "Running": true,
            "StartedAt": "2026-01-03T00:00:00Z", "Health": {"Status": "healthy"},
        }),
    );
    home.docker.on_exec(|_, _, _| (0, String::new()));
    // 后台重查也不再碰关着的入口：重开是 C7 的开关界面，不是命令说了算。
    home.manager.recheck_clients_setup(None).await;
    assert_eq!(home.docker.execs().len(), 2);
    let reply = home.get(&home.host("entryapp"), "/", &entry_public()).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_kind(), Some("entry_closed"));
}

#[tokio::test]
async fn undetermined_state_is_retried_by_the_background_watch() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    home.docker
        .on_exec(|_, _, _| (7, "still starting".to_owned()));
    let first = home.get(&home.host("entryapp"), "/", &entry_public()).await;
    assert_eq!(first.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(home.docker.execs().len(), 1);
    // 后台重查（请求之外）也查未知状态：还查不出时再执行一次。
    home.manager.recheck_clients_setup(None).await;
    assert_eq!(home.docker.execs().len(), 2);
    // 主人注册完管理员，后台重查把状态翻过来，下一个请求放行。
    home.docker.on_exec(|_, _, _| (0, String::new()));
    home.manager.recheck_clients_setup(None).await;
    let ok = home.get(&home.host("entryapp"), "/", &entry_public()).await;
    assert_eq!(ok.status, StatusCode::OK);
}

#[tokio::test]
async fn concurrent_requests_share_one_setup_check() {
    let home = Arc::new(Home::start().await);
    home.register(ENTRY_APP, "entryapp", false, true);
    // exec 的结果在 start 时按当时的 handler 定死：先设成返回 1，检查应回 Pending。
    home.docker.on_exec(|_, _, _| (1, String::new()));
    let hold = home.docker.hold("GET /exec/exec1/json");
    let requests: Vec<_> = (0..3)
        .map(|_| {
            let home = home.clone();
            tokio::spawn(
                async move { home.get(&home.host("entryapp"), "/", &entry_public()).await },
            )
        })
        .collect();
    reached(&hold).await;
    // 并发请求共享同一次检查：只创建一条命令，不放大成多条 Docker exec。
    assert_eq!(home.docker.execs().len(), 1);
    hold.release();
    for request in requests {
        let reply = request.await.unwrap();
        assert_eq!(reply.status, StatusCode::CONFLICT);
        assert_eq!(reply.error_kind(), Some("setup_pending"));
    }
    assert_eq!(home.docker.execs().len(), 1);
    assert!(home.upstream_hits().is_empty());
}

#[tokio::test]
async fn previous_deployment_check_cannot_authorize_replacement() {
    let home = Arc::new(Home::start().await);
    home.register(ENTRY_APP, "entryapp", false, true);
    let hold = home.docker.hold("GET /exec/exec1/json");
    let request_home = home.clone();
    let old_request = tokio::spawn(async move {
        request_home
            .get(&request_home.host("entryapp"), "/", &entry_public())
            .await
    });
    reached(&hold).await;
    home.register(ENTRY_APP, "entryapp", false, true);
    home.docker.on_exec(|_, _, _| (1, String::new()));
    hold.release();
    let old = old_request.await.unwrap();
    assert_ne!(old.status, StatusCode::OK);
    assert!(home.upstream_hits().is_empty());
    let current = home.get(&home.host("entryapp"), "/", &entry_public()).await;
    assert_eq!(current.status, StatusCode::CONFLICT);
    assert_eq!(current.error_kind(), Some("setup_pending"));
    assert!(home.upstream_hits().is_empty());
}

#[tokio::test]
async fn unavailable_target_rejected_independently_of_auxiliary_services() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    home.manager.serving_for_test(&entry_id(), &[]);
    let unavailable = home.get(&host, "/", &entry_public()).await;
    assert_eq!(unavailable.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(unavailable.error_kind(), Some("plugin_unavailable"));
    assert!(home.upstream_hits().is_empty());
    home.manager.clients_status_for_test(
        &entry_id(),
        PluginStatus::Degraded {
            reason: "auxiliary service failed".into(),
        },
    );
    home.manager.serving_for_test(&entry_id(), &["app"]);
    let serving = home.get(&host, "/", &entry_public()).await;
    assert_eq!(serving.status, StatusCode::OK);
    home.docker.set_state(
        &app_container(),
        serde_json::json!({
            "Status": "running", "Running": true, "Health": {"Status": "unhealthy"},
        }),
    );
    let unhealthy = home.get(&host, "/", &entry_public()).await;
    assert_eq!(unhealthy.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(unhealthy.error_kind(), Some("plugin_unavailable"));
    assert_eq!(home.upstream_hits().len(), 1);
}

#[tokio::test]
async fn lifecycle_transitions_cannot_reuse_old_clients_route() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    for status in [
        PluginStatus::Installing,
        PluginStatus::Upgrading,
        PluginStatus::Uninstalling,
        PluginStatus::Stopped,
    ] {
        home.manager.clients_status_for_test(&entry_id(), status);
        let reply = home.get(&home.host("entryapp"), "/", &entry_public()).await;
        assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(reply.error_kind(), Some("plugin_unavailable"));
    }
    assert!(home.upstream_hits().is_empty());
    home.manager
        .clients_status_for_test(&entry_id(), PluginStatus::Running);
    let recovered = home.get(&home.host("entryapp"), "/", &entry_public()).await;
    assert_eq!(recovered.status, StatusCode::OK);
}

#[tokio::test]
async fn unknown_label_is_cached_until_records_change() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let absent = home.host("absent");
    assert_eq!(
        home.get(&absent, "/", &entry_public()).await.status,
        StatusCode::MISDIRECTED_REQUEST
    );
    let mut record = home.manager.record_for_test(&entry_id()).unwrap();
    record.entry_label = Some("absent".into());
    home.manager.store().save(&record).unwrap();
    assert_eq!(
        home.get(&absent, "/", &entry_public()).await.status,
        StatusCode::MISDIRECTED_REQUEST
    );
    home.manager
        .clients_entry_for_test(&entry_id(), "absent", false, true);
    assert_eq!(
        home.get(&absent, "/", &entry_public()).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn duplicate_labels_never_choose_one_plugin() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "same", false, true);
    home.register(HOOK_APP, "same", false, true);
    let reply = home
        .get(&home.host("same"), "/webhook", &entry_public())
        .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), Some("plugin_unavailable"));
    assert!(home.upstream_hits().is_empty());
}

#[tokio::test]
async fn unreadable_and_corrupt_records_are_not_unknown_labels() {
    for directory_error in [false, true] {
        let home = Home::start().await;
        home.register(ENTRY_APP, "entryapp", false, true);
        let records = home._harness.data_dir.join("plugins");
        if directory_error {
            std::fs::rename(&records, home._harness.data_dir.join("saved-plugins")).unwrap();
            std::fs::write(&records, b"not a directory").unwrap();
        } else {
            std::fs::write(records.join(format!("{}.json", entry_id())), b"{broken").unwrap();
        }
        home.manager.serving_for_test(&entry_id(), &["app"]);
        let reply = home.get(&home.host("entryapp"), "/", &entry_public()).await;
        assert_eq!(
            reply.status,
            StatusCode::SERVICE_UNAVAILABLE,
            "directory_error={directory_error}"
        );
        assert_eq!(reply.error_kind(), Some("plugin_unavailable"));
        assert!(home.upstream_hits().is_empty());
    }
}
