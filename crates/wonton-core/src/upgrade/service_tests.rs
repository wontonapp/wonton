//! 核心这边的行为测试：查新版本、发起、取消、看住升级器、冻结插件。

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use wonton_protocol::{
    CoreUpgradeCheck, CoreUpgradeFailure as Failure, CoreUpgradePhase as Phase, CoreUpgradeRunState,
};

use super::fake_host::{FakeHost, NEW, OLD, label};
use super::host::{ContainerInfo, ImageInfo};
use super::service::{UpgradeConfig, UpgradeError, Upgrades};
use super::state::{self, ImageRef, Layout, Outcome, UpgradeState, WATCHDOG_MARGIN};
use super::{LABEL_MIGRATIONS, LABEL_VERSION, UPDATER_CONTAINER, backup};
use crate::plugin::languages::Languages;

const SOURCE: &str = "reg/core:stable";

struct Scene {
    root: PathBuf,
    layout: Layout,
    host: Arc<FakeHost>,
    upgrades: Arc<Upgrades>,
}

impl Drop for Scene {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn config(layout: &Layout) -> UpgradeConfig {
    UpgradeConfig {
        source: Some(SOURCE.to_owned()),
        install_dir: Some(layout.install_dir.clone()),
        data_dir: layout.data_dir.clone(),
        core_container: Some("wonton-core".to_owned()),
        has_instance: false,
    }
}

/// 测试没写主人的语言文件：后台文字按英文（期望值都由目录格式化出来）。
fn langs() -> Languages {
    Languages::from_header(None)
}

fn scene() -> Scene {
    let root =
        std::env::temp_dir().join(format!("wonton-upgrades-{}", crate::secret::random_token()));
    let layout = Layout {
        install_dir: root.join("opt"),
        data_dir: root.join("data"),
    };
    fs::create_dir_all(layout.upgrade_dir()).unwrap();
    let host = Arc::new(FakeHost::installed(&layout));
    host.world()
        .registry
        .insert(SOURCE.to_owned(), "sha256:new".to_owned());
    let upgrades = Arc::new(Upgrades::new(config(&layout), host.clone()));
    Scene {
        root,
        layout,
        host,
        upgrades,
    }
}

fn in_progress(phase: Phase, now: u64) -> UpgradeState {
    let image = |image: &str, digest: &str| ImageRef {
        image: image.to_owned(),
        digest: Some(digest.to_owned()),
        version: None,
        image_id: None,
    };
    let mut state = UpgradeState::new(
        "u1".to_owned(),
        image(OLD, "sha256:old"),
        image(NEW, "sha256:new"),
        Vec::new(),
        now,
    );
    state.enter(phase, now);
    state
}

impl Scene {
    fn now(&self) -> u64 {
        self.host.now.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn write_state(&self, state: &UpgradeState) {
        state::save(&self.layout.state_path(), state, &langs()).unwrap();
    }

    fn state(&self) -> UpgradeState {
        state::load(&self.layout.state_path(), &langs())
            .unwrap()
            .unwrap()
    }

    fn updater(&self, running: bool, restart_disabled: bool) {
        self.host.world().updater = Some(ContainerInfo {
            image_id: "id-core".to_owned(),
            config_image: "id-core".to_owned(),
            running,
            restart_disabled,
        });
    }

    /// 查新版本，把 [`NEW`] 预下载好。
    async fn ready(&self) {
        self.upgrades.check_now(false).await;
        let status = self.upgrades.status(&langs()).await.unwrap();
        assert!(
            matches!(status.check, CoreUpgradeCheck::Ready { .. }),
            "{:?}",
            status.check
        );
    }

    fn frozen(&self) -> bool {
        *self.upgrades.frozen().borrow()
    }
}

#[tokio::test]
async fn a_new_digest_on_the_channel_is_offered_with_what_the_image_says() {
    let s = scene();
    label(&s.host, NEW, LABEL_MIGRATIONS, "tuwunel@0.5.0");
    s.upgrades.check_now(false).await;

    let status = s.upgrades.status(&langs()).await.unwrap();
    assert_eq!(status.channel.as_deref(), Some("stable"));
    assert_eq!(status.current.version.as_deref(), Some("0.1.0"));
    assert_eq!(status.current.digest.as_deref(), Some("sha256:old"));
    let CoreUpgradeCheck::Ready { offer } = status.check else {
        panic!("{:?}", status.check);
    };
    assert_eq!(offer.digest, "sha256:new");
    assert_eq!(offer.version.as_deref(), Some("0.2.0"));
    assert_eq!(offer.migrates, ["tuwunel"]);
}

#[tokio::test]
async fn the_running_digest_is_up_to_date() {
    let s = scene();
    s.host
        .world()
        .registry
        .insert(SOURCE.to_owned(), "sha256:old".to_owned());
    s.upgrades.check_now(false).await;
    assert_eq!(
        s.upgrades.status(&langs()).await.unwrap().check,
        CoreUpgradeCheck::UpToDate
    );
}

#[tokio::test]
async fn an_older_version_on_the_channel_is_not_an_upgrade() {
    let s = scene();
    label(&s.host, NEW, LABEL_VERSION, "0.0.9");
    s.upgrades.check_now(false).await;
    assert_eq!(
        s.upgrades.status(&langs()).await.unwrap().check,
        CoreUpgradeCheck::UpToDate
    );
    assert!(s.host.did(&format!("remove-image {NEW}")));
}

#[tokio::test]
async fn a_registry_that_cannot_be_reached_is_reported_as_such() {
    let s = scene();
    s.host.world().registry_error = Some("连不上".to_owned());
    s.upgrades.check_now(false).await;
    let CoreUpgradeCheck::Failed { problem } = s.upgrades.status(&langs()).await.unwrap().check
    else {
        panic!();
    };
    assert_eq!(problem.reason, Failure::RegistryUnavailable);
}

/// 后台文字按主人的语言写（ADR-0043）：主人最近用 ja 就是日文，没见过的语言回退英文。
/// 期望值由目录本身格式化出来，再断言两种请求真的给出两种语言。
#[tokio::test]
async fn background_problems_follow_the_owner_language() {
    let mut messages = Vec::new();
    for (header, tag) in [("ja", "ja"), ("fr-FR", "en")] {
        let s = scene();
        let core = s.layout.data_dir.join("core");
        std::fs::create_dir_all(&core).unwrap();
        std::fs::write(core.join("owner.json"), r#"{"username": "owner"}"#).unwrap();
        let dir = crate::plugin::store::people_languages_dir(&s.layout.data_dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::plugin::store::write_json(
            &dir.join("people-languages.json"),
            &std::collections::BTreeMap::from([("owner".to_owned(), header.to_owned())]),
        )
        .unwrap();
        s.host.world().registry_error = Some("连不上".to_owned());
        s.upgrades.check_now(false).await;
        let CoreUpgradeCheck::Failed { problem } = s.upgrades.status(&langs()).await.unwrap().check
        else {
            panic!();
        };
        let expected = crate::l10n::CATALOG.negotiate(&[tag]);
        assert_eq!(
            problem.message,
            crate::l10n::CATALOG.format(
                expected,
                "upgrade-latest-version-failed",
                &[("source", SOURCE.into()), ("error", "连不上".into())],
            ),
            "{header}"
        );
        messages.push(problem.message);
    }
    assert_ne!(messages[0], messages[1], "ja 和 en 要不同");
}

/// 后台自动下载：剩不到 5 GB 就不下载；主人手动下载按镜像大小加余量看。
#[tokio::test]
async fn automatic_downloads_keep_5_gb_free_but_manual_ones_only_need_room_for_the_image() {
    let s = scene();
    {
        let mut world = s.host.world();
        world.images.remove(NEW);
        world.free = 4_000_000_000;
        world.pullable.insert(NEW.to_owned(), ImageInfo::default());
    }
    s.upgrades.check_now(false).await;
    let CoreUpgradeCheck::DownloadFailed { digest, problem } =
        s.upgrades.status(&langs()).await.unwrap().check
    else {
        panic!();
    };
    assert_eq!(digest, "sha256:new");
    assert_eq!(problem.reason, Failure::NoSpace);
    assert!(!s.host.did(&format!("pull {NEW}")));

    s.upgrades.check_now(true).await;
    assert!(s.host.did(&format!("pull {NEW}")));
    assert!(matches!(
        s.upgrades.status(&langs()).await.unwrap().check,
        CoreUpgradeCheck::Ready { .. }
    ));

    let s = scene();
    {
        let mut world = s.host.world();
        world.images.remove(NEW);
        world.free = 100; // 当前镜像 100 字节，加余量放不下。
    }
    s.upgrades.check_now(true).await;
    let CoreUpgradeCheck::DownloadFailed { problem, .. } =
        s.upgrades.status(&langs()).await.unwrap().check
    else {
        panic!();
    };
    assert_eq!(problem.reason, Failure::NoSpace);
}

#[tokio::test]
async fn a_download_failure_is_shown_with_its_reason() {
    let s = scene();
    {
        let mut world = s.host.world();
        world.images.remove(NEW);
        world
            .pull_errors
            .insert(NEW.to_owned(), "连不上仓库".to_owned());
    }
    s.upgrades.check_now(false).await;
    let CoreUpgradeCheck::DownloadFailed { problem, .. } =
        s.upgrades.status(&langs()).await.unwrap().check
    else {
        panic!();
    };
    assert_eq!(problem.reason, Failure::RegistryUnavailable);
    assert!(
        problem.message.contains("连不上仓库"),
        "{}",
        problem.message
    );
}

/// 拉到更新的版本就删掉之前预下载的；不删正在运行的。
#[tokio::test]
async fn only_one_downloaded_version_is_kept() {
    let s = scene();
    s.ready().await;
    let newer = "reg/core@sha256:newer";
    {
        let mut world = s.host.world();
        world
            .registry
            .insert(SOURCE.to_owned(), "sha256:newer".to_owned());
        let mut info = ImageInfo::default();
        info.labels
            .insert(LABEL_VERSION.to_owned(), "0.3.0".to_owned());
        world.pullable.insert(newer.to_owned(), info);
    }
    s.upgrades.check_now(false).await;
    assert!(s.host.did(&format!("remove-image {NEW}")));
    assert!(!s.host.did(&format!("remove-image {OLD}")));
}

#[tokio::test]
async fn a_pinned_or_informal_install_says_so_and_cannot_upgrade() {
    let s = scene();
    let pinned = UpgradeConfig {
        source: Some("reg/core@sha256:old".to_owned()),
        ..config(&s.layout)
    };
    let pinned = Upgrades::new(pinned, s.host.clone());
    assert_eq!(
        pinned.status(&langs()).await.unwrap().check,
        CoreUpgradeCheck::Pinned
    );

    for informal in [
        UpgradeConfig {
            source: None,
            ..config(&s.layout)
        },
        UpgradeConfig {
            install_dir: None,
            ..config(&s.layout)
        },
        UpgradeConfig {
            has_instance: true,
            ..config(&s.layout)
        },
    ] {
        let informal = Upgrades::new(informal, s.host.clone());
        assert_eq!(
            informal.status(&langs()).await.unwrap().check,
            CoreUpgradeCheck::NotInstalled
        );
        assert!(matches!(
            informal.start("sha256:new", &langs()).await,
            Err(UpgradeError::NotReady(_))
        ));
    }
}

#[tokio::test]
async fn starting_launches_the_updater_from_the_running_image() {
    let s = scene();
    s.ready().await;
    let status = s.upgrades.start("sha256:new", &langs()).await.unwrap();

    let run = status.run.unwrap();
    assert_eq!(run.to.digest, "sha256:new");
    assert_eq!(run.from.digest.as_deref(), Some("sha256:old"));
    assert_eq!(
        s.state().from.image_id.as_deref(),
        Some("id-core"),
        "回滚要认得出升级前跑的是哪个镜像"
    );
    assert!(matches!(
        run.state,
        CoreUpgradeRunState::InProgress {
            phase: Phase::Pulling,
            ..
        }
    ));

    let spec = s.host.world().updater_spec.clone().unwrap();
    assert_eq!(spec.image_id, "id-core", "回滚的逻辑来自已知能用的旧版本");
    assert_eq!(spec.cmd[0], "updater");
    assert_eq!(s.state().to.image, NEW);
    assert!(!s.frozen(), "还在拉镜像，什么都没动");
}

/// 主人确认的就是装上的：digest 变了要重新确认。
#[tokio::test]
async fn starting_refuses_anything_but_what_was_confirmed() {
    let s = scene();
    let refused = s.upgrades.start("sha256:new", &langs()).await.unwrap_err();
    assert_eq!(refused.kind(), "not_ready");

    s.ready().await;
    let refused = s
        .upgrades
        .start("sha256:other", &langs())
        .await
        .unwrap_err();
    assert_eq!(refused.kind(), "changed");

    s.upgrades.start("sha256:new", &langs()).await.unwrap();
    let refused = s.upgrades.start("sha256:new", &langs()).await.unwrap_err();
    assert_eq!(refused.kind(), "busy");
}

#[tokio::test]
async fn an_updater_that_cannot_start_ends_as_failed_with_nothing_touched() {
    let s = scene();
    s.ready().await;
    s.host.world().updater_start_error = Some("Docker 拒绝".to_owned());
    let status = s.upgrades.start("sha256:new", &langs()).await.unwrap();
    assert!(matches!(
        status.run.unwrap().state,
        CoreUpgradeRunState::Failed { .. }
    ));
    // 可以再发起一次。
    s.host.world().updater_start_error = None;
    s.upgrades.start("sha256:new", &langs()).await.unwrap();
}

#[tokio::test]
async fn canceling_only_works_while_pulling() {
    let s = scene();
    let refused = s.upgrades.cancel(&langs()).await.unwrap_err();
    assert_eq!(refused.kind(), "too_late");

    s.ready().await;
    s.upgrades.start("sha256:new", &langs()).await.unwrap();
    s.upgrades.cancel(&langs()).await.unwrap();
    assert!(s.layout.cancel_path(&s.state().id).exists());

    let mut moved_on = s.state();
    moved_on.enter(Phase::BackingUp, s.now());
    s.write_state(&moved_on);
    let refused = s.upgrades.cancel(&langs()).await.unwrap_err();
    assert_eq!(refused.kind(), "too_late");
}

/// 新核心在观察期里启动：冻结插件，等到结果才放行。
#[tokio::test]
async fn a_core_restarted_during_an_upgrade_stays_frozen_until_there_is_a_result() {
    let s = scene();
    s.write_state(&in_progress(Phase::Observing, s.now()));
    s.updater(true, false);
    let restarted = Arc::new(Upgrades::new(config(&s.layout), s.host.clone()));
    let frozen = restarted.frozen();
    assert!(*frozen.borrow());

    let status = restarted.status(&langs()).await.unwrap();
    assert!(matches!(
        status.run.unwrap().state,
        CoreUpgradeRunState::InProgress {
            phase: Phase::Observing,
            ..
        }
    ));
    restarted.supervise().await;
    assert!(*frozen.borrow());

    let mut ended = s.state();
    ended.result = Some(Outcome::Completed);
    s.write_state(&ended);
    restarted.supervise().await;
    assert!(!*frozen.borrow());
    tokio::time::timeout(Duration::from_secs(5), restarted.wait_released())
        .await
        .unwrap();
}

/// 没有结束的升级时不查新版本；写下结果后恢复。
#[tokio::test]
async fn no_background_check_while_an_upgrade_is_running() {
    let s = scene();
    s.write_state(&in_progress(Phase::Pulling, s.now()));
    s.updater(true, false);
    s.upgrades.trigger_check(false);
    tokio::task::yield_now().await;
    assert_eq!(
        s.upgrades.status(&langs()).await.unwrap().check,
        CoreUpgradeCheck::NotChecked
    );

    let mut ended = s.state();
    ended.result = Some(Outcome::Failed);
    s.write_state(&ended);
    s.upgrades.trigger_check(false);
    assert_eq!(
        s.upgrades.status(&langs()).await.unwrap().check,
        CoreUpgradeCheck::Checking
    );
}

/// 容器不存在，或者已退出而且重启策略是 `no`：升级器不在了，写 `unknown`，备份留着，放行。
#[tokio::test]
async fn an_updater_that_is_gone_ends_as_unknown_and_keeps_the_backup() {
    for container in [None, Some(false)] {
        let s = scene();
        let dir = backup::create_dir(&s.layout, "sha256:old", "u1").unwrap();
        let mut state = in_progress(Phase::Starting, s.now());
        state.backup_dir = Some(dir);
        s.write_state(&state);
        if container.is_some() {
            s.updater(false, true);
        }

        s.upgrades.supervise().await;

        let end = s.state();
        assert_eq!(end.result, Some(Outcome::Unknown));
        assert_eq!(end.problem.unwrap().reason, Failure::UpdaterFailed);
        assert!(!s.frozen());
        assert!(backup::deletable(&s.layout).unwrap().0.is_empty());
    }
}

/// 容器在就是升级中：宿主刚重启时它可能还没被 Docker 拉起来。
#[tokio::test]
async fn an_exited_updater_that_docker_will_restart_is_still_upgrading() {
    let s = scene();
    s.write_state(&in_progress(Phase::Starting, s.now()));
    s.updater(false, false);
    s.upgrades.supervise().await;
    assert_eq!(s.state().result, None);
    assert!(s.frozen());
}

/// 升级器先写结果再退出：核心查完容器后重读，盖不掉它写的结果。
#[tokio::test]
async fn a_result_written_by_the_updater_is_never_overwritten() {
    let s = scene();
    let mut state = in_progress(Phase::Observing, s.now());
    state.result = Some(Outcome::Completed);
    s.write_state(&state);
    s.upgrades.supervise().await;
    assert_eq!(s.state().result, Some(Outcome::Completed));
}

/// 反复崩溃的升级器：超过上限加余量后改成 `no`，它停下，核心写 `unknown`。
#[tokio::test]
async fn the_watchdog_stops_a_crash_looping_updater() {
    let s = scene();
    s.write_state(&in_progress(Phase::Starting, s.now()));
    s.updater(true, false);

    s.upgrades.supervise().await;
    assert!(
        !s.host.did("disable-restart wonton-updater"),
        "没到时限不动它"
    );

    s.host.advance(Duration::from_secs(40 * 60 + 1));
    s.upgrades.supervise().await;
    assert!(s.host.did("disable-restart wonton-updater"));
    assert_eq!(s.state().result, None, "还在运行就先不动它");

    s.updater(false, true);
    s.upgrades.supervise().await;
    assert_eq!(s.state().result, Some(Outcome::Unknown));
}

/// 再过 10 分钟还在运行，说明它卡死了：停掉它，写 `unknown`。
#[tokio::test]
async fn the_watchdog_stops_an_updater_that_hangs() {
    let s = scene();
    s.write_state(&in_progress(Phase::Starting, s.now()));
    s.updater(true, true);
    s.host.advance(
        Duration::from_secs(30 * 60) + WATCHDOG_MARGIN + WATCHDOG_MARGIN + Duration::from_secs(1),
    );

    s.upgrades.supervise().await;

    assert!(s.host.did(&format!("stop {UPDATER_CONTAINER}")));
    assert_eq!(s.state().result, Some(Outcome::Unknown));
    assert!(!s.frozen());
}
