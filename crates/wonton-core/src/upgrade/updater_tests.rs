//! 升级器的行为测试：给定主机的情况，断言结果、状态文件和对主机的副作用。

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use wonton_protocol::{CoreUpgradeFailure as Failure, CoreUpgradePhase as Phase};

use super::backup;
use super::fake_host::{FakeHost, NEW, OLD, supports};
use super::host::Component;
use super::state::{
    self, ImageRef, Layout, MAX_UPDATER_STARTS, Outcome, UpgradeState, WINDOW_LIMIT,
};
use super::updater;
use crate::l10n;
use crate::plugin::languages::Languages;

struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Scene {
    _temp: Temp,
    layout: Layout,
    host: FakeHost,
}

/// 测试没写主人的语言文件：升级器文字按英文（期望值由目录格式化出来）。
fn langs() -> Languages {
    Languages::from_header(None)
}

const ENV: &str = "WONTON_CORE_SOURCE=reg/core:beta\nWONTON_CORE_IMAGE=reg/core@sha256:old\n\
                   WONTON_INSTALL_DIR=/opt/wonton\nWONTON_DATA_DIR=/var/lib/wonton\nWONTON_PORT=8480\n\
                   WONTON_TUNNEL_PORT=8482\nWONTON_BASE_DOMAIN=wonton.localhost\n";

fn scene() -> Scene {
    let root =
        std::env::temp_dir().join(format!("wonton-updater-{}", crate::secret::random_token()));
    let layout = Layout {
        install_dir: root.join("opt"),
        data_dir: root.join("data"),
    };
    fs::create_dir_all(&layout.install_dir).unwrap();
    fs::create_dir_all(layout.core_dir()).unwrap();
    fs::write(layout.env_path(), ENV).unwrap();
    fs::write(layout.install_dir.join("compose.yaml"), "compose of old").unwrap();
    fs::write(layout.core_dir().join("devices.json"), "paired before").unwrap();
    let host = FakeHost::installed(&layout);
    let scene = Scene {
        _temp: Temp(root),
        layout,
        host,
    };
    scene.begin(Vec::new());
    scene
}

impl Scene {
    fn begin(&self, migrates: Vec<String>) {
        let image = |image: &str, digest: &str, version: &str| ImageRef {
            image: image.to_owned(),
            digest: Some(digest.to_owned()),
            version: Some(version.to_owned()),
        };
        let fresh = UpgradeState::new(
            "u1".to_owned(),
            image(OLD, "sha256:old", "0.1.0"),
            image(NEW, "sha256:new", "0.2.0"),
            migrates,
            self.host.now.load(std::sync::atomic::Ordering::SeqCst),
        );
        state::save(&self.layout.state_path(), &fresh, &langs()).unwrap();
    }

    async fn run(&self) {
        updater::run(
            &self.host,
            self.layout.clone(),
            langs(),
            Duration::from_secs(180),
        )
        .await
        .unwrap();
    }

    /// 跑到主机上设好的停住的地方，升级器在那一刻没了。
    async fn run_until_killed(&self) {
        tokio::select! {
            result = self.run() => panic!("没有走到要停的地方：{result:?}"),
            () = self.host.stalled.notified() => {}
        }
    }

    fn state(&self) -> UpgradeState {
        state::load(&self.layout.state_path(), &langs())
            .unwrap()
            .unwrap()
    }

    fn env_image(&self) -> String {
        let text = fs::read_to_string(self.layout.env_path()).unwrap();
        let image = text
            .lines()
            .rev()
            .find_map(|l| l.strip_prefix("WONTON_CORE_IMAGE="));
        image.unwrap().to_owned()
    }

    fn compose(&self) -> String {
        fs::read_to_string(self.layout.install_dir.join("compose.yaml")).unwrap()
    }

    fn backup_dir(&self) -> PathBuf {
        self.layout.backup_root().join("sha256:old")
    }

    /// 升级期间写了东西：回滚后要退掉。
    fn write_during_upgrade(&self) {
        fs::write(self.layout.core_dir().join("devices.json"), "paired during").unwrap();
    }
}

fn devices(scene: &Scene) -> String {
    fs::read_to_string(scene.layout.core_dir().join("devices.json")).unwrap()
}

#[tokio::test]
async fn a_healthy_new_version_completes() {
    let s = scene();
    s.run().await;

    let end = s.state();
    assert_eq!(end.result, Some(Outcome::Completed));
    assert_eq!(end.problem, None);
    assert_eq!(s.env_image(), NEW);
    assert_eq!(s.host.core_image(), NEW);
    assert_eq!(s.compose(), format!("compose of {NEW}"));
    assert!(end.rollback_images.contains(&OLD.to_owned()));
    // 手动退回要用。
    assert!(s.backup_dir().join("core/devices.json").exists());
    assert!(s.backup_dir().join("install/.env").exists());
    // 做完把自己的策略改成 no，下次不再被拉起来。
    assert_eq!(
        s.host.log().last().unwrap(),
        "disable-restart wonton-updater"
    );

    // 顺序：先备份，再改 .env 和执行 init，新核心起来之前旧核心是停着的。
    let log = s.host.log();
    let at = |op: &str| {
        log.iter()
            .position(|l| l == op)
            .unwrap_or_else(|| panic!("没有 {op}：{log:?}"))
    };
    assert!(at("stop wonton-core") < at(&format!("run wonton-updater-init {NEW}")));
    assert!(at(&format!("run wonton-updater-init {NEW}")) < at(&format!("up {NEW}")));
}

/// 观察 3 分钟：上限里的 18 分钟不会把健康的升级截断，也不会提前结束。
#[tokio::test]
async fn a_new_version_is_observed_for_the_whole_period() {
    let s = scene();
    let before = s.host.now.load(std::sync::atomic::Ordering::SeqCst);
    s.run().await;
    let after = s.host.now.load(std::sync::atomic::Ordering::SeqCst);
    assert!(after - before >= 180, "只观察了 {} 秒", after - before);
}

#[tokio::test]
async fn a_new_version_that_fails_its_health_check_is_rolled_back() {
    let s = scene();
    s.host.world().unhealthy.insert(NEW.to_owned());
    s.host.world().stall_at = Some(format!("up {NEW}"));
    s.run_until_killed().await;
    s.write_during_upgrade();
    s.run().await;

    let end = s.state();
    assert_eq!(end.result, Some(Outcome::RolledBack));
    assert_eq!(end.problem.unwrap().reason, Failure::Unhealthy);
    assert_eq!(s.env_image(), OLD);
    assert_eq!(s.host.core_image(), OLD);
    // 回滚写回的是备份里的 compose 文件和配置，不是只改 .env。
    assert_eq!(s.compose(), "compose of old");
    assert_eq!(devices(&s), "paired before");
}

#[tokio::test]
async fn a_crash_while_being_observed_is_rolled_back() {
    let s = scene();
    s.host.world().crashes.insert(NEW.to_owned());
    s.run().await;

    let end = s.state();
    assert_eq!(end.result, Some(Outcome::RolledBack));
    assert_eq!(end.problem.unwrap().reason, Failure::Crashed);
    assert_eq!(s.host.core_image(), OLD);
}

#[tokio::test]
async fn an_image_that_cannot_be_pulled_fails_without_touching_anything() {
    let s = scene();
    {
        let mut world = s.host.world();
        world.images.remove("tuwunel:1");
        world
            .pull_errors
            .insert("tuwunel:1".to_owned(), "连不上仓库".to_owned());
    }
    s.run().await;

    let end = s.state();
    assert_eq!(end.result, Some(Outcome::Failed));
    let problem = end.problem.unwrap();
    assert_eq!(problem.reason, Failure::RegistryUnavailable);
    assert!(
        problem.message.contains("tuwunel:1"),
        "要写明拉不到哪个镜像：{}",
        problem.message
    );
    assert_eq!(s.env_image(), OLD);
    assert!(!s.host.did("stop wonton-core"));
    assert!(!s.backup_dir().exists());
}

/// `init` 拒绝（比如要先升到某个版本）时还没开始迁移，可以回滚，原因写在结果里。
#[tokio::test]
async fn a_refused_init_is_rolled_back_with_its_reason() {
    let s = scene();
    s.host
        .world()
        .init_errors
        .insert(NEW.to_owned(), "要先升到 0.15".to_owned());
    s.run().await;

    let end = s.state();
    assert_eq!(end.result, Some(Outcome::RolledBack));
    let problem = end.problem.unwrap();
    assert_eq!(problem.reason, Failure::InitFailed);
    assert!(
        problem.message.contains("要先升到 0.15"),
        "{}",
        problem.message
    );
    assert_eq!(s.env_image(), OLD);
}

/// 宿主重启、升级器被杀后，Docker 把它拉起来，它按状态文件接着做。
#[tokio::test]
async fn an_updater_killed_midway_continues_from_the_state_file() {
    for stall in [
        format!("run wonton-updater-images {NEW}"),
        "stop wonton-core".to_owned(),
        format!("run wonton-updater-init {NEW}"),
        format!("up {NEW}"),
    ] {
        let s = scene();
        s.host.world().stall_at = Some(stall.clone());
        s.run_until_killed().await;
        assert_eq!(s.state().result, None, "停在 {stall}");

        s.run().await;
        let end = s.state();
        assert_eq!(end.result, Some(Outcome::Completed), "停在 {stall}");
        assert_eq!(s.host.core_image(), NEW, "停在 {stall}");
    }
}

/// `observing` 重启后先 `up` 再观察满 3 分钟，时限不重新计时。
#[tokio::test]
async fn an_updater_killed_while_observing_brings_services_up_and_observes_again() {
    let s = scene();
    s.run().await;
    let mut observing = s.state();
    observing.result = None;
    observing.enter(
        Phase::Observing,
        s.host.now.load(std::sync::atomic::Ordering::SeqCst),
    );
    state::save(&s.layout.state_path(), &observing, &langs()).unwrap();
    s.host.world().log.clear();
    let before = s.host.now.load(std::sync::atomic::Ordering::SeqCst);

    s.run().await;

    assert!(s.host.did(&format!("up {NEW}")));
    assert!(s.host.now.load(std::sync::atomic::Ordering::SeqCst) - before >= 180);
    assert_eq!(s.state().result, Some(Outcome::Completed));
    assert_eq!(
        s.state().observing_started_at,
        observing.observing_started_at
    );
}

/// 同一阶段升级器最多启动 3 次，超过按这个阶段超时处理。
#[tokio::test]
async fn an_updater_that_keeps_crashing_gives_up_on_the_phase() {
    let s = scene();
    s.host.world().stall_at = Some(format!("up {NEW}"));
    s.run_until_killed().await;
    let mut crashing = s.state();
    assert_eq!(crashing.phase, Phase::Starting);
    crashing.updater_starts = MAX_UPDATER_STARTS;
    state::save(&s.layout.state_path(), &crashing, &langs()).unwrap();

    s.run().await;

    let end = s.state();
    assert_eq!(end.result, Some(Outcome::RolledBack));
    let problem = end.problem.unwrap();
    assert_eq!(problem.reason, Failure::UpdaterFailed);
    assert!(
        problem
            .message
            .contains(&l10n::text(&langs(), "upgrade-updater-crashed", &[])),
        "{}",
        problem.message
    );
}

#[tokio::test]
async fn canceling_while_pulling_stops_before_anything_is_touched() {
    let s = scene();
    {
        let mut world = s.host.world();
        world.images.remove("tuwunel:1");
        world.cancel_at = Some(("pull tuwunel:1".to_owned(), s.layout.cancel_path("u1")));
    }
    s.run().await;

    assert_eq!(s.state().result, Some(Outcome::Canceled));
    assert_eq!(s.env_image(), OLD);
    assert!(!s.host.did("stop wonton-core"));
}

/// 旧的取消标记带着别的编号，取消不了这一次。
#[tokio::test]
async fn a_cancel_marker_of_another_upgrade_is_ignored() {
    let s = scene();
    fs::write(s.layout.cancel_path("older"), b"").unwrap();
    s.run().await;
    assert_eq!(s.state().result, Some(Outcome::Completed));
}

/// 磁盘不够时如实报告，什么都不删，也不停服务。
#[tokio::test]
async fn too_little_disk_fails_and_deletes_nothing() {
    let s = scene();
    let old = backup::create_dir(&s.layout, "sha256:older", "u0").unwrap();
    fs::write(old.join("data"), "x").unwrap();
    s.host.world().free = 0;
    s.run().await;

    let end = s.state();
    assert_eq!(end.result, Some(Outcome::Failed));
    assert_eq!(end.problem.unwrap().reason, Failure::NoSpace);
    assert!(old.join("data").exists());
    assert!(!s.host.did("stop wonton-core"));
    assert_eq!(s.env_image(), OLD);
}

/// 旧备份能腾出空间时算作可用空间：够了才删。
#[tokio::test]
async fn old_backups_count_as_free_space() {
    let s = scene();
    let old = backup::create_dir(&s.layout, "sha256:older", "u0").unwrap();
    fs::write(old.join("data"), vec![0u8; 100_000]).unwrap();
    s.host.world().free = 0;
    s.run().await;

    assert_eq!(s.state().result, Some(Outcome::Completed));
    assert!(!old.exists());
}

/// 迁移开始后出问题不回滚：把旧版本配到迁移过的数据上只会更糟。
#[tokio::test]
async fn an_irreversible_version_that_fails_stays_put_and_reports_uncertain() {
    let s = scene();
    s.begin(vec!["tuwunel".to_owned()]);
    s.host.world().unhealthy.insert(NEW.to_owned());
    s.run().await;

    let end = s.state();
    assert_eq!(end.result, Some(Outcome::Unresolved));
    assert_eq!(s.env_image(), NEW);
    assert_eq!(s.host.core_image(), NEW);
    assert!(s.host.did(&format!("run wonton-updater-volume-copy {OLD}")));
    assert!(
        s.host.did("stop wonton-tuwunel-1"),
        "用着卷的服务要先停：{:?}",
        s.host.log()
    );
    // 这份备份不能被下次升级删掉。
    let (deletable, _) = backup::deletable(&s.layout).unwrap();
    assert!(deletable.is_empty(), "{deletable:?}");
}

/// `starting` 之前失败还可以回滚：迁移还没开始。
#[tokio::test]
async fn an_irreversible_version_rolls_back_before_the_migration_starts() {
    let s = scene();
    s.begin(vec!["tuwunel".to_owned()]);
    s.host
        .world()
        .init_errors
        .insert(NEW.to_owned(), "拒绝".to_owned());
    s.run().await;
    assert_eq!(s.state().result, Some(Outcome::RolledBack));
    assert_eq!(s.env_image(), OLD);
}

#[tokio::test]
async fn a_failed_rollback_reports_both_reasons_and_keeps_the_backup() {
    let s = scene();
    {
        let mut world = s.host.world();
        world.unhealthy.insert(NEW.to_owned());
        world.up_error = Some("旧版本也起不来".to_owned());
    }
    s.run().await;

    let end = s.state();
    assert_eq!(end.result, Some(Outcome::RollbackFailed));
    let message = end.problem.unwrap().message;
    // 两条原因都在：回滚时的失败（起不回旧版本），和当初触发回滚的失败。文字由目录出，
    // 这两条是假主机给的原始错误（数据），断言它们被带上。
    assert!(message.contains("旧版本也起不来"), "{message}");
    assert!(
        message.contains("服务 core 不健康"),
        "要带上回滚的起因：{message}"
    );
    assert!(backup::deletable(&s.layout).unwrap().0.is_empty());
}

/// 超过总时限：动过的回滚。
#[tokio::test]
async fn running_out_of_the_window_rolls_back() {
    let s = scene();
    s.host.world().stall_at = Some(format!("up {NEW}"));
    s.run_until_killed().await;
    s.host.advance(WINDOW_LIMIT + Duration::from_secs(1));
    s.run().await;

    let end = s.state();
    assert_eq!(end.result, Some(Outcome::RolledBack));
    assert_eq!(end.problem.unwrap().reason, Failure::TimedOut);
    assert_eq!(s.env_image(), OLD);
}

/// 备份失败时还没改东西：启动回原样，`failed`。
#[tokio::test]
async fn a_window_that_runs_out_while_backing_up_restarts_the_old_version() {
    let s = scene();
    s.host.world().stall_at = Some("stop wonton-core".to_owned());
    s.run_until_killed().await;
    assert_eq!(s.state().phase, Phase::BackingUp);
    s.host.advance(WINDOW_LIMIT + Duration::from_secs(1));
    s.run().await;

    let end = s.state();
    assert_eq!(end.result, Some(Outcome::Failed));
    assert_eq!(end.problem.unwrap().reason, Failure::TimedOut);
    assert!(s.host.did(&format!("up {OLD}")));
    assert_eq!(s.env_image(), OLD);
}

/// 切到一个镜像之前处理它不支持的组件：compose 项目 `down`，经 API 建的删容器。升级和回滚都一样。
#[tokio::test]
async fn components_the_target_does_not_support_are_removed() {
    let s = scene();
    supports(&s.host, NEW, "wonton-egress");
    s.host.world().components = vec![
        Component::Project("wonton-clients".to_owned()),
        Component::Container {
            component: "wonton-egress".to_owned(),
            container: "id-egress".to_owned(),
        },
    ];
    s.run().await;
    assert_eq!(s.state().result, Some(Outcome::Completed));
    assert!(s.host.did("down wonton-clients"));
    assert!(!s.host.did("remove-container id-egress"));

    let s = scene();
    supports(&s.host, NEW, "wonton-egress");
    {
        let mut world = s.host.world();
        world.unhealthy.insert(NEW.to_owned());
        world.components = vec![Component::Container {
            component: "wonton-egress".to_owned(),
            container: "id-egress".to_owned(),
        }];
    }
    s.run().await;
    assert_eq!(s.state().result, Some(Outcome::RolledBack));
    assert!(
        s.host.did("remove-container id-egress"),
        "旧版本没有声明，回滚时删掉"
    );
}

/// `completed` 后只留这一次的备份，之前留着的也不用了；上一次的回滚镜像可以删了。
#[tokio::test]
async fn completing_cleans_up_what_is_no_longer_needed() {
    let s = scene();
    let kept = backup::create_dir(&s.layout, "sha256:older", "u0").unwrap();
    backup::mark_keep(&kept).unwrap();
    let mut fresh = s.state();
    fresh.stale_images = vec![
        "reg/core@sha256:older".to_owned(),
        OLD.to_owned(),
        "tuwunel:1".to_owned(),
    ];
    state::save(&s.layout.state_path(), &fresh, &langs()).unwrap();

    s.run().await;

    assert_eq!(s.state().result, Some(Outcome::Completed));
    assert!(!kept.exists());
    assert!(s.backup_dir().exists());
    assert!(s.host.did("remove-image reg/core@sha256:older"));
    assert!(!s.host.did(&format!("remove-image {OLD}")), "回滚要用");
    assert!(!s.host.did("remove-image tuwunel:1"), "正在用");
}

/// 升级器启动后发现状态已结束：把重启策略改成 `no`，退出。
#[tokio::test]
async fn an_updater_that_finds_a_finished_state_just_stops_restarting() {
    let s = scene();
    let mut ended = s.state();
    ended.result = Some(Outcome::Completed);
    state::save(&s.layout.state_path(), &ended, &langs()).unwrap();
    s.run().await;
    assert_eq!(s.host.log(), vec!["disable-restart wonton-updater"]);
    assert_eq!(s.env_image(), OLD);
}
