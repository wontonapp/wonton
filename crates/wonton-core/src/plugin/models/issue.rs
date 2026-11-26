use super::{
    COMMAND_TIMEOUT, InstallError, Languages, ModelVars, ModelsRoute, NAME_PLACEHOLDER, PluginId,
    PluginManager, PluginPackage, ProviderState, credential_names, empty_model_vars, random_token,
    unavailable, uses_models,
};

impl PluginManager {
    /// 安装声明了 `models` 的插件时签发并验证凭据（[`provider_when_ready`](Self::provider_when_ready) 已在
    /// 拿依赖锁之前等过网关）；没装、没就绪或签发失败都照样装，变量定义成空字符串（见 [`empty_model_vars`]）。
    /// 签发失败记日志、原因记进凭据文件，详情页的轻提示带上它（网关不在的情形详情页现场就能算出来，不记）。
    pub(in crate::plugin) async fn provision_models(
        &self,
        package: &PluginPackage,
        langs: &Languages,
    ) -> Result<Option<ModelVars>, InstallError> {
        let manifest = &package.manifest;
        if !uses_models(manifest) {
            return Ok(None);
        }
        let bot_names = self.bot_names(&manifest.id);
        let vars = match self.provider(langs) {
            Err(e) => {
                tracing::info!(plugin = %manifest.id, error = %e, "模型网关没接上，照样安装，模型变量为空");
                empty_model_vars(package, &bot_names)
            }
            Ok(route) => match self
                .issue_consumer_credentials(&route, package, langs, &bot_names)
                .await
            {
                Ok(vars) => vars,
                Err(e) => {
                    tracing::warn!(plugin = %manifest.id, error = %e, "模型网关凭据签发失败，照样安装，模型变量为空");
                    empty_model_vars(package, &bot_names)
                }
            },
        };
        Ok(Some(vars))
    }

    /// 给使用方签发凭据并验证：缺的名字生成新值，逐个签发；成功后把变量一并记下（重建容器时用），
    /// 失败就吊销已经签发的、清掉记下的值，只留下原因。网关在签发中途坏掉时也照此办理，不留半套凭据。
    pub(super) async fn issue_consumer_credentials(
        &self,
        route: &ModelsRoute,
        package: &PluginPackage,
        langs: &Languages,
        bot_names: &[String],
    ) -> Result<ModelVars, InstallError> {
        let store = self.store.model_keys();
        let id = &package.manifest.id;
        let mut file = store.load(id)?;
        let (plugin, bots) = credential_names(package, bot_names);
        for name in plugin.iter().chain(bots.iter()) {
            file.keys.entry(name.clone()).or_insert_with(random_token);
        }
        let issued = async {
            for name in plugin.iter().chain(bots.iter()) {
                self.issue_verified(route, name, &file.keys[name], langs)
                    .await?;
            }
            Ok::<_, InstallError>(())
        }
        .await;
        match issued {
            Ok(()) => {
                let vars = self.model_vars(route, package, &file.keys, bot_names);
                file.vars = Some(vars.clone());
                file.problem = None;
                file.reconcile_problem = None;
                // 凭据重新接上：之前的探测结论是旧状态留下的，下一轮探测重写。
                file.route = None;
                store.save(id, &file)?;
                Ok(vars)
            }
            Err(e) => {
                for name in file.keys.keys() {
                    let revoke = &route.spec.credentials.revoke;
                    if let Err(cleanup) = self.run_command(route, revoke, name, None, langs).await {
                        tracing::error!(plugin = %id, credential = %name, error = %cleanup, "吊销没签发完的模型网关凭据失败");
                    }
                }
                file.keys.clear();
                file.vars = None;
                file.problem = Some(e.to_string());
                store.save(id, &file)?;
                Err(e)
            }
        }
    }

    /// 升级时：旧凭据原样留着（新版本装不好还要退回去），新版本多要的名字网关就绪就补签（记进 `added`），
    /// 网关不在就沿用记下的变量（可能为空），缺的等网关回来在对账时补。
    pub(in crate::plugin) async fn upgrade_model_vars(
        &self,
        package: &PluginPackage,
        added: &mut Vec<String>,
        langs: &Languages,
    ) -> Result<Option<ModelVars>, InstallError> {
        let manifest = &package.manifest;
        if !uses_models(manifest) {
            return Ok(None);
        }
        let store = self.store.model_keys();
        let mut file = store.load(&manifest.id)?;
        let Ok(route) = self.provider(langs) else {
            return Ok(Some(file.vars.clone().unwrap_or_else(|| {
                empty_model_vars(package, &self.bot_names(&manifest.id))
            })));
        };
        let bot_names = self.bot_names(&manifest.id);
        let (plugin, bots) = credential_names(package, &bot_names);
        for name in plugin.iter().chain(bots.iter()) {
            if file.keys.contains_key(name) {
                continue;
            }
            let key = random_token();
            file.keys.insert(name.clone(), key.clone());
            added.push(name.clone());
            self.issue_verified(&route, name, &key, langs).await?;
        }
        let vars = self.model_vars(&route, package, &file.keys, &bot_names);
        file.vars = Some(vars.clone());
        file.problem = None;
        file.reconcile_problem = None;
        file.route = None;
        store.save(&manifest.id, &file)?;
        Ok(Some(vars))
    }

    /// 退回旧版本时吊销 [`Self::upgrade_model_vars`] 多签发的。网关已经卸载就没有可吊销的，只清掉记下的值；
    /// 网关装着但没在运行时吊销不了，如实报错。
    pub(in crate::plugin) async fn revoke_model_names(
        &self,
        id: &PluginId,
        names: &[String],
        langs: &Languages,
    ) -> Result<(), InstallError> {
        if names.is_empty() {
            return Ok(());
        }
        let store = self.store.model_keys();
        let mut file = store.load(id)?;
        match self.models_provider() {
            ProviderState::Absent => {}
            ProviderState::NotReady(_) => {
                return Err(unavailable(crate::l10n::text(
                    langs,
                    "plugin-settings-models-revoke-not-running",
                    &[("id", id.as_str().into())],
                )));
            }
            ProviderState::Ready(route) => {
                for name in names {
                    let revoke = &route.spec.credentials.revoke;
                    self.run_command(&route, revoke, name, None, langs).await?;
                }
            }
        }
        for name in names {
            file.keys.remove(name);
        }
        if file.keys.is_empty() {
            store.remove(id)?;
        } else {
            store.save(id, &file)?;
        }
        Ok(())
    }

    /// 给一个 Bot 签发自己的模型网关凭据（`<插件 id>/<Bot 名>`），建 Bot 时用。插件没申请 `models` 时不动。
    pub(in crate::plugin) async fn issue_bot_credential(
        &self,
        package: &PluginPackage,
        bot: &str,
        langs: &Languages,
    ) -> Result<Option<String>, InstallError> {
        let manifest = &package.manifest;
        if !uses_models(manifest) || manifest.bots.is_none() {
            return Ok(None);
        }
        let route = self.provider(langs)?;
        let store = self.store.model_keys();
        let mut file = store.load(&manifest.id)?;
        let name = format!("{}/{bot}", manifest.id);
        let key = file
            .keys
            .entry(name.clone())
            .or_insert_with(random_token)
            .clone();
        store.save(&manifest.id, &file)?;
        self.issue_verified(&route, &name, &key, langs).await?;
        Ok(Some(key))
    }

    /// 删掉一个 Bot：吊销它的模型网关凭据，再从记下的值里删掉。没有记过就什么都不做。
    pub(in crate::plugin) async fn revoke_bot_credential(
        &self,
        id: &PluginId,
        bot: &str,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let name = format!("{id}/{bot}");
        if !self.store.model_keys().load(id)?.keys.contains_key(&name) {
            return Ok(());
        }
        self.revoke_model_names(id, &[name], langs).await
    }

    /// 重建容器时（见 [`super::lan`]、[`super::egress`]）：注入记下的变量，不问网关在不在——网关不在，
    /// 变量也保持原样，等网关回来对账后就接上了。从来没接上的注入空变量。
    pub(in crate::plugin) fn stored_model_vars(
        &self,
        package: &PluginPackage,
    ) -> Result<Option<ModelVars>, InstallError> {
        if !uses_models(&package.manifest) {
            return Ok(None);
        }
        let file = self.store.model_keys().load(&package.manifest.id)?;
        Ok(Some(file.vars.clone().unwrap_or_else(|| {
            empty_model_vars(package, &self.bot_names(&package.manifest.id))
        })))
    }

    /// 拒绝类错误文本里已经带了凭据名，没带的（命令超时之类）补上，免得原因里看不出是哪一个。
    pub(super) fn named(name: &str, error: &InstallError) -> String {
        let text = error.to_string();
        if text.contains(name) {
            text
        } else {
            format!("{name}: {text}")
        }
    }

    /// 签发，再用这个凭据请求一次模型列表。返回 401 说明网关里的同名凭据不是这个值：先吊销再签发一次。
    pub(super) async fn issue_verified(
        &self,
        route: &ModelsRoute,
        name: &str,
        value: &str,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        self.issue_verified_catalog(route, name, value, langs)
            .await
            .map(|_| ())
    }

    /// 在网关的容器里执行凭据命令，`{name}` 逐项替换成凭据名，不经 shell。
    pub(in crate::plugin) async fn run_command(
        &self,
        route: &ModelsRoute,
        argv: &[String],
        name: &str,
        value: Option<&str>,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let argv: Vec<String> = argv
            .iter()
            .map(|arg| arg.replace(NAME_PLACEHOLDER, name))
            .collect();
        let stdin = value.map(|v| format!("{v}\n"));
        let output = self
            .engine
            .exec(
                &route.credentials_container,
                &argv,
                stdin.as_deref().map(str::as_bytes),
                COMMAND_TIMEOUT,
            )
            .await
            .map_err(|e| {
                unavailable(crate::l10n::text(
                    langs,
                    "plugin-settings-models-not-ready",
                    &[
                        ("provider", route.id.as_str().into()),
                        ("error", e.to_string().into()),
                    ],
                ))
            })?;
        if output.exit_code == 0 {
            return Ok(());
        }
        Err(unavailable(crate::l10n::text(
            langs,
            "plugin-settings-models-command-rejected",
            &[
                ("provider", route.id.as_str().into()),
                ("command", argv.first().map_or("", String::as_str).into()),
                ("code", output.exit_code.to_string().into()),
                ("stderr", output.stderr.clone().into()),
            ],
        )))
    }
}
