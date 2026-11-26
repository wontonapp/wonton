//! 对账：网关丢状态或凭据命令结局未知后，巡检按记下的变量补发并恢复，不重建容器、不换凭据。

use super::*;

#[tokio::test]
pub(crate) async fn the_gateway_gets_lost_or_stale_credentials_back_from_the_core() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let key = web_env(&bed)["KEY"].clone();
    // 网关的数据丢了：对账时用同一个值重新签发，使用方不用重启。
    bed.keys.lock().unwrap().clear();
    assert!(bed.manager.reconcile_models().await.is_empty());
    assert_eq!(bed.keys.lock().unwrap()["app.test"], key);
    // 网关里的同名凭据不是这个值（issue 不改动已有的）：验证得到 401，先吊销再签发。
    bed.keys
        .lock()
        .unwrap()
        .insert("app.test".into(), "stale".into());
    assert!(bed.manager.reconcile_models().await.is_empty());
    assert_eq!(bed.keys.lock().unwrap()["app.test"], key);
}

#[tokio::test]
pub(crate) async fn a_rebuild_keeps_the_recorded_vars_even_when_the_gateway_is_gone() {
    let (docker, engine) = FakeDocker::start().await;
    let keys = Keys::default();
    let script = keys.clone();
    docker.on_exec(move |_, cmd, stdin| {
        let mut keys = script.lock().unwrap();
        let value = String::from_utf8_lossy(stdin).trim().to_owned();
        match cmd {
            [_, _, action, name] if action == "issue" => {
                keys.entry(name.clone()).or_insert(value);
                (0, String::new())
            }
            [_, _, action, name] if action == "revoke" => {
                keys.remove(name);
                (0, String::new())
            }
            _ => (2, "用法不对".to_owned()),
        }
    });
    let port = fake_gateway(keys).await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    manager
        .install_checked(
            provider(port),
            &[],
            &[],
            false,
            &[],
            Default::default(),
            &super::langs(),
        )
        .await
        .unwrap();
    manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let env = env_of_container(&docker, &container_name("app-test", "web"));

    // 网关不在时重建使用方（改资源上限）：注入记下的变量，不退回空值。
    manager
        .uninstall(
            &PluginId::parse("system.models").unwrap(),
            false,
            &super::langs(),
        )
        .await
        .unwrap();
    let detail = manager
        .set_limits(
            PluginId::parse("app.test").unwrap(),
            wonton_protocol::PluginLimits {
                memory: Some(1024 * 1024 * 1024),
                cpu_percent: None,
            },
            zh(),
        )
        .await
        .unwrap();
    let rebuilt = env_of_container(&docker, &container_name("app-test", "web"));
    assert_eq!(rebuilt["KEY"], env["KEY"], "记下的凭据原样注入");
    assert_eq!(rebuilt["BASE"], env["BASE"]);
    assert_eq!(detail.hints.len(), 1, "网关不在，重建后轻提示照给");
}

#[tokio::test]
pub(crate) async fn a_record_without_bot_keys_still_reads_and_the_plugin_comes_up() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let env = web_env(&bed);
    // 较早的核心写下的记录：`vars` 里没有之后才加的 `bot_keys`。
    let path = bed._dir.path().join("plugins/model-keys/app.test.json");
    let mut stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    stored["vars"].as_object_mut().unwrap().remove("bot_keys");
    std::fs::write(&path, stored.to_string()).unwrap();

    // 核心重启：同一份数据目录和容器。
    let restarted = manager_with(bed.manager.engine.clone(), &bed._dir);
    restarted.restore().await.unwrap();
    let status = |m: &PluginManager| {
        m.list()
            .into_iter()
            .find(|p| p.id.as_str() == "app.test")
            .unwrap()
            .status
    };
    assert!(matches!(status(&restarted), PluginStatus::Running));

    // 网关就绪后对账：旧记录能读出来，不把插件标成异常，也不白重建容器。
    let failures = restarted.reconcile_models().await;
    assert!(failures.is_empty(), "{failures:?}");
    assert!(matches!(status(&restarted), PluginStatus::Running));
    assert_eq!(web_env(&bed), env);
    let detail = restarted
        .judged_detail(&PluginId::parse("app.test").unwrap(), &zh())
        .await
        .unwrap();
    assert!(detail.hints.is_empty());
}

/// 对账失败：容器状态不动（不撒谎说“异常”），原因写进详情页的轻提示；巡检按原值补发，
/// 网关还拒绝就一直带着原因，容器和凭据都不换。
#[tokio::test]
pub(crate) async fn a_failed_reconcile_keeps_the_container_running_and_carries_the_reason() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    // 网关重启后不认旧凭据，重签又被拒绝。
    bed.keys.lock().unwrap().remove("app.test");
    bed.docker.on_exec(|_, _, _| (1, "网关坏了".to_owned()));
    let failures = bed.manager.reconcile_models().await;
    assert_eq!(failures.len(), 1);
    let consumer = bed
        .manager
        .list()
        .into_iter()
        .find(|p| p.id.as_str() == "app.test")
        .unwrap();
    assert!(
        matches!(consumer.status, PluginStatus::Running),
        "{:?}",
        consumer.status
    );
    let hints = bed.hints().await.hints;
    let [hint] = hints.as_slice() else {
        panic!("对账失败的使用方应有轻提示: {hints:?}");
    };
    assert_eq!(
        hint.category,
        wonton_protocol::PluginHintCategory::ModelsNotConnected
    );
    assert!(
        hint.detail
            .as_deref()
            .is_some_and(|d| d.contains("网关坏了")),
        "{hint:?}"
    );

    let web = container_name("app-test", "web");
    let execs = credential_execs(&bed, "app.test");
    let creations = bed.docker.creations(&web);
    let env_before = web_env(&bed);
    bed.manager.watch_containers().await;
    bed.manager.watch_containers().await;
    let consumer = bed
        .manager
        .list()
        .into_iter()
        .find(|p| p.id.as_str() == "app.test")
        .unwrap();
    assert!(matches!(consumer.status, PluginStatus::Running));
    let hints = bed.hints().await.hints;
    let [hint] = hints.as_slice() else {
        panic!("网关不认凭据时轻提示不能消失: {hints:?}");
    };
    assert_eq!(
        hint.category,
        wonton_protocol::PluginHintCategory::ModelsNotConnected
    );
    assert!(
        hint.detail
            .as_deref()
            .is_some_and(|d| d.contains("网关坏了")),
        "{hint:?}"
    );
    assert!(
        credential_execs(&bed, "app.test") > execs,
        "巡检要拿原值补发凭据命令"
    );
    assert_eq!(bed.docker.creations(&web), creations, "巡检不重建容器");
    assert_eq!(web_env(&bed), env_before, "巡检不换凭据");
}

/// 凭据命令的结局未知（#643 的超时）：容器照常运行、原因写明；命令在引擎侧完成后，
/// 巡检自己对账恢复，不重启、不重建、不换钥匙、不补发写命令。
#[tokio::test]
pub(crate) async fn an_unknown_command_outcome_recovers_once_the_command_settles() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let web = container_name("app-test", "web");
    let env_before = web_env(&bed);
    let creations = bed.docker.creations(&web);

    // 网关丢了状态；重签的命令已创建，但启动时连接断了：结局未知，对账失败。
    bed.keys.lock().unwrap().remove("app.test");
    bed.docker.fail_next_exec_start("connection reset", true);
    let failures = bed.manager.reconcile_models().await;
    assert_eq!(failures.len(), 1);
    let consumer = bed
        .manager
        .list()
        .into_iter()
        .find(|p| p.id.as_str() == "app.test")
        .unwrap();
    assert!(
        matches!(consumer.status, PluginStatus::Running),
        "{:?}",
        consumer.status
    );
    let hints = bed.hints().await.hints;
    let [hint] = hints.as_slice() else {
        panic!("命令结局未知时应有轻提示: {hints:?}");
    };
    assert_eq!(
        hint.category,
        wonton_protocol::PluginHintCategory::ModelsNotConnected
    );
    assert!(
        hint.detail
            .as_deref()
            .is_some_and(|d| d.contains("connection reset")),
        "{hint:?}"
    );

    // 巡检只看不动：命令没回来之前不签发、不吊销、不清原因。
    bed.manager.watch_containers().await;
    let consumer = bed
        .manager
        .list()
        .into_iter()
        .find(|p| p.id.as_str() == "app.test")
        .unwrap();
    assert!(matches!(consumer.status, PluginStatus::Running));
    assert!(
        bed.hints()
            .await
            .hints
            .iter()
            .any(|h| h.category == wonton_protocol::PluginHintCategory::ModelsNotConnected)
    );
    let execs = credential_execs(&bed, "app.test");

    // 命令在网关那边其实完成了（钥匙登记回去了）：下一次巡检自己恢复。
    let key = env_before["KEY"].clone();
    bed.keys.lock().unwrap().insert("app.test".into(), key);
    bed.docker.finish_last_exec(0);
    bed.manager.watch_containers().await;
    let consumer = bed
        .manager
        .list()
        .into_iter()
        .find(|p| p.id.as_str() == "app.test")
        .unwrap();
    assert!(matches!(consumer.status, PluginStatus::Running));
    assert!(
        !bed.hints()
            .await
            .hints
            .iter()
            .any(|h| h.category == wonton_protocol::PluginHintCategory::ModelsNotConnected)
    );
    assert_eq!(bed.docker.creations(&web), creations, "恢复不重建容器");
    assert_eq!(web_env(&bed), env_before, "恢复不换凭据");
    assert_eq!(
        credential_execs(&bed, "app.test"),
        execs,
        "恢复不补发写命令"
    );
}

/// 命令创建了但根本没启动（Docker 报没在跑、也没有退出码）：结局其实已经定了。
/// 巡检按原值补发一次，网关收下就恢复，不重建容器。
#[tokio::test]
pub(crate) async fn an_unknown_command_that_never_started_settles_and_retries() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let web = container_name("app-test", "web");
    let env_before = web_env(&bed);
    let creations = bed.docker.creations(&web);

    bed.keys.lock().unwrap().remove("app.test");
    bed.docker.fail_next_exec_start("connection reset", false);
    assert_eq!(bed.manager.reconcile_models().await.len(), 1);
    assert!(
        bed.hints()
            .await
            .hints
            .iter()
            .any(|h| h.category == wonton_protocol::PluginHintCategory::ModelsNotConnected)
    );

    // 命令没跑，网关还缺这把钥匙：巡检补发成功，提示清掉。
    bed.manager.watch_containers().await;
    assert!(
        !bed.hints()
            .await
            .hints
            .iter()
            .any(|h| h.category == wonton_protocol::PluginHintCategory::ModelsNotConnected)
    );
    assert_eq!(bed.docker.creations(&web), creations, "恢复不重建容器");
    assert_eq!(web_env(&bed), env_before, "恢复不换凭据");
}

/// 凭据命令进行中网关容器重启：Docker 把 exec 记录丢掉了，不能一直卡在“结局未知”。
/// 命令也许已经在重启前落地，重新验证同一份凭据就能确定。
#[tokio::test]
pub(crate) async fn an_unknown_command_whose_exec_vanished_recovers_by_verifying() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let web = container_name("app-test", "web");
    let env_before = web_env(&bed);
    let creations = bed.docker.creations(&web);

    bed.keys.lock().unwrap().remove("app.test");
    bed.docker.fail_next_exec_start("connection reset", true);
    assert_eq!(bed.manager.reconcile_models().await.len(), 1);

    // 命令在引擎侧跑完并登记了钥匙，核心没等到结果；紧接着网关容器重启，exec 查不到了。
    let key = env_before["KEY"].clone();
    bed.keys.lock().unwrap().insert("app.test".into(), key);
    bed.docker.forget_execs();
    let execs = credential_execs(&bed, "app.test");
    bed.manager.watch_containers().await;

    assert!(
        !bed.hints()
            .await
            .hints
            .iter()
            .any(|h| h.category == wonton_protocol::PluginHintCategory::ModelsNotConnected)
    );
    assert_eq!(
        credential_execs(&bed, "app.test"),
        execs,
        "凭据已生效，不补发命令"
    );
    assert_eq!(bed.docker.creations(&web), creations, "恢复不重建容器");
    assert_eq!(web_env(&bed), env_before, "恢复不换凭据");
}

#[tokio::test]
pub(crate) async fn the_watch_retries_a_failed_reconcile_and_the_consumer_recovers() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let web = container_name("app-test", "web");
    let key = web_env(&bed)["KEY"].clone();
    let status = |bed: &Bed| {
        bed.manager
            .list()
            .into_iter()
            .find(|p| p.id.as_str() == "app.test")
            .unwrap()
            .status
    };

    // 网关丢了状态，重签时又一时应答慢（I/O 压力下命令超时）：对账失败，容器状态照实报，原因进轻提示。
    bed.keys.lock().unwrap().remove("app.test");
    bed.docker.on_exec(|_, _, _| (1, "命令超时".to_owned()));
    assert_eq!(bed.manager.reconcile_models().await.len(), 1);
    assert!(
        matches!(status(&bed), PluginStatus::Running),
        "{:?}",
        status(&bed)
    );
    let reason = bed
        .hints()
        .await
        .hints
        .into_iter()
        .find(|h| h.category == wonton_protocol::PluginHintCategory::ModelsNotConnected)
        .and_then(|h| h.detail)
        .expect("对账失败的使用方应有轻提示");
    assert!(reason.contains("命令超时"), "{reason}");

    // 还没好：下一轮巡检重试还是失败，继续带着原因，不假装恢复，也不重建容器。
    bed.manager.watch_containers().await;
    assert!(matches!(status(&bed), PluginStatus::Running));
    let reason = bed
        .hints()
        .await
        .hints
        .into_iter()
        .find(|h| h.category == wonton_protocol::PluginHintCategory::ModelsNotConnected)
        .and_then(|h| h.detail)
        .expect("网关不认凭据时轻提示不能消失");
    assert!(reason.contains("命令超时"), "{reason}");
    assert_eq!(bed.docker.creations(&web), 1);

    // 网关缓过来了：下一轮巡检自己重试凭据，容器没白重建，状态回到运行中。
    bed.docker.on_exec(credential_script(bed.keys.clone()));
    bed.manager.watch_containers().await;
    assert!(
        matches!(status(&bed), PluginStatus::Running),
        "{:?}",
        status(&bed)
    );
    assert_eq!(web_env(&bed)["KEY"], key, "重试沿用记下的同一个值");
    assert_eq!(bed.docker.creations(&web), 1);
}

/// 记下的全部凭据值：`{名字: 值}`。
fn stored_keys(bed: &Bed) -> BTreeMap<String, String> {
    let stored =
        std::fs::read_to_string(bed._dir.path().join("plugins/model-keys/app.test.json")).unwrap();
    let file: serde_json::Value = serde_json::from_str(&stored).unwrap();
    serde_json::from_value(file["keys"].clone()).unwrap()
}

#[tokio::test]
pub(crate) async fn a_failed_retry_still_tries_the_rest_and_replaces_only_the_failed_value() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let bed = bed().await;
    let source = package(
        "  gateway:\n    image: hermes\n    environment:\n      KEY: ${WONTON_MODEL_API_KEY}\n  \
         web:\n    image: nginx\n    environment:\n      KEY: ${WONTON_MODEL_API_KEY}",
        "  bot: {display_name: {zh: 测试, en: Test}}\n  bots: {services: [gateway]}\n  \
         permissions: [matrix.bot, models]",
    );
    let granted = vec!["matrix.bot".parse().unwrap(), "models".parse().unwrap()];
    bed.manager
        .install(&source, &granted, super::langs())
        .await
        .unwrap();
    let first = "app.test".to_owned();
    let second = "app.test/default".to_owned();
    let before = stored_keys(&bed);
    let status = |bed: &Bed| {
        bed.manager
            .list()
            .into_iter()
            .find(|p| p.id.as_str() == "app.test")
            .unwrap()
            .status
    };

    // 网关丢了状态，第一个凭据一直超时，第二个每次都正常：一轮里两个都要试，不能撞到第一个就收手。
    bed.keys.lock().unwrap().clear();
    let keys = bed.keys.clone();
    let issues = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
    let counted = issues.clone();
    let failing = first.clone();
    bed.docker.on_exec(move |_, cmd, stdin| {
        let value = String::from_utf8_lossy(stdin).trim().to_owned();
        let mut keys = keys.lock().unwrap();
        let mut issues = counted.lock().unwrap();
        match cmd {
            [_, _, action, name] if action == "issue" => {
                let count = issues.entry(name.clone()).or_default();
                *count += 1;
                if name == &failing && *count <= 2 {
                    (1, "命令超时".to_owned())
                } else {
                    // 网关的 issue 收下 stdin 的值：重新签发成功时值不变，同一个值又认了。
                    keys.entry(name.clone()).or_insert(value);
                    (0, String::new())
                }
            }
            [_, _, action, name] if action == "revoke" => {
                keys.remove(name);
                (0, String::new())
            }
            _ => (2, "用法不对".to_owned()),
        }
    });

    // 对账失败：两个凭据都试过，第二个照常重新签发了同一个值。
    assert_eq!(bed.manager.reconcile_models().await.len(), 1);
    assert_eq!(issues.lock().unwrap()[&second], 1);
    assert_eq!(stored_keys(&bed)[&second], before[&second]);
    // 再一轮还是失败：轻提示里带上是哪个凭据，第二个也又试了一次。
    bed.manager.watch_containers().await;
    let reason = bed
        .hints()
        .await
        .hints
        .into_iter()
        .find(|h| h.category == wonton_protocol::PluginHintCategory::ModelsNotConnected)
        .and_then(|h| h.detail)
        .expect("凭据还在失败，使用方应继续带着轻提示");
    assert!(reason.contains(&format!("{first}: ")), "{reason}");
    assert_eq!(issues.lock().unwrap()[&second], 2);
    // 第三轮缓过来：坏的那个重新签发、第二个也照试一遍，状态回去。
    bed.manager.watch_containers().await;
    let counts = issues.lock().unwrap().clone();
    assert_eq!(counts[&first], 3, "{counts:?}");
    assert!(
        matches!(status(&bed), PluginStatus::Running),
        "{:?}",
        status(&bed)
    );
    let after = stored_keys(&bed);
    assert_eq!(after, before, "值没变：重签发只重建网关里的那条记录");
    let gateway = env_of_container(
        &bed.docker,
        &Instance::default().bot_container("app-test", "gateway", "default"),
    );
    assert_eq!(gateway["KEY"], before[&second], "好的那个凭据原样留着");
}
