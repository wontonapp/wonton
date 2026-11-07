//! `wonton-core updater <安装目录> <数据目录>`：真正动手升级的一次性容器（ADR-0025）。镜像就是发起升级的那个核心的
//! 镜像：回滚的逻辑来自已知能用的旧版本。
//!
//! 每进入一个阶段先写状态文件，阶段都能重做：宿主重启、升级器崩溃后 Docker 把它拉起来，它按状态文件接着做。

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use wonton_l10n::Arg;
use wonton_protocol::{
    CoreUpgradeFailure as Failure, CoreUpgradePhase as Phase, CoreUpgradeProblem,
};

use super::backup::{compose_volume, volume_mount};
use super::envfile::EnvFile;
use super::host::{Host, Mount, OneShot, ServiceContainer};
use super::observe::Baseline;
use super::state::{self, Layout, MAX_UPDATER_STARTS, OBSERVE, Outcome, UpgradeState, problem};
use super::{CORE_SERVICE, PROJECT, UPDATER_CONTAINER, backup, components, compose_images};
use crate::l10n;
use crate::plugin::languages::Languages;

/// 拉镜像、观察时多久看一次。
const POLL: Duration = Duration::from_secs(5);
/// 一次 `up` 最多等这么久。
const UP_WAIT: u64 = 600;
/// 跑 `init`、`compose-images` 这种一下就结束的命令的时限。
const QUICK_JOB: Duration = Duration::from_secs(5 * 60);
/// 备份的余量，和文件服务一样（ADR-0020）。
const SPACE_MARGIN_PERCENT: u64 = 5;

/// 这一步没走下去的原因。
enum Halt {
    Problem(CoreUpgradeProblem),
    Canceled,
    /// 写不了状态文件这样的错误：结果也写不下来，退出，等 Docker 把升级器拉起来再试。
    Fatal(anyhow::Error),
}

impl From<anyhow::Error> for Halt {
    fn from(e: anyhow::Error) -> Self {
        Self::Fatal(e)
    }
}

impl From<CoreUpgradeProblem> for Halt {
    fn from(p: CoreUpgradeProblem) -> Self {
        Self::Problem(p)
    }
}

fn halt(reason: Failure, message: impl Into<String>) -> Halt {
    Halt::Problem(problem(reason, message))
}

/// 升级器的入口。`observe` 是观察多久，正式运行是 [`OBSERVE`]。文字按主人的语言写：App 的管理请求会把它存下来
/// （ADR-0043），升级器进程没有插件管理器，进来时已经读好。
pub async fn run(
    host: &dyn Host,
    layout: Layout,
    langs: Languages,
    observe: Duration,
) -> anyhow::Result<()> {
    let state = state::load(&layout.state_path(), &langs)?;
    if let Some(mut state) = state.filter(|s| !s.is_ended()) {
        state.updater_starts += 1;
        state::save(&layout.state_path(), &state, &langs)?;
        let mut updater = Updater {
            host,
            layout,
            langs,
            observe,
            state,
            resumed: true,
        };
        updater.drive().await?;
    }
    // 先写结果，再改重启策略，再退出：核心看到升级器不在了时，结果已经写下了。
    host.disable_restart(UPDATER_CONTAINER)
        .await
        .context("把升级器的重启策略改成 no")?;
    Ok(())
}

/// 正式运行时观察多久。
pub const fn observe_period() -> Duration {
    OBSERVE
}

struct Updater<'a> {
    host: &'a dyn Host,
    layout: Layout,
    langs: Languages,
    observe: Duration,
    state: UpgradeState,
    /// 这个阶段是升级器重启后接着做的，不是刚从上一个阶段走过来的。
    resumed: bool,
}

impl Updater<'_> {
    /// 这个进程写进状态文件的文字（App 的详情里显示）按主人的语言写（ADR-0043）。
    fn msg(&self, id: &str, args: &[(&str, Arg<'_>)]) -> String {
        l10n::text(&self.langs, id, args)
    }

    async fn drive(&mut self) -> anyhow::Result<()> {
        if self.state.updater_starts > MAX_UPDATER_STARTS {
            let crashed = problem(
                Failure::UpdaterFailed,
                self.msg("upgrade-updater-crashed", &[]),
            );
            self.handle(crashed).await?;
        }
        while !self.state.is_ended() {
            let step = match self.state.phase {
                Phase::Pulling => self.pulling().await,
                Phase::BackingUp => self.backing_up().await,
                Phase::Switching => self.switching().await,
                Phase::Starting => self.starting().await,
                Phase::Observing => self.observing().await,
                Phase::RollingBack => self.rolling_back().await,
                Phase::Other => Err(halt(
                    Failure::Protocol,
                    self.msg("upgrade-phase-unreadable", &[]),
                )),
            };
            self.resumed = false;
            match step {
                Ok(()) => {}
                Err(Halt::Problem(p)) => self.handle(p).await?,
                Err(Halt::Canceled) => self.finish(Outcome::Canceled, None)?,
                Err(Halt::Fatal(e)) => return Err(e),
            }
        }
        if self.state.result == Some(Outcome::Completed) {
            self.clean_up_after_completed().await;
        }
        Ok(())
    }

    /// 这个阶段失败或超时了（ADR-0025 “步骤”那张表的最后一列）。
    async fn handle(&mut self, failure: CoreUpgradeProblem) -> anyhow::Result<()> {
        tracing::error!(phase = ?self.state.phase, reason = ?failure.reason, message = %failure.message, "升级这一步失败了");
        match self.state.phase {
            Phase::Pulling => self.finish(Outcome::Failed, Some(failure)),
            Phase::BackingUp => self.undo_backing_up(failure).await,
            Phase::Switching => self.start_rollback(failure),
            // 迁移可能已经开始了：把旧版本配到迁移过的数据上只会更糟。
            Phase::Starting | Phase::Observing if self.state.is_irreversible() => {
                self.finish(Outcome::Unresolved, Some(failure))
            }
            Phase::Starting | Phase::Observing => self.start_rollback(failure),
            Phase::RollingBack => {
                let failure = match self.state.problem.take() {
                    Some(first) => problem(
                        failure.reason,
                        self.msg(
                            "upgrade-rollback-cause",
                            &[
                                ("message", failure.message.as_str().into()),
                                ("cause", first.message.as_str().into()),
                            ],
                        ),
                    ),
                    None => failure,
                };
                self.finish(Outcome::RollbackFailed, Some(failure))
            }
            Phase::Other => self.finish(Outcome::Unknown, Some(failure)),
        }
    }

    fn start_rollback(&mut self, failure: CoreUpgradeProblem) -> anyhow::Result<()> {
        self.state.problem = Some(failure);
        self.enter(Phase::RollingBack)
    }

    /// 什么都还没改，只是可能停了服务：启动回原样。起不来就是真的坏了，不能说“什么都没动”。
    async fn undo_backing_up(&mut self, failure: CoreUpgradeProblem) -> anyhow::Result<()> {
        let wait = Duration::from_secs(UP_WAIT);
        match self.host.compose_up(&self.layout.install_dir, wait).await {
            Ok(()) => self.finish(Outcome::Failed, Some(failure)),
            Err(e) => {
                let message = self.msg(
                    "upgrade-restore-also-failed",
                    &[
                        ("message", failure.message.as_str().into()),
                        ("error", e.to_string().into()),
                    ],
                );
                self.finish(
                    Outcome::RollbackFailed,
                    Some(problem(failure.reason, message)),
                )
            }
        }
    }

    fn enter(&mut self, phase: Phase) -> anyhow::Result<()> {
        self.state.enter(phase, self.host.now());
        self.save()
    }

    fn save(&self) -> anyhow::Result<()> {
        state::save(&self.layout.state_path(), &self.state, &self.langs)
    }

    fn finish(
        &mut self,
        outcome: Outcome,
        failure: Option<CoreUpgradeProblem>,
    ) -> anyhow::Result<()> {
        if outcome.keeps_backup()
            && let Some(dir) = &self.state.backup_dir
            && dir.exists()
        {
            backup::mark_keep(dir)?;
        }
        if failure.is_some() {
            self.state.problem = failure;
        }
        self.state.result = Some(outcome);
        tracing::info!(?outcome, problem = ?self.state.problem, "升级结束");
        self.save()
    }

    fn canceled(&self) -> bool {
        self.layout.cancel_path(&self.state.id).exists()
    }

    fn check_canceled(&self) -> Result<(), Halt> {
        if self.canceled() {
            return Err(Halt::Canceled);
        }
        Ok(())
    }

    async fn pulling(&mut self) -> Result<(), Halt> {
        self.check_canceled()?;
        let target = self.state.to.image.clone();
        self.ensure_image(&target).await?;
        let listed = self
            .run_once(
                "wonton-updater-images",
                &target,
                &["compose-images"],
                Vec::new(),
            )
            .await?;
        if listed.exit_code != 0 {
            let message = self.msg(
                "upgrade-list-images-failed",
                &[
                    ("image", target.as_str().into()),
                    ("error", listed.stderr.as_str().into()),
                ],
            );
            return Err(halt(Failure::Upstream, message));
        }
        for image in listed.stdout.lines().filter(|l| !l.trim().is_empty()) {
            self.ensure_image(image.trim()).await?;
        }
        // 升级器就是旧版本：旧版本要用的镜像写在它自己的 compose 文件里。
        let mut rollback = compose_images::images(crate::install::COMPOSE)
            .context("解析这个版本的 compose 文件")?;
        rollback.push(self.state.from.image.clone());
        self.state.rollback_images = rollback;
        // 查完立刻写：之后才来的取消已经晚了。
        self.check_canceled()?;
        self.enter(Phase::BackingUp)?;
        Ok(())
    }

    /// 本机没有就拉。拉取时查取消和时限。
    async fn ensure_image(&self, image: &str) -> Result<(), Halt> {
        let present = self.host.image(image).await.map_err(|e| {
            halt(
                Failure::Upstream,
                self.msg(
                    "upgrade-inspect-image-failed",
                    &[("image", image.into()), ("error", e.to_string().into())],
                ),
            )
        })?;
        if present.is_some() {
            return Ok(());
        }
        let limit = self.state.phase_limit_at();
        let ignore = |_| {};
        let mut pull = self.host.pull(image, &ignore);
        loop {
            tokio::select! {
                biased;
                pulled = &mut pull => {
                    return pulled.map_err(|e| halt(
                        Failure::RegistryUnavailable,
                        self.msg(
                            "upgrade-pull-image-failed",
                            &[("image", image.into()), ("error", e.to_string().into())],
                        ),
                    ));
                }
                () = self.host.sleep(POLL) => {
                    self.check_canceled()?;
                    if self.host.now() > limit {
                        return Err(halt(
                            Failure::TimedOut,
                            self.msg("upgrade-pull-image-timed-out", &[("image", image.into())]),
                        ));
                    }
                }
            }
        }
    }

    async fn run_once(
        &self,
        name: &str,
        image: &str,
        cmd: &[&str],
        mounts: Vec<Mount>,
    ) -> Result<super::host::OneShotOutput, Halt> {
        let job = OneShot {
            name: name.to_owned(),
            image: image.to_owned(),
            entrypoint: None,
            cmd: cmd.iter().map(|c| (*c).to_owned()).collect(),
            mounts,
            timeout: QUICK_JOB,
        };
        self.host.run_once(&job).await.map_err(|e| {
            halt(
                Failure::Upstream,
                self.msg(
                    "upgrade-run-in-image-failed",
                    &[
                        ("image", image.into()),
                        ("cmd", format!("{cmd:?}").into()),
                        ("error", e.to_string().into()),
                    ],
                ),
            )
        })
    }

    /// 总时限还剩多少：从进入 `backing_up` 算到 `starting` 结束。
    fn window_left(&self) -> Result<u64, Halt> {
        let left = self.state.phase_limit_at().saturating_sub(self.host.now());
        if left == 0 {
            return Err(halt(
                Failure::TimedOut,
                self.msg("upgrade-window-exceeded", &[]),
            ));
        }
        Ok(left)
    }

    fn backup_key(&self) -> String {
        let from = &self.state.from;
        from.digest
            .clone()
            .unwrap_or_else(|| from.image.replace('/', "_"))
    }

    async fn backing_up(&mut self) -> Result<(), Halt> {
        self.window_left()?;
        let volumes = self.volume_sizes().await?;
        self.make_room(volumes.iter().map(|(_, size)| size).sum())
            .await?;
        self.stop_services_for_backup().await?;
        let dir = backup::create_dir(&self.layout, &self.backup_key(), &self.state.id)?;
        let limit = self.state.phase_limit_at();
        let expired = || self.host.now() > limit;
        backup::copy_files(&self.layout, &dir, &expired).map_err(|e| {
            halt(
                Failure::Upstream,
                self.msg(
                    "upgrade-backup-failed",
                    &[("error", format!("{e:#}").into())],
                ),
            )
        })?;
        for (volume, _) in &volumes {
            self.back_up_volume(volume, &dir).await?;
        }
        backup::mark_complete(&dir)?;
        self.state.backup_dir = Some(dir);
        self.enter(Phase::Switching)?;
        Ok(())
    }

    /// 这一版声明会迁移的卷和它们的大小。
    async fn volume_sizes(&self) -> Result<Vec<(String, u64)>, Halt> {
        let mut sizes = Vec::new();
        for volume in &self.state.migrates {
            let job = OneShot {
                name: "wonton-updater-volume-size".to_owned(),
                image: self.state.from.image.clone(),
                entrypoint: Some(vec!["du".to_owned()]),
                cmd: vec!["-sb".to_owned(), "/volume".to_owned()],
                mounts: vec![volume_mount(volume, true)],
                timeout: QUICK_JOB,
            };
            let output = self.host.run_once(&job).await.map_err(|e| {
                halt(
                    Failure::Upstream,
                    self.msg(
                        "upgrade-volume-size-failed",
                        &[("volume", volume.into()), ("error", e.to_string().into())],
                    ),
                )
            })?;
            let size = output
                .stdout
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok());
            let size = size.ok_or_else(|| {
                halt(
                    Failure::Upstream,
                    self.msg(
                        "upgrade-volume-size-unknown",
                        &[
                            ("volume", volume.into()),
                            ("error", output.stderr.as_str().into()),
                        ],
                    ),
                )
            })?;
            sizes.push((volume.clone(), size));
        }
        Ok(sizes)
    }

    /// 放得下才删旧备份；放不下什么都不删。
    async fn make_room(&self, volumes: u64) -> Result<(), Halt> {
        let size = backup::files_size(&self.layout)? + volumes;
        let needed = size + size / 100 * SPACE_MARGIN_PERCENT;
        let free = self.host.free_bytes(&self.layout.core_dir()).map_err(|e| {
            halt(
                Failure::Upstream,
                self.msg(
                    "upgrade-disk-space-failed",
                    &[("error", e.to_string().into())],
                ),
            )
        })?;
        let (old, old_size) = backup::deletable(&self.layout)?;
        if free + old_size < needed {
            let message = self.msg(
                "upgrade-backup-no-space",
                &[
                    ("needed", needed.to_string().into()),
                    ("free", free.to_string().into()),
                    ("old", old_size.to_string().into()),
                ],
            );
            return Err(halt(Failure::NoSpace, message));
        }
        backup::remove_all(&old)?;
        Ok(())
    }

    /// 停 `core` 和用着要备份的卷的服务。
    async fn stop_services_for_backup(&self) -> Result<(), Halt> {
        let containers = self.containers().await?;
        for container in containers.iter().filter(|c| {
            c.service == CORE_SERVICE
                || self
                    .state
                    .migrates
                    .iter()
                    .any(|v| c.volumes.contains(&compose_volume(v)))
        }) {
            self.host.stop(&container.name).await.map_err(|e| {
                halt(
                    Failure::Upstream,
                    self.msg(
                        "upgrade-stop-service-failed",
                        &[
                            ("service", container.service.as_str().into()),
                            ("error", e.to_string().into()),
                        ],
                    ),
                )
            })?;
        }
        Ok(())
    }

    async fn back_up_volume(&self, volume: &str, dir: &std::path::Path) -> Result<(), Halt> {
        let target = dir.join(backup::VOLUMES).join(volume);
        fs::create_dir_all(&target).with_context(|| format!("建立 {}", target.display()))?;
        let left = self.window_left()?;
        // 用当前镜像跑：升级器容器里挂不上卷。
        let job = OneShot {
            name: "wonton-updater-volume-copy".to_owned(),
            image: self.state.from.image.clone(),
            entrypoint: Some(vec!["cp".to_owned()]),
            cmd: vec![
                "-a".to_owned(),
                "/volume/.".to_owned(),
                "/backup/".to_owned(),
            ],
            mounts: vec![
                volume_mount(volume, true),
                Mount::Bind {
                    source: target,
                    target: PathBuf::from("/backup"),
                    read_only: false,
                },
            ],
            timeout: Duration::from_secs(left),
        };
        let output = self.host.run_once(&job).await.map_err(|e| {
            halt(
                Failure::Upstream,
                self.msg(
                    "upgrade-backup-volume-failed",
                    &[("volume", volume.into()), ("error", e.to_string().into())],
                ),
            )
        })?;
        if output.exit_code != 0 {
            let message = self.msg(
                "upgrade-backup-volume-failed",
                &[
                    ("volume", volume.into()),
                    ("error", output.stderr.as_str().into()),
                ],
            );
            return Err(halt(Failure::Upstream, message));
        }
        Ok(())
    }

    async fn switching(&mut self) -> Result<(), Halt> {
        self.window_left()?;
        self.switch_image().await?;
        self.enter(Phase::Starting)?;
        Ok(())
    }

    /// 改 `.env`，在新镜像里执行 `init`。重做也不怕：`init` 不动已有的密钥。
    async fn switch_image(&self) -> Result<(), Halt> {
        let path = self.layout.env_path();
        let env = EnvFile::read(&path).map_err(|e| halt(Failure::Upstream, format!("{e:#}")))?;
        if let Some(key) = env.missing_required_key() {
            return Err(halt(
                Failure::Upstream,
                self.msg(
                    "upgrade-env-missing-key",
                    &[
                        ("path", path.display().to_string().into()),
                        ("key", key.into()),
                    ],
                ),
            ));
        }
        let domain = env.get("WONTON_BASE_DOMAIN").unwrap_or_default().to_owned();
        let target = self.state.to.image.clone();
        env.write_image(&path, &target)?;
        let install = self.layout.install_dir.display().to_string();
        let mounts = vec![
            Mount::same_path(&self.layout.install_dir, false),
            Mount::same_path(&self.layout.data_dir, true),
        ];
        let output = self
            .run_once(
                "wonton-updater-init",
                &target,
                &["init", &install, &domain],
                mounts,
            )
            .await?;
        if output.exit_code != 0 {
            // 跳不过去的版本在这里拒绝，并写明先升到哪个版本（已定 3）。
            let message = self.msg(
                "upgrade-init-failed",
                &[("error", output.stderr.trim().into())],
            );
            return Err(halt(Failure::InitFailed, message));
        }
        Ok(())
    }

    async fn starting(&mut self) -> Result<(), Halt> {
        let left = self.window_left()?;
        if self.resumed {
            self.switch_image().await?;
        }
        let target = self.state.to.image.clone();
        components::prepare(self.host, &target, &self.langs)
            .await
            .map_err(|e| {
                halt(
                    Failure::Upstream,
                    self.msg(
                        "upgrade-components-failed",
                        &[("target", "new".into()), ("error", e.to_string().into())],
                    ),
                )
            })?;
        self.up(left).await?;
        self.verify_core_image(&target, None).await?;
        self.enter(Phase::Observing)?;
        Ok(())
    }

    async fn up(&self, left: u64) -> Result<(), Halt> {
        let wait = Duration::from_secs(UP_WAIT.min(left));
        self.host
            .compose_up(&self.layout.install_dir, wait)
            .await
            .map_err(|e| {
                halt(
                    Failure::Unhealthy,
                    self.msg("upgrade-up-failed", &[("error", e.to_string().into())]),
                )
            })
    }

    /// 容器必须真的跑在 `image` 上：引用会被移走，比的是移不走的镜像 ID。`expected_id` 是升级开始时记下的实际镜像；
    /// 没有（旧版本写的状态文件）时按引用查本机镜像。
    async fn verify_core_image(&self, image: &str, expected_id: Option<&str>) -> Result<(), Halt> {
        let expected = match expected_id {
            Some(id) => id.to_owned(),
            None => self.local_image_id(image).await?,
        };
        let containers = self.containers().await?;
        match containers.iter().find(|c| c.service == CORE_SERVICE) {
            Some(core) if core.image_id == expected => Ok(()),
            Some(core) => Err(halt(
                Failure::Unhealthy,
                self.msg(
                    "upgrade-core-image-wrong",
                    &[
                        ("actual", core.image.as_str().into()),
                        ("actual_id", core.image_id.as_str().into()),
                        ("image", image.into()),
                        ("image_id", expected.as_str().into()),
                    ],
                ),
            )),
            None => Err(halt(
                Failure::Unhealthy,
                self.msg("upgrade-core-container-missing", &[]),
            )),
        }
    }

    async fn local_image_id(&self, image: &str) -> Result<String, Halt> {
        let info = self.host.image(image).await.map_err(|e| {
            halt(
                Failure::Upstream,
                self.msg(
                    "upgrade-inspect-image-failed",
                    &[("image", image.into()), ("error", e.to_string().into())],
                ),
            )
        })?;
        info.map(|info| info.id).ok_or_else(|| {
            halt(
                Failure::Unhealthy,
                self.msg("upgrade-no-image", &[("image", image.into())]),
            )
        })
    }

    async fn containers(&self) -> Result<Vec<ServiceContainer>, Halt> {
        self.host.project_containers(PROJECT).await.map_err(|e| {
            halt(
                Failure::Upstream,
                self.msg(
                    "upgrade-containers-failed",
                    &[("error", e.to_string().into())],
                ),
            )
        })
    }

    async fn observing(&mut self) -> Result<(), Halt> {
        let limit = self.state.phase_limit_at();
        if self.resumed {
            let left = limit.saturating_sub(self.host.now());
            if left == 0 {
                return Err(halt(
                    Failure::TimedOut,
                    self.msg("upgrade-observe-timed-out", &[]),
                ));
            }
            self.up(left).await?;
        }
        let baseline = Baseline::of(&self.containers().await?);
        let until = self.host.now() + self.observe.as_secs();
        loop {
            if let Some(found) = baseline.problem(&self.containers().await?, &self.langs) {
                return Err(halt(Failure::Crashed, found));
            }
            if self.host.now() >= until {
                break;
            }
            self.host.sleep(POLL).await;
        }
        self.finish(Outcome::Completed, None)?;
        Ok(())
    }

    async fn rolling_back(&mut self) -> Result<(), Halt> {
        let limit = self.state.phase_limit_at();
        let left = limit.saturating_sub(self.host.now());
        if left == 0 {
            return Err(halt(
                Failure::TimedOut,
                self.msg("upgrade-rollback-timed-out", &[]),
            ));
        }
        let Some(dir) = self.state.backup_dir.clone() else {
            return Err(halt(
                Failure::Upstream,
                self.msg("upgrade-no-backup-dir", &[]),
            ));
        };
        let from = self.state.from.image.clone();
        // 新版本的核心可能没起来，也可能正在反复重启：停掉它再动它的数据。
        for container in self
            .containers()
            .await?
            .iter()
            .filter(|c| c.service == CORE_SERVICE)
        {
            self.host.stop(&container.name).await.map_err(|e| {
                halt(
                    Failure::Upstream,
                    self.msg(
                        "upgrade-stop-core-failed",
                        &[("error", e.to_string().into())],
                    ),
                )
            })?;
        }
        backup::restore_files(&self.layout, &dir, &self.langs).map_err(|e| {
            halt(
                Failure::Upstream,
                self.msg(
                    "upgrade-restore-failed",
                    &[("error", format!("{e:#}").into())],
                ),
            )
        })?;
        components::prepare(self.host, &from, &self.langs)
            .await
            .map_err(|e| {
                halt(
                    Failure::Upstream,
                    self.msg(
                        "upgrade-components-failed",
                        &[("target", "old".into()), ("error", e.to_string().into())],
                    ),
                )
            })?;
        self.up(left).await?;
        let recorded = self.state.from.image_id.as_deref();
        self.verify_core_image(&from, recorded).await?;
        self.finish(Outcome::RolledBack, None)?;
        Ok(())
    }

    /// 只留这一次的备份、当前用的和回滚要用的镜像。删不掉不影响升级的结果。
    async fn clean_up_after_completed(&self) {
        let dir = self.state.backup_dir.as_deref();
        if let Err(e) = backup::remove_others(&self.layout, dir) {
            tracing::warn!(error = %format!("{e:#}"), "清理旧备份失败");
        }
        let current = self.current_images().await;
        for image in &self.state.stale_images {
            if !current.contains(image)
                && !self.state.rollback_images.contains(image)
                && let Err(e) = self.host.remove_image(image).await
            {
                tracing::warn!(%image, error = %e, "删除旧镜像失败");
            }
        }
    }

    async fn current_images(&self) -> Vec<String> {
        match self.host.project_containers(PROJECT).await {
            Ok(containers) => containers.into_iter().map(|c| c.image).collect(),
            Err(e) => {
                tracing::warn!(error = %e, "查系统服务的容器失败");
                Vec::new()
            }
        }
    }
}
