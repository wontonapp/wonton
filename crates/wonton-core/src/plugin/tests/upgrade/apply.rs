use super::*;

#[tokio::test]
pub(crate) async fn owner_limits_survive_the_upgrade() {
    let (github, docker, m, _dir) = with_github(false).await;
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    let revision = m.installed_plugin(&id(), &zh()).unwrap().revision;
    m.set_limits(id(), memory_only(6 * GIB), Default::default())
        .await
        .unwrap();
    assert_eq!(m.installed_plugin(&id(), &zh()).unwrap().revision, revision);

    release(&github, &hello("1.1.0", "", "  permissions: []\n"));
    upgrade(&m).await.unwrap();
    assert_eq!(version(&m), "1.1.0");
    let host = &docker.created(&web()).unwrap()["HostConfig"];
    assert_eq!(host["Memory"], json!(6 * GIB));
    assert_eq!(host["MemorySwap"], json!(6 * GIB));
    // 记录里没调过 CPU，compose 也没写 cpus：重建出来的容器不设 NanoCpus（默认不限）。
    assert_eq!(host["NanoCpus"], Value::Null);
    let detail = m.installed_detail(&id(), &zh()).unwrap();
    assert_eq!(detail.limits, Some(memory_only(6 * GIB)));
}

#[tokio::test]
pub(crate) async fn a_new_version_that_will_not_start_rolls_back_to_the_old_containers() {
    let (github, docker, m, _dir) = with_github(false).await;
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    let revision = m.installed_plugin(&id(), &zh()).unwrap().revision;
    let worker = container_name("app-hello", "worker");
    docker.set_state(
        &worker,
        json!({"Status": "exited", "Running": false, "ExitCode": 1}),
    );
    release(
        &github,
        &hello(
            "1.1.0",
            "  worker:\n    image: worker:1\n",
            "  permissions: []\n",
        ),
    );

    let err = upgrade(&m).await.unwrap_err();
    assert!(
        err.to_string().contains(&super::message(
            "plugin-settings-upgrade-rolled-back",
            &[("new", "1.1.0"), ("version", "1.0.0")],
        )),
        "{err}"
    );
    assert_eq!(version(&m), "1.0.0");
    assert_eq!(m.status_of(&id()), Some(PluginStatus::Running));
    assert_eq!(m.installed_plugin(&id(), &zh()).unwrap().revision, revision);
    assert_eq!(m.installed(&zh()).plugins[0].revision, revision);
    assert_eq!(
        m.installed_detail(&id(), &zh())
            .unwrap()
            .installed
            .unwrap()
            .revision,
        revision
    );
    assert_eq!(image(&docker, &web()), "web:1.0.0");
    assert!(!docker.containers().contains(&worker), "新版本的容器都删了");
    assert_eq!(m.store.load(&id()).unwrap().unwrap().version, "1.0.0");
}

#[tokio::test]
pub(crate) async fn an_image_that_cannot_be_pulled_never_touches_the_old_containers() {
    let (github, docker, m, _dir) = with_github(false).await;
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    release(&github, &hello("1.1.0", "", "  permissions: []\n"));
    docker.fail_pulls(Some("toomanyrequests"));

    let err = upgrade(&m).await.unwrap_err();
    assert_eq!(err.kind(), "image_pull_failed", "{err}");
    let stopped = format!("POST /containers/{}/stop", web());
    assert!(!docker.writes().iter().any(|w| w.starts_with(&stopped)));
    assert_eq!(m.status_of(&id()), Some(PluginStatus::Running));
    assert_eq!(version(&m), "1.0.0");
}

#[tokio::test]
pub(crate) async fn nothing_is_upgraded_while_the_core_itself_is_upgrading() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (frozen, watching) = watch::channel(false);
    let m = Arc::new(manager_with(engine, &dir).with_upgrade_freeze(watching));
    let v1 = hello("1.0.0", "", "  permissions: []\n");
    m.install(&v1, &[], super::langs()).await.unwrap();
    frozen.send(true).unwrap();
    let v2 =
        PluginPackage::parse(&hello("1.1.0", "", "  permissions: []\n"), &super::langs()).unwrap();
    let err = m
        .upgrade_upload(
            &id(),
            v2,
            Vec::new(),
            Vec::new(),
            false,
            &[],
            &Default::default(),
            &super::langs(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "core_upgrading", "{err}");
    assert_eq!(image(&docker, &web()), "web:1.0.0");
}
