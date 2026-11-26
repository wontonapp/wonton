//! 网关的安装、卸载和到达：没网关也能装、事后补上、已接上的不被多签一次、网关只能有一个。

use super::*;

#[tokio::test]
pub(crate) async fn uninstall_revokes_the_credential() {
    let bed = bed().await;
    let id = PluginId::parse("app.test").unwrap();
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    bed.manager
        .uninstall(&id, false, &super::langs())
        .await
        .unwrap();
    // 健康探测凭据（<网关>/health）不属于使用方，不算数。
    let consumer_keys = |keys: &BTreeMap<String, String>| {
        keys.keys()
            .filter(|name| !name.ends_with("/health"))
            .count()
    };
    assert_eq!(
        consumer_keys(&bed.keys.lock().unwrap()),
        0,
        "卸载时在网关里吊销"
    );
    // 重装是一个新值：不删数据卸载也不保留凭据。
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    assert_eq!(consumer_keys(&bed.keys.lock().unwrap()), 1);
}

#[tokio::test]
pub(crate) async fn installs_without_a_gateway_and_the_variables_come_up_empty() {
    // 没有网关：照样装，状态照常 Running，模型变量全是空字符串。
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    m.install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let env = env_of_container(&docker, &container_name("app-test", "web"));
    for name in ["BASE", "ANTHROPIC", "KEY", "MODEL"] {
        assert_eq!(env[name], "", "{name}");
    }
    // 详情页有“没接上”的轻提示，没有签发失败的原因可带。
    let detail = m
        .judged_detail(&PluginId::parse("app.test").unwrap(), &zh())
        .await
        .unwrap();
    assert_eq!(detail.hints.len(), 1);
    assert_eq!(
        detail.hints[0].category,
        wonton_protocol::PluginHintCategory::ModelsNotConnected
    );
    assert_eq!(detail.hints[0].detail, None);
}

#[tokio::test]
pub(crate) async fn the_gateway_arriving_later_issues_and_rebuilds_the_consumer() {
    let bed = bed_without_gateway().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    assert_eq!(web_env(&bed)["BASE"], "");

    // 网关装上：核心给没接上的使用方签发凭据、按新变量重建容器，轻提示消失。
    bed.install_gateway().await;
    let env = web_env(&bed);
    assert_eq!(env["BASE"], "http://models.wonton.internal:8080/v1");
    assert_eq!(bed.keys.lock().unwrap()["app.test"], env["KEY"]);
    assert!(bed.hints().await.hints.is_empty());
}

#[tokio::test]
pub(crate) async fn an_install_waits_for_a_gateway_that_is_still_starting() {
    // 默认 Bot 和网关一起装时，签发会赶上网关还在起：要等它就绪，不能拿空变量收场。
    let bed = bed().await;
    let gateway = PluginId::parse("system.models").unwrap();
    let manager = Arc::new(bed.manager);
    manager.stop_detached(gateway.clone()).await.unwrap();

    let installing = manager.clone();
    let package = consumer();
    let granted = models();
    let pending =
        tokio::spawn(async move { installing.install(&package, &granted, super::langs()).await });
    // 安装已经进到等网关的循环里，再让网关起来。
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    manager.start_detached(gateway).await.unwrap();
    pending.await.unwrap().unwrap();

    let env = env_of_container(&bed.docker, &container_name("app-test", "web"));
    assert_eq!(env["BASE"], "http://models.wonton.internal:8080/v1");
    assert_eq!(env["KEY"].len(), 64);
}

#[tokio::test]
pub(crate) async fn a_restored_consumer_without_a_listed_entry_is_connected_by_its_record() {
    // restore 回来的插件没有店里的那份条目：网关事后到达时也要按记录接上，不能整条跳过。
    let bed = bed_without_gateway().await;
    let id = PluginId::parse("app.test").unwrap();
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    bed.manager
        .plugins
        .write()
        .expect("插件表锁中毒")
        .get_mut(&id)
        .expect("刚装好的插件")
        .listed = None;

    bed.install_gateway().await;

    let env = env_of_container(&bed.docker, &container_name("app-test", "web"));
    assert_eq!(env["KEY"].len(), 64);
    assert!(bed.hints().await.hints.is_empty());
}

#[tokio::test]
pub(crate) async fn the_gateway_coming_back_again_leaves_connected_consumers_alone() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let env = web_env(&bed);
    let web = container_name("app-test", "web");
    assert_eq!(bed.docker.creations(&web), 1);

    // 网关再就绪一次（核心重启、网关容器恢复、升级都对一次账）：已接上的使用方
    // 凭据值没变，容器不动，不白重启一遍。
    assert!(bed.manager.reconcile_models().await.is_empty());
    assert_eq!(bed.docker.creations(&web), 1);
    assert_eq!(web_env(&bed), env);
}

#[tokio::test]
pub(crate) async fn a_failed_issue_still_installs_and_the_hint_carries_the_reason() {
    let bed = bed().await;
    bed.docker
        .on_exec(|_, cmd, _| match cmd.get(2).map(String::as_str) {
            Some("issue") => (1, "数据库被锁住了".to_owned()),
            _ => (0, String::new()),
        });
    // 签发失败：照样装，状态照常，变量为空。
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    assert_eq!(web_env(&bed)["KEY"], "");
    let consumer = bed
        .manager
        .list()
        .into_iter()
        .find(|p| p.id.as_str() == "app.test")
        .unwrap();
    assert!(
        matches!(consumer.status, PluginStatus::Running),
        "签发失败不改变插件状态"
    );
    // 轻提示带上签发失败的原因；记下的文件里只有原因，没有凭据值。
    let detail = bed.hints().await;
    assert_eq!(detail.hints.len(), 1);
    let reason = detail.hints[0].detail.as_deref().unwrap();
    assert!(reason.contains("数据库被锁住了"), "{reason}");
    let stored =
        std::fs::read_to_string(bed._dir.path().join("plugins/model-keys/app.test.json")).unwrap();
    assert!(stored.contains("数据库被锁住了"), "{stored}");

    // 网关修好：对账时签发、按新变量重建，提示消失。
    bed.docker.on_exec(credential_script(bed.keys.clone()));
    let failures = bed.manager.reconcile_models().await;
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        web_env(&bed)["BASE"],
        "http://models.wonton.internal:8080/v1"
    );
    assert!(bed.hints().await.hints.is_empty());
}

#[tokio::test]
pub(crate) async fn uninstalling_the_gateway_leaves_the_consumer_running_and_the_hint_returns() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let key = web_env(&bed)["KEY"].clone();

    // 网关卸掉：使用方不动，还在运行，轻提示变回“没接上”。
    bed.manager
        .uninstall(
            &PluginId::parse("system.models").unwrap(),
            false,
            &super::langs(),
        )
        .await
        .unwrap();
    let consumer = bed
        .manager
        .list()
        .into_iter()
        .find(|p| p.id.as_str() == "app.test")
        .unwrap();
    assert!(matches!(consumer.status, PluginStatus::Running));
    assert_eq!(web_env(&bed)["KEY"], key, "使用方的容器没动");
    let detail = bed.hints().await;
    assert_eq!(detail.hints.len(), 1);
    assert_eq!(
        detail.hints[0].category,
        wonton_protocol::PluginHintCategory::ModelsNotConnected
    );
    // 网关不在：对账不碰使用方。
    assert!(bed.manager.reconcile_models().await.is_empty());

    // 网关装回来：凭据值还记着，对账用同一个值重新签发，容器不用重建就接上了。
    bed.install_gateway().await;
    assert_eq!(web_env(&bed)["KEY"], key, "重新签发用记下的同一个值");
    assert!(bed.hints().await.hints.is_empty());
}

#[tokio::test]
pub(crate) async fn only_one_gateway_can_be_installed() {
    let bed = bed().await;
    let port = fake_gateway(Keys::default()).await;
    let err = bed
        .manager
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
        .unwrap_err();
    assert_eq!(err.kind(), "conflict", "{err}");
}
