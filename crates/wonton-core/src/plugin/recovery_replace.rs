//! 换一个 Bot 的容器：先把它的匿名卷记进记录、放上“待恢复”，再删旧容器，照记录接回同一批卷起新的。
//!
//! 新建、名单变了重建、失败后重试走同一条路：重试时记录里已经有卷，接回去的是原来那份数据。失败时限制
//! 不撤，本次创建了一半的容器由回滚删掉，没人会在恢复前把它起起来（见 [`super::member_bots`]）。

use wonton_protocol::InstallStage;

use super::bot_services::BotVars;
use super::health_gate;
use super::languages::Languages;
use super::lifecycle::Undo;
use super::manager::{InstallError, PluginManager};
use super::package::PluginPackage;
use super::render::{InstallContext, render};
use super::store::InstalledRecord;

impl PluginManager {
    pub(super) async fn deploy_bot(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
        bot: &str,
        runtime: BotVars,
    ) -> Result<(), InstallError> {
        let langs = self.owner_languages();
        let lan = self.lan_grant(package, record.lan_webui, &langs).await?;
        let mut ctx = self.rebuild_context(package, record, lan, None).await?;
        ctx.bots.insert(bot.to_owned(), runtime);
        let mut plan = render(package, &ctx, &langs)?;
        plan.containers.retain(|c| c.bot.as_deref() == Some(bot));
        let id = &package.manifest.id;
        let old = self.bot_containers(id, bot).await?;
        self.stage(id, InstallStage::Preparing);
        self.prepare_images(&mut plan, Some(id), &langs).await?;
        let mut undo = Undo::default();
        let deployed = self
            .start_bot_replacement(id, bot, &ctx, &mut plan, &old, &mut undo, &langs)
            .await;
        let Err(error) = deployed else {
            return Ok(());
        };
        // 半路起的容器也可能已经建了卷：先记下再回滚删掉，重试照样接得回。
        let created: Vec<String> = plan.containers.iter().map(|c| c.name.clone()).collect();
        let _ = self.remember_bot_volumes(id, bot, &created).await;
        let _ = self.mark_restricted(id, bot, error.to_string()).await;
        let failures = self.rollback(undo, &langs).await;
        if failures.is_empty() {
            Err(error)
        } else {
            Err(error.with_note(
                &langs,
                &format!("回滚未完成，可能有残留：{}", failures.join("；")),
            ))
        }
    }

    /// 记卷、删旧容器、接回旧卷、起新容器。成功才清限制。
    #[allow(clippy::too_many_arguments)]
    async fn start_bot_replacement(
        &self,
        id: &super::manifest::PluginId,
        bot: &str,
        ctx: &InstallContext,
        plan: &mut super::render::Deployment,
        old: &[String],
        undo: &mut Undo,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        // 先记卷再放限制再删容器：删到一半进程没了，卷也还在记录里，核心重启不会把它起起来。
        self.remember_bot_volumes(id, bot, old).await?;
        self.mark_pending(id, bot).await?;
        self.stage(id, InstallStage::Preparing);
        self.prepare_dirs(ctx, &plan.dirs, undo, langs)?;
        self.attach_bot_volumes(id, bot, plan).await?;
        // 卷留着删（`v` 关了）：要接回的匿名卷这时只挂在旧容器上，带卷删会把它们删掉，新的按名字只能
        // 建出一个空卷。
        for name in old {
            self.engine.remove_container_keep_volumes(name).await?;
        }
        self.rewrite_image_healthchecks(ctx, plan).await?;
        let deadline =
            health_gate::deadline(plan, self.cfg.install_timeout, tokio::time::Instant::now());
        self.stage(id, InstallStage::Starting);
        self.start_containers(plan, Some(id), undo, deadline, langs)
            .await?;
        let started: Vec<String> = plan.containers.iter().map(|c| c.name.clone()).collect();
        self.recovered_bot(id, bot, &started).await
    }
}
