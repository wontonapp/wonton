use super::{
    Gate, InstallError, InstalledRecord, Languages, PluginId, PluginManager, PluginPackage,
    PluginStatus, reconcile_failed, render, unavailable, uses_models,
};

impl PluginManager {
    /// 网关启动后（安装、恢复、升级）为使用方对一次账：记着凭据的用原值重新签发并验证，对账失败的标为异常、
    /// 写明原因，按记下的值交给巡检重试（见 `watch_one`）；没接上的（装它的时候网关不在或签发失败）现在签发，
    /// 并按新变量重建运行中的容器。
    pub async fn reconcile_models(&self) -> Vec<(PluginId, InstallError)> {
        let langs = self.owner_languages();
        let mut failures = self.reconcile_model_keys(&langs).await;
        for (id, error) in &failures {
            let reason = reconcile_failed(&langs, &error.to_string());
            self.set(id, PluginStatus::Degraded { reason }, None);
            // 凭据对账失败不说明容器坏了：照样让巡检看着它，下一轮才有机会重试（#643）。
            self.follow_for_reconcile(id);
        }
        failures.extend(self.connect_unconnected_consumers(&langs).await);
        failures
    }

    async fn reconcile_model_keys(&self, langs: &Languages) -> Vec<(PluginId, InstallError)> {
        let Some(route) = self.models_route() else {
            return Vec::new();
        };
        let all = match self.store.model_keys().all() {
            Ok(all) => all,
            Err(e) => return vec![(route.id.clone(), e.into())],
        };
        let mut failures = Vec::new();
        for (id, mut file) in all {
            let mut problems = Vec::new();
            // 卸载时网关没在运行、没吊销成的：网关回来了补吊销，成功了就不必再重签。
            if file.revoke_pending {
                let mut failed = Vec::new();
                for name in file.keys.keys() {
                    if let Err(e) = self
                        .run_command(&route, &route.spec.credentials.revoke, name, None, langs)
                        .await
                    {
                        tracing::warn!(plugin = %id, credential = %name, error = %e, "补吊销模型网关凭据失败");
                        failed.push(Self::named(name, &e));
                    }
                }
                if failed.is_empty() {
                    match self.store.model_keys().remove(&id) {
                        Ok(()) => continue,
                        Err(e) => failures.push((id.clone(), e.into())),
                    }
                    continue;
                }
                failures.push((id.clone(), unavailable(failed.join("\n"))));
                continue;
            }
            for (name, value) in &file.keys {
                if let Err(e) = self.issue_verified(&route, name, value, langs).await {
                    tracing::warn!(plugin = %id, credential = %name, error = %e, "模型网关凭据对账失败");
                    problems.push(Self::named(name, &e));
                }
            }
            // 网关升级可能改了接口路径和默认模型：对账顺手把记下的变量刷新成现在的。
            let mut changed = false;
            if !file.keys.is_empty()
                && let Ok(Some(record)) = self.store.load(&id)
                && let Ok(mut package) =
                    PluginPackage::with_files(&record.source, record.files, &self.owner_languages())
            {
                package.origin = record.origin.clone();
                let vars = self.model_vars(&route, &package, &file.keys, &self.bot_names(&id));
                file.vars = Some(vars);
                file.problem = None;
                file.route = None;
                changed = true;
            }
            let problem = (!problems.is_empty()).then(|| problems.join("\n"));
            if file.reconcile_problem != problem {
                file.reconcile_problem = problem.clone();
                changed = true;
            }
            if changed && let Err(e) = self.store.model_keys().save(&id, &file) {
                tracing::warn!(plugin = %id, error = %e, "记下模型网关对账结果失败");
            }
            if let Some(problem) = problem {
                failures.push((id, unavailable(problem)));
            }
        }
        failures
    }

    /// 对账失败的使用方重新跟着容器巡检：容器回来、或下一次凭据重试成功时状态才有机会回去。
    fn follow_for_reconcile(&self, id: &PluginId) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(entry) = plugins.get_mut(id) {
            entry.follows_containers = true;
        }
    }

    /// 给没接上的使用方签发凭据：装它的时候网关不在或签发失败，变量是空的。签发成功后，运行中的按新变量
    /// 重建容器；没在运行的等下次启动时从记下的变量里拿。还接不上的记下原因，详情页的轻提示带它。
    async fn connect_unconnected_consumers(
        &self,
        langs: &Languages,
    ) -> Vec<(PluginId, InstallError)> {
        let Some(route) = self.models_route() else {
            return Vec::new();
        };
        // 用记录里的清单判断，不看店里的条目：restore 回来的插件没有那条，照样要接。
        let candidates: Vec<(PluginId, bool)> = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins
                .iter()
                .filter(|(id, entry)| {
                    **id != route.id
                        && matches!(entry.status, PluginStatus::Running | PluginStatus::Stopped)
                })
                .map(|(id, entry)| (id.clone(), entry.status == PluginStatus::Running))
                // 只补没接上的：已接上的对账刚把凭据对好，变量没变，容器不用动。
                .filter(|(id, _)| {
                    !self
                        .store
                        .model_keys()
                        .load(id)
                        .is_ok_and(|file| file.connected())
                })
                .collect()
        };
        let mut failures = Vec::new();
        for (id, running) in candidates {
            let Ok(Some(record)) = self.store.load(&id) else {
                continue;
            };
            let Ok(mut package) = PluginPackage::with_files(
                &record.source,
                record.files.clone(),
                &self.owner_languages(),
            ) else {
                continue;
            };
            package.origin = record.origin.clone();
            if !uses_models(&package.manifest) {
                continue;
            }
            let bot_names = self.bot_names(&id);
            match self
                .issue_consumer_credentials(&route, &package, langs, &bot_names)
                .await
            {
                Err(e) => {
                    tracing::warn!(plugin = %id, error = %e, "模型网关凭据签发失败，使用方保持没接上");
                    failures.push((id, e));
                }
                Ok(_) if running => match self.rebuild_for_models(&package, &record).await {
                    Ok(()) => tracing::info!(plugin = %id, "模型网关接上了，使用方已按新变量重建"),
                    Err(e) => {
                        tracing::error!(plugin = %id, error = %e, "模型网关接上了，重建使用方失败");
                        failures.push((id, e));
                    }
                },
                Ok(_) => {
                    tracing::info!(plugin = %id, "模型网关接上了，使用方没在运行，下次启动带上变量")
                }
            }
        }
        failures
    }

    /// 模型网关接上后重建使用方的容器：和安装时一样渲染，全部容器换一遍。
    async fn rebuild_for_models(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
    ) -> Result<(), InstallError> {
        let lan = self
            .lan_grant(package, record.lan_webui, &self.owner_languages())
            .await?;
        let (pending, reservation) = match self.prepare_migration(&package.manifest).await? {
            Some((pending, reservation)) => (Some(pending), Some(reservation)),
            None => (None, None),
        };
        let mut ctx = self
            .rebuild_context(package, record, lan, pending.as_ref())
            .await?;
        // Bot 的 Matrix 变量和总线地址也要填：Bot 服务的容器引用了它们，令牌从旧容器读回。
        self.fill_bot_vars(package, &mut ctx, &self.owner_languages())
            .await?;
        let plan = render::render(package, &ctx, &self.owner_languages())?;
        if let (Some(pending), Some(reservation)) = (&pending, reservation) {
            self.apply_migration(
                &package.manifest,
                pending,
                reservation,
                ctx.egress.is_some(),
                &self.owner_languages(),
            )
            .await?;
        }
        self.replace_containers(&ctx, plan, Gate::StartPeriod)
            .await?;
        self.reconcile_running(package, record).await?;
        Ok(())
    }
}
