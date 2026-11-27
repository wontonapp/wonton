use super::{
    Gate, InstallError, InstalledRecord, Languages, ModelsRoute, PluginId, PluginManager,
    PluginPackage, PluginStatus, ProviderState, credential_names, render, unavailable, uses_models,
};

impl PluginManager {
    /// 凭据错误属于模型连接，不改变容器状态；恢复由使用方凭据验证决定。
    pub async fn reconcile_models(&self) -> Vec<(PluginId, InstallError)> {
        let _changing = self.dependencies.changing.lock().await;
        self.reconcile_models_locked().await
    }

    /// 已经拿着依赖锁时用（安装、恢复、升级）。
    pub(in crate::plugin) async fn reconcile_models_locked(&self) -> Vec<(PluginId, InstallError)> {
        let langs = self.owner_languages();
        let mut failures = self.reconcile_model_keys(&langs).await;
        for (id, _) in &failures {
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
        for (id, file) in all {
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
            let _allocating = self.bot_allocation.lock().await;
            if let Err(error) = self.reconcile_one_model_key(&route, &id, langs).await {
                tracing::warn!(plugin = %id, %error, "模型网关凭据对账失败");
                failures.push((id, error));
            }
        }
        failures
    }

    /// 对一个使用方对账：先只读验证，网关还认这些凭据就不发写命令；只有网关 401 才用原值重签，
    /// 容器里的变量不变（#643 的超时发生在凭据写命令上）。
    async fn reconcile_one_model_key(
        &self,
        route: &ModelsRoute,
        id: &PluginId,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let store = self.store.model_keys();
        let mut file = store.load(id)?;
        if file.vars.is_none() || file.keys.is_empty() {
            return Ok(());
        }
        let Some(record) = self.store.load(id)? else {
            return Ok(());
        };
        let package = record.package(langs)?;
        let bot_names = self.stored_bot_names(id)?;
        let (plugin, bots) = credential_names(&package, &bot_names);
        let names: Vec<_> = plugin.iter().chain(bots.iter()).collect();
        let missing = |name: &str| {
            unavailable(crate::l10n::text(
                langs,
                "plugin-settings-models-credential-missing",
                &[("id", id.as_str().into()), ("name", name.into())],
            ))
        };
        let result = async {
            let mut recognized = true;
            for name in &names {
                let value = file.keys.get(name.as_str()).ok_or_else(|| missing(name))?;
                if self.verify(route, value, langs).await?.is_none() {
                    recognized = false;
                    break;
                }
            }
            if !recognized {
                // 网关丢了状态：用原值重签，容器里的变量不变；一个失败也要试完其余的。
                let mut problems = Vec::new();
                for name in names {
                    let Some(value) = file.keys.get(name.as_str()) else {
                        problems.push(Self::named(name, &missing(name)));
                        continue;
                    };
                    if let Err(e) = self.issue_verified(route, name, value, langs).await {
                        problems.push(Self::named(name, &e));
                    }
                }
                if !problems.is_empty() {
                    return Err(unavailable(problems.join("\n")));
                }
            }
            Ok::<(), InstallError>(())
        }
        .await;
        match &result {
            Ok(()) => {
                file.vars = Some(self.model_vars(route, &package, &file.keys, &bot_names));
                file.problem = None;
                file.reconcile_problem = None;
                file.route = None;
            }
            Err(error) => {
                let problem = error.to_string();
                file.problem = Some(problem.clone());
                file.reconcile_problem = Some(problem);
            }
        }
        store.save(id, &file)?;
        self.changed();
        result
    }

    /// 巡检里查看凭据：命令还没有结局时不动；已经失败或结局未知的，重新验证，验不过就用原值补发一次
    /// （网关丢了状态时靠它恢复；不换凭据、不重建容器）。
    pub(in crate::plugin) async fn watch_model_credentials(&self) {
        let Some(route) = self.models_route() else {
            return;
        };
        // 网关不在或没就绪：验证只会白报错，恢复由网关就绪后的对账负责。
        if !matches!(self.models_provider(), ProviderState::Ready(_)) {
            return;
        }
        let store = self.store.model_keys();
        let all = match store.all() {
            Ok(all) => all,
            Err(error) => {
                tracing::warn!(%error, "读取模型网关凭据失败");
                return;
            }
        };
        for (id, _) in all {
            let _allocating = self.bot_allocation.lock().await;
            if let Err(error) = self.observe_model_credentials(&route, &id).await {
                tracing::warn!(plugin = %id, %error, "查看模型网关凭据失败");
            }
        }
    }

    async fn observe_model_credentials(
        &self,
        route: &ModelsRoute,
        id: &PluginId,
    ) -> Result<(), InstallError> {
        let store = self.store.model_keys();
        let mut file = store.load(id)?;
        let pending = store
            .pending()?
            .is_some_and(|command| command.consumer == id.as_str());
        if file.vars.is_none() || (file.problem.is_none() && !pending) {
            return Ok(());
        }
        let settled = {
            let _commands = store.commands.lock().await;
            self.settle_model_command(None).await?
        };
        if !settled {
            return Ok(());
        }
        let Some(record) = self.store.load(id)? else {
            return Ok(());
        };
        let langs = self.owner_languages();
        let package = record.package(&langs)?;
        let bot_names = self.stored_bot_names(id)?;
        let (plugin, bots) = credential_names(&package, &bot_names);
        let names: Vec<_> = plugin.iter().chain(bots.iter()).collect();
        if names.is_empty() {
            return Ok(());
        }
        let missing = |name: &str| {
            unavailable(crate::l10n::text(
                &langs,
                "plugin-settings-models-credential-missing",
                &[("id", id.as_str().into()), ("name", name.into())],
            ))
        };
        let result = async {
            let mut recognized = true;
            for name in &names {
                let value = file.keys.get(name.as_str()).ok_or_else(|| missing(name))?;
                if self.verify(route, value, &langs).await?.is_none() {
                    recognized = false;
                    break;
                }
            }
            if !recognized {
                // 命令可能没落地：用记下的原值补发，验过才算恢复；一个失败也要试完其余的。
                let mut problems = Vec::new();
                for name in names {
                    let Some(value) = file.keys.get(name.as_str()) else {
                        problems.push(Self::named(name, &missing(name)));
                        continue;
                    };
                    if let Err(e) = self.issue_verified(route, name, value, &langs).await {
                        problems.push(Self::named(name, &e));
                    }
                }
                if !problems.is_empty() {
                    return Err(unavailable(problems.join("\n")));
                }
            }
            Ok::<(), InstallError>(())
        }
        .await;
        match &result {
            Ok(()) => {
                file.problem = None;
                file.reconcile_problem = None;
                file.route = None;
            }
            Err(error) => {
                let problem = error.to_string();
                file.problem = Some(problem.clone());
                file.reconcile_problem = Some(problem);
            }
        }
        store.save(id, &file)?;
        self.changed();
        result
    }

    /// 上一条命令的记录没了（容器重启丢掉）或者创建了没启动：都算结局已定，剩下的事实靠重新验证。
    /// 没有按时间猜的逃生口：只有验证能解；容器重启、换掉或 404 都算已定。
    pub(super) async fn settle_model_command(
        &self,
        container_id: Option<&str>,
    ) -> Result<bool, InstallError> {
        let store = self.store.model_keys();
        let Some(pending) = store.pending()? else {
            return Ok(true);
        };
        // 网关容器换掉了（重建/重装）：上一条命令所在的容器已经没了，剩下的事实靠重新验证。
        if container_id.is_some_and(|current| current != pending.container_id) {
            store.clear_pending()?;
            return Ok(true);
        }
        if !self.engine.exec_finished(&pending.exec_id).await? {
            return Ok(false);
        }
        store.clear_pending()?;
        Ok(true)
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
                // 已注入变量的使用方由对账处理，不因一次失败换掉凭据。
                .filter(|(id, _)| {
                    self.store
                        .model_keys()
                        .load(id)
                        .is_ok_and(|file| file.vars.is_none())
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
            let issued = {
                let _allocating = self.bot_allocation.lock().await;
                match self.stored_bot_names(&id) {
                    Ok(bot_names) => {
                        self.issue_consumer_credentials(&route, &package, langs, &bot_names)
                            .await
                    }
                    Err(error) => Err(error.into()),
                }
            };
            match issued {
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
