use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::super::PluginManager;
use super::super::fake_docker::FakeDocker;
use super::super::gpu::fake_host::FakeHost;
use super::super::languages::Languages;
use super::super::manifest::{Permission, PluginId};
use super::super::names::container_name;
use super::super::store::{InstalledRecord, Store};
use super::super::{PluginStatus, PluginView};
use super::manager::manager_with;
use super::package;

const GIB: i64 = 1 << 30;

fn plugin(service: &str, requirements: &str) -> String {
    package(
        &format!("  web:\n    image: example/web\n{service}"),
        &format!("  permissions: []\n{requirements}"),
    )
}

fn host_config(docker: &FakeDocker) -> Value {
    docker
        .created(&container_name("app-test", "web"))
        .expect("容器已创建")["HostConfig"]
        .clone()
}

async fn manager_on(host: &FakeHost) -> (FakeDocker, PluginManager, tempfile::TempDir) {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir).with_host(host.paths());
    (docker, manager, dir)
}

fn limits(memory: u64, cpu_percent: u32) -> wonton_protocol::PluginLimits {
    wonton_protocol::PluginLimits {
        memory: Some(memory),
        cpu_percent: Some(cpu_percent),
    }
}

/// 只调内存：CPU 恢复默认（`cpu_percent: None`）。
fn memory_only(memory: u64) -> wonton_protocol::PluginLimits {
    wonton_protocol::PluginLimits {
        memory: Some(memory),
        cpu_percent: None,
    }
}

/// OOM 事件是广播：监听建立前的会错过，等到通知出现为止，每轮重发。
async fn wait_for_notice(
    docker: &FakeDocker,
    m: &Arc<PluginManager>,
    id: &PluginId,
    container: &str,
) -> Vec<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        docker.emit_oom(container);
        let texts: Vec<String> = m
            .installed_plugin(id, &Default::default())
            .map(|p| p.notices.into_iter().map(|n| n.text).collect())
            .unwrap_or_default();
        if !texts.is_empty() || std::time::Instant::now() > deadline {
            return texts;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn only(views: Vec<PluginView>) -> PluginView {
    let [view] = <[PluginView; 1]>::try_from(views).expect("只有一个插件");
    view
}

fn record_of(dir: &tempfile::TempDir, id: &PluginId) -> InstalledRecord {
    Store::open(dir.path().join("plugins"))
        .unwrap()
        .load(id)
        .unwrap()
        .expect("记录已写入")
}

#[tokio::test]
async fn defaults_limit_each_service_without_a_compose_override() {
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (docker, manager, _dir) = manager_on(&host).await;

    manager
        .install(&plugin("", ""), &[], super::langs())
        .await
        .unwrap();

    let host = host_config(&docker);
    assert_eq!(host["CpuShares"], json!(512));
    assert_eq!(host["Memory"], json!(8 * GIB));
    assert_eq!(host["MemorySwap"], json!(8 * GIB));
    assert_eq!(host["PidsLimit"], json!(4096));
    assert_eq!(host["OomScoreAdj"], json!(500));
    assert_eq!(host["MemoryReservation"], Value::Null);
    assert_eq!(host["NanoCpus"], Value::Null);
    assert_eq!(host["CgroupParent"], Value::Null);
}

#[tokio::test]
async fn compose_values_override_the_defaults() {
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (docker, manager, _dir) = manager_on(&host).await;
    let service = "    mem_limit: 256m\n    mem_reservation: 128m\n    cpus: \"1.5\"\n    cpu_shares: 1024\n    pids_limit: 64\n";

    manager
        .install(&plugin(service, ""), &[], super::langs())
        .await
        .unwrap();

    let host = host_config(&docker);
    assert_eq!(host["Memory"], json!(256 * 1024 * 1024));
    assert_eq!(host["MemorySwap"], json!(256 * 1024 * 1024));
    assert_eq!(host["MemoryReservation"], json!(128 * 1024 * 1024));
    assert_eq!(host["NanoCpus"], json!(1_500_000_000i64));
    assert_eq!(host["CpuShares"], json!(1024));
    assert_eq!(host["PidsLimit"], json!(64));
}

#[tokio::test]
async fn requirements_memory_can_raise_the_default_but_not_exceed_total() {
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (docker, manager, _dir) = manager_on(&host).await;

    manager
        .install(
            &plugin("", "  requirements: {memory: 9G}\n"),
            &[],
            super::langs(),
        )
        .await
        .unwrap();

    assert_eq!(host_config(&docker)["Memory"], json!(9 * GIB));
}

#[tokio::test]
async fn unknown_host_memory_does_not_set_a_memory_limit() {
    let host = FakeHost::empty();
    let (docker, manager, _dir) = manager_on(&host).await;

    manager
        .install(&plugin("", ""), &[], super::langs())
        .await
        .unwrap();

    let host = host_config(&docker);
    assert_eq!(host["Memory"], Value::Null);
    assert_eq!(host["MemorySwap"], Value::Null);
    assert_eq!(host["CpuShares"], json!(512));
}

#[tokio::test]
async fn configured_slice_is_the_container_parent() {
    let host = FakeHost::empty();
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir)
        .with_host(host.paths())
        .with_plugin_slice(Some("wonton_plugins.slice".to_owned()));

    manager
        .install(&plugin("", ""), &[], super::langs())
        .await
        .unwrap();

    assert_eq!(
        host_config(&docker)["CgroupParent"],
        json!("wonton_plugins.slice")
    );
}

#[tokio::test]
async fn invalid_resource_values_name_the_service_and_field() {
    for (field, value) in [
        ("mem_limit", "0"),
        ("mem_reservation", "bad"),
        ("cpus", "0"),
        ("cpu_shares", "0"),
        ("pids_limit", "0"),
    ] {
        let host = FakeHost::empty();
        let (_docker, manager, _dir) = manager_on(&host).await;
        let compose = plugin(&format!("    {field}: {value}\n"), "");
        let error = manager
            .install(&compose, &[], super::langs())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("web"), "{field}: {error}");
        assert!(error.contains(field), "{field}: {error}");
    }
}

#[tokio::test]
async fn owner_limits_update_existing_container_without_recreating_it() {
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (docker, manager, _dir) = manager_on(&host).await;
    let manager = Arc::new(manager);
    manager
        .install(&plugin("", ""), &[], super::langs())
        .await
        .unwrap();
    let id = super::super::PluginId::parse("app.test").unwrap();
    manager
        .set_limits(
            id,
            wonton_protocol::PluginLimits {
                memory: Some(6 * GIB as u64),
                cpu_percent: Some(150),
            },
            Default::default(),
        )
        .await
        .unwrap();
    let updated = host_config(&docker);
    assert_eq!(updated["Memory"], json!(6 * GIB));
    assert_eq!(updated["MemorySwap"], json!(6 * GIB));
    assert_eq!(updated["NanoCpus"], json!(1_500_000_000i64));
    assert!(
        docker
            .writes()
            .iter()
            .any(|write| write == "POST /containers/wonton-app-test--web/update")
    );
}

#[tokio::test]
async fn restoring_cpu_limit_uses_all_host_cpus() {
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (docker, manager, _dir) = manager_on(&host).await;
    let manager = Arc::new(manager);
    manager
        .install(&plugin("", ""), &[], super::langs())
        .await
        .unwrap();
    let id = super::super::PluginId::parse("app.test").unwrap();
    // 没设过 CPU 上限：不发 `NanoCpus`，不给容器平白加一个配额。
    manager
        .set_limits(id.clone(), memory_only(6 * GIB as u64), Default::default())
        .await
        .unwrap();
    assert_eq!(host_config(&docker)["NanoCpus"], Value::Null);

    manager
        .set_limits(id.clone(), limits(6 * GIB as u64, 150), Default::default())
        .await
        .unwrap();
    manager
        .set_limits(
            id,
            wonton_protocol::PluginLimits::default(),
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(host_config(&docker)["NanoCpus"], json!(32_000_000_000i64));
}

#[tokio::test]
async fn invalid_owner_limit_is_rejected_before_record_change() {
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (_docker, manager, dir) = manager_on(&host).await;
    let manager = Arc::new(manager);
    manager
        .install(&plugin("", ""), &[], super::langs())
        .await
        .unwrap();
    let error = manager
        .set_limits(
            super::super::PluginId::parse("app.test").unwrap(),
            wonton_protocol::PluginLimits {
                memory: Some(5 * 1024 * 1024),
                cpu_percent: None,
            },
            Default::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    let record = record_of(&dir, &super::super::PluginId::parse("app.test").unwrap());
    assert_eq!(record.limits, Default::default());
}

#[tokio::test]
async fn restoring_memory_goes_back_to_the_compose_value() {
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (docker, manager, _dir) = manager_on(&host).await;
    let manager = Arc::new(manager);
    let id = PluginId::parse("app.test").unwrap();
    manager
        .install(&plugin("    mem_limit: 256m\n", ""), &[], super::langs())
        .await
        .unwrap();
    manager
        .set_limits(id.clone(), memory_only(6 * GIB as u64), Default::default())
        .await
        .unwrap();
    manager
        .set_limits(
            id,
            wonton_protocol::PluginLimits::default(),
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(host_config(&docker)["Memory"], json!(256 * 1024 * 1024));
    assert_eq!(host_config(&docker)["MemorySwap"], json!(256 * 1024 * 1024));
}

#[tokio::test]
async fn limits_survive_a_core_restart_and_a_rebuild() {
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (docker, manager, dir) = manager_on(&host).await;
    let engine = manager.engine.clone();
    let id = PluginId::parse("app.test").unwrap();
    manager
        .install(&plugin("", ""), &[], super::langs())
        .await
        .unwrap();
    Arc::new(manager)
        .set_limits(id.clone(), limits(6 * GIB as u64, 150), Default::default())
        .await
        .unwrap();

    let restarted = Arc::new(manager_with(engine, &dir).with_host(host.paths()));
    restarted.restore().await.unwrap();
    let host = host_config(&docker);
    assert_eq!(host["Memory"], json!(6 * GIB));
    assert_eq!(host["NanoCpus"], json!(1_500_000_000i64));
    assert_eq!(record_of(&dir, &id).limits, limits(6 * GIB as u64, 150));
}

#[tokio::test]
async fn limits_of_a_missing_plugin_are_reported() {
    let host = FakeHost::empty();
    let (_docker, manager, _dir) = manager_on(&host).await;
    let manager = Arc::new(manager);
    let error = manager
        .set_limits(
            PluginId::parse("app.missing").unwrap(),
            Default::default(),
            Default::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "not_installed");
}

#[tokio::test]
async fn limits_are_refused_while_the_plugin_is_busy() {
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (_docker, manager, _dir) = manager_on(&host).await;
    let manager = Arc::new(manager);
    let id = PluginId::parse("app.test").unwrap();
    manager
        .install(&plugin("", ""), &[], super::langs())
        .await
        .unwrap();
    manager
        .plugins
        .write()
        .unwrap()
        .get_mut(&id)
        .unwrap()
        .status = PluginStatus::Upgrading;

    let error = manager
        .set_limits(id, memory_only(6 * GIB as u64), Default::default())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "conflict");
}

#[tokio::test]
async fn restore_default_without_host_memory_is_reported_and_keeps_the_record() {
    let host = FakeHost::empty();
    let (docker, manager, dir) = manager_on(&host).await;
    let manager = Arc::new(manager);
    let id = PluginId::parse("app.test").unwrap();
    manager
        .install(&plugin("", ""), &[], super::langs())
        .await
        .unwrap();
    let set = memory_only(6 * GIB as u64);
    manager
        .set_limits(id.clone(), set, Default::default())
        .await
        .unwrap();
    let updates = |docker: &FakeDocker| {
        let writes = docker.writes();
        writes.iter().filter(|w| w.ends_with("/update")).count()
    };
    let before = updates(&docker);
    // compose 也没写 mem_limit：恢复默认算出来是不设，Docker 又把 0 当“不改”，如实报错。
    let error = manager
        .set_limits(id.clone(), Default::default(), Default::default())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "unsupported", "{error}");
    assert_eq!(updates(&docker), before);
    assert_eq!(record_of(&dir, &id).limits, set);
}

#[tokio::test]
async fn a_bot_plugin_can_change_its_limits() {
    // 核心不存 Bot 的令牌，真的环境凑不齐插值，但改上限不需要它们。
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let id = PluginId::parse("app.test").unwrap();
    let source = package(
        "  web:\n    image: example/web\n    environment:\n      TOKEN: ${WONTON_MATRIX_ACCESS_TOKEN}",
        "  bot: {display_name: {zh: 测试, en: Test}}\n  permissions: [matrix.bot]",
    );
    Store::open(dir.path().join("plugins"))
        .unwrap()
        .save(&InstalledRecord {
            id: id.clone(),
            version: "1.0.0".into(),
            source,
            files: Default::default(),
            granted_permissions: vec![Permission::MatrixBot],
            enabled_permissions: Vec::new(),
            limits: Default::default(),
            lan_webui: false,
            relays: Vec::new(),
            firewalled: false,
            proxied: false,
            oidc_client_secret: None,
            installed_at: 1,
            entry_label: None,
            origin: None,
            written_by: None,
            ask_every_update: false,
            family: false,
            settings: Default::default(),
            settings_changed_ms: None,
            static_files: Default::default(),
            hosted: Vec::new(),
        })
        .unwrap();
    let web = container_name("app-test", "web");
    docker.add_container(&web, "app.test");
    let manager = Arc::new(manager_with(engine, &dir).with_host(host.paths()));
    manager.restore().await.unwrap();

    manager
        .set_limits(id, memory_only(6 * GIB as u64), Default::default())
        .await
        .unwrap();
    assert_eq!(host_config(&docker)["Memory"], json!(6 * GIB));
}

#[tokio::test]
async fn a_failing_container_update_names_the_service_and_keeps_the_record() {
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (docker, manager, dir) = manager_on(&host).await;
    let manager = Arc::new(manager);
    let id = PluginId::parse("app.test").unwrap();
    let compose = package(
        "  web:\n    image: example/web\n  worker:\n    image: example/worker\n",
        "  permissions: []",
    );
    manager
        .install(&compose, &[], super::langs())
        .await
        .unwrap();
    docker.fail_updates(Some("boom"));

    let error = manager
        .set_limits(id.clone(), memory_only(6 * GIB as u64), Default::default())
        .await
        .unwrap_err();
    // 内层是 from_bollard 的 rejected（context 由调用点拼），外层补「Service web：」。
    let rejected = super::message(
        "plugin-install-docker-rejected",
        &[
            (
                "context",
                &super::message(
                    "plugin-settings-limits-update-container",
                    &[("name", "wonton-app-test--web")],
                ),
            ),
            ("error", "boom"),
        ],
    );
    let expected = super::message(
        "plugin-install-service-error",
        &[("service", "web"), ("error", &rejected)],
    );
    assert!(error.to_string().contains(&expected), "{error}");
    // 记录先写：重试是幂等的，第二次没有 fail_updates 就能全部更新。
    assert_eq!(record_of(&dir, &id).limits, memory_only(6 * GIB as u64));
    docker.fail_updates(None);
    manager
        .set_limits(id, memory_only(6 * GIB as u64), Default::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn an_oom_event_adds_a_notice_and_another_instances_container_is_ignored() {
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (docker, manager, _dir) = manager_on(&host).await;
    let manager = Arc::new(manager);
    let id = PluginId::parse("app.test").unwrap();
    manager
        .install(&plugin("", ""), &[], super::langs())
        .await
        .unwrap();
    manager.spawn_oom_watch();

    let notices = wait_for_notice(&docker, &manager, &id, &container_name("app-test", "web")).await;
    let en = Languages::from_header(Some("en"));
    let limit = crate::l10n::text(
        &en,
        "plugin-install-oom-limit-service",
        &[("size", "8 GB".into())],
    );
    let expected = crate::l10n::text(
        &en,
        "plugin-install-oom-notice",
        &[("service", "web".into()), ("limit", limit.into())],
    );
    assert_eq!(notices, [expected.as_str()], "{notices:?}");

    // 别的实例的容器：inspect 得到，但标签的实例名对不上，忽略。
    docker.add_container("wonton-other--web", "app.other");
    docker.emit_oom("wonton-other--web");
    // 没有插件标签的容器：同样忽略。
    docker.add_container("stray", "");
    docker.emit_oom("stray");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let view = manager.installed_plugin(&id, &Default::default()).unwrap();
    assert_eq!(view.notices.len(), 1, "{:?}", view.notices);
}

#[tokio::test]
async fn an_oom_in_the_shared_slice_says_the_total_may_be_what_ran_out() {
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(
        manager_with(engine, &dir)
            .with_host(host.paths())
            .with_plugin_slice(Some("wonton_plugins.slice".to_owned())),
    );
    let id = PluginId::parse("app.test").unwrap();
    manager
        .install(&plugin("", ""), &[], super::langs())
        .await
        .unwrap();
    manager.spawn_oom_watch();

    let notices = wait_for_notice(&docker, &manager, &id, &container_name("app-test", "web")).await;

    assert_eq!(
        notices,
        [crate::l10n::text(
            &Languages::from_header(Some("en")),
            "plugin-install-oom-notice",
            &[
                ("service", "web".into()),
                (
                    "limit",
                    crate::l10n::text(
                        &Languages::from_header(Some("en")),
                        "plugin-install-oom-limit-both",
                        &[("size", "8 GB".into())]
                    )
                    .into()
                ),
            ],
        )
        .as_str()],
        "{notices:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn oom_notices_expire_after_24h() {
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (docker, manager, _dir) = manager_on(&host).await;
    let manager = Arc::new(manager);
    let id = PluginId::parse("app.test").unwrap();
    manager
        .install(&plugin("", ""), &[], super::langs())
        .await
        .unwrap();
    manager.spawn_oom_watch();
    let notices = wait_for_notice(&docker, &manager, &id, &container_name("app-test", "web")).await;
    assert!(!notices.is_empty());

    tokio::time::advance(Duration::from_secs(25 * 60 * 60)).await;
    let view = manager.installed_plugin(&id, &Default::default()).unwrap();
    assert!(view.notices.is_empty(), "{:?}", view.notices);
}

#[tokio::test]
async fn raising_the_limit_clears_the_oom_notice() {
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (docker, manager, _dir) = manager_on(&host).await;
    let manager = Arc::new(manager);
    let id = PluginId::parse("app.test").unwrap();
    manager
        .install(&plugin("", ""), &[], super::langs())
        .await
        .unwrap();
    manager.spawn_oom_watch();
    assert!(
        !wait_for_notice(&docker, &manager, &id, &container_name("app-test", "web"))
            .await
            .is_empty()
    );

    manager
        .set_limits(id.clone(), memory_only(16 * GIB as u64), Default::default())
        .await
        .unwrap();
    let view = manager.installed_plugin(&id, &Default::default()).unwrap();
    assert!(view.notices.is_empty(), "{:?}", view.notices);
}

#[tokio::test]
async fn an_oomkilled_container_is_degraded_with_the_memory_reason() {
    let host = FakeHost::empty();
    host.memory_total_kib(10 * 1024 * 1024);
    let (docker, manager, dir) = manager_on(&host).await;
    let engine = manager.engine.clone();
    manager
        .install(&plugin("", ""), &[], super::langs())
        .await
        .unwrap();

    let web = container_name("app-test", "web");
    docker.set_state(
        &web,
        json!({"Status": "restarting", "Running": false, "OOMKilled": true, "ExitCode": 137}),
    );
    // 运行中的插件被 OOM：定期核对（watch → check_now）标为异常，不依赖事件。
    manager.watch_containers().await;
    let view = only(manager.list());
    let PluginStatus::Degraded { reason } = view.status else {
        panic!("{:?}", view.status);
    };
    let en = Languages::from_header(Some("en"));
    let expected = crate::l10n::text(
        &en,
        "plugin-install-oom-reason",
        &[
            ("service", "web".into()),
            (
                "limit",
                crate::l10n::text(
                    &en,
                    "plugin-install-oom-limit-service",
                    &[("size", "8 GB".into())],
                )
                .into(),
            ),
        ],
    );
    assert!(reason.contains(&expected), "{reason}");
    assert!(reason.contains("8 GB"), "{reason}");

    // 核心重启后的核对（restore → check_now）同样如此。
    let restarted = manager_with(engine, &dir).with_host(host.paths());
    restarted.restore().await.unwrap();
    let view = only(restarted.list());
    let PluginStatus::Degraded { reason } = view.status else {
        panic!("{:?}", view.status);
    };
    assert!(reason.contains(&expected), "{reason}");
}
