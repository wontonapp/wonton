use super::{
    InstallError, KeyFile, Keys, Languages, ModelVars, ModelsRoute, PluginId, PluginManager,
    PluginPackage, ProviderState, base_url, credential_names, unavailable,
};

/// 没接上网关时注入的变量：`WONTON_MODEL_*` 全部定义成空字符串，compose 引用它们不会因为变量缺失而失败，
/// 插件用它自己的登录或 key 设置。
pub(super) fn empty_model_vars(package: &PluginPackage, bot_names: &[String]) -> ModelVars {
    let (plugin, _) = credential_names(package, bot_names);
    ModelVars {
        base_url: String::new(),
        anthropic_url: Some(String::new()),
        gemini_url: Some(String::new()),
        plugin_key: plugin.map(|_| String::new()),
        bot_keys: bot_names
            .iter()
            .map(|bot| (bot.clone(), String::new()))
            .collect(),
        default_model: String::new(),
    }
}

impl PluginManager {
    /// 注入的变量；`keys` 里有使用方要的每个凭据。
    pub(super) fn model_vars(
        &self,
        route: &ModelsRoute,
        package: &PluginPackage,
        keys: &Keys,
        bot_names: &[String],
    ) -> ModelVars {
        let (plugin, _) = credential_names(package, bot_names);
        let spec = &route.spec;
        let port = self.cfg.gateway_port;
        ModelVars {
            base_url: base_url(port, &spec.apis.openai),
            anthropic_url: spec.apis.anthropic.as_ref().map(|p| base_url(port, p)),
            gemini_url: spec.apis.gemini.as_ref().map(|p| base_url(port, p)),
            plugin_key: plugin.map(|name| keys[&name].clone()),
            bot_keys: bot_names
                .iter()
                .map(|bot| {
                    (
                        bot.clone(),
                        keys[&format!("{}/{}", package.manifest.id, bot)].clone(),
                    )
                })
                .collect(),
            default_model: spec.default_model.clone(),
        }
    }

    /// 卸载使用方或安装失败回滚：在网关里吊销它的凭据，再删掉记下的文件（没有凭据也删——里面可能只记着
    /// 签发失败的原因）。网关已经卸载了就没有可吊销的；网关装着但没在运行时不挡卸载，把文件留下、记上待补，
    /// 网关回来对账时再吊销（吊销不掉最多留一条孤立的凭据，不影响卸载本身）。
    pub(in crate::plugin) async fn revoke_models(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let store = self.store.model_keys();
        let mut file = store.load(id)?;
        match self.models_provider() {
            ProviderState::Absent => {}
            ProviderState::NotReady(provider) => {
                if !file.keys.is_empty() {
                    tracing::warn!(
                        plugin = %id,
                        provider = %provider,
                        "模型网关没在运行，凭据留到网关回来后补吊销"
                    );
                    file.revoke_pending = true;
                    store.save(id, &file)?;
                    return Ok(());
                }
            }
            ProviderState::Ready(route) => {
                for name in file.keys.keys() {
                    let revoke = &route.spec.credentials.revoke;
                    self.run_command(&route, revoke, name, None, langs).await?;
                }
            }
        }
        store.remove(id)?;
        Ok(())
    }

    /// 按当前的接口路径和默认模型把记下的变量刷新一遍（网关升级会改这两样）。插件记录读不到就不动。
    fn refresh_model_vars(
        &self,
        id: &PluginId,
        route: &ModelsRoute,
        langs: &Languages,
        file: &mut KeyFile,
    ) {
        let Ok(Some(record)) = self.store.load(id) else {
            return;
        };
        let Ok(mut package) = PluginPackage::with_files(&record.source, record.files, langs) else {
            return;
        };
        package.origin = record.origin.clone();
        file.vars = Some(self.model_vars(route, &package, &file.keys, &self.bot_names(id)));
        file.route = None;
    }

    /// 上次对账失败过、凭据值还记着：巡检在容器没问题时调用。
    pub(in crate::plugin) fn model_reconcile_failed(&self, id: &PluginId) -> bool {
        self.store
            .model_keys()
            .load(id)
            .is_ok_and(|file| file.reconcile_problem.is_some() && !file.keys.is_empty())
    }

    /// 用记下的值重试一次对账，每个凭据都试（和启动、升级时的对账一致）。成功清掉原因；
    /// 失败把失败的名字和原因合并记下，并交回调用方。
    pub(in crate::plugin) async fn retry_model_keys(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let store = self.store.model_keys();
        let mut file = store.load(id)?;
        let route = self.provider(langs)?;
        let mut problems = Vec::new();
        for (name, value) in &file.keys {
            if let Err(e) = self.issue_verified(&route, name, value, langs).await {
                tracing::warn!(plugin = %id, credential = %name, error = %e, "模型网关凭据重试失败");
                problems.push(Self::named(name, &e));
            }
        }
        let problem = (!problems.is_empty()).then(|| problems.join("\n"));
        file.reconcile_problem = problem.clone();
        match problem {
            Some(problem) => {
                file.problem = Some(problem.clone());
                store.save(id, &file)?;
                Err(unavailable(problem))
            }
            None => {
                // 值不一定变，接口路径和默认模型却会跟着网关版本变：成功就按现在的刷新一遍变量。
                self.refresh_model_vars(id, &route, langs, &mut file);
                file.problem = None;
                store.save(id, &file)?;
                Ok(())
            }
        }
    }
}
