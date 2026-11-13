//! 安装前准备密钥、权限上下文与默认 Bot。

use super::super::gpu::GpuGrant;
use super::super::lan::LanGrant;
use super::super::languages::Languages;
use super::super::manager::{InstallError, PluginManager};
use super::super::manifest::Manifest;
use super::super::package::PluginPackage;
use super::super::proxy_auth::ProxyAuth;
use super::super::render::InstallContext;
use super::super::secrets::{self, Secrets};
use super::super::store::EnabledPermission;
use super::super::upstreams::uses_sso;
use super::Undo;
use crate::secret::random_token;
use wonton_protocol::InstallStage;

impl PluginManager {
    /// 读出插件的密钥（上次不删数据卸载时保留的也在），给还没有值的 `generate` 项生成并保存。
    pub(in crate::plugin) fn prepare_secrets(
        &self,
        manifest: &Manifest,
        undo: &mut Undo,
    ) -> Result<Secrets, InstallError> {
        if manifest.secrets.is_empty() {
            return Ok(Secrets::new());
        }
        let store = self.store.secrets();
        let before = store.load(&manifest.id)?;
        let mut secrets = before.clone().unwrap_or_default();
        undo.secrets_before = Some((manifest.id.clone(), before));
        if secrets::generate_missing(manifest, &mut secrets) {
            store.save(&manifest.id, &secrets)?;
        }
        Ok(secrets)
    }

    /// 安装的前半段：清残留、备密钥、定权限和上下文，再定默认 Bot 和模型网关凭据。
    pub(super) async fn prepare_install(
        &self,
        package: &PluginPackage,
        enabled: &[EnabledPermission],
        (gpu, lan): (Option<GpuGrant>, Option<LanGrant>),
        settings: std::collections::BTreeMap<String, serde_json::Value>,
        undo: &mut Undo,
        langs: &Languages,
    ) -> Result<(InstallContext, Option<String>, Secrets, Option<ProxyAuth>), InstallError> {
        let manifest = &package.manifest;
        self.stage(&manifest.id, InstallStage::Preparing);
        self.clear_leftovers(&manifest.id).await?;
        let secret = uses_sso(manifest).then(random_token);
        let secrets = self.prepare_secrets(manifest, undo)?;
        let generated =
            secrets::generated_values(manifest, &secrets, langs).map_err(InstallError::Storage)?;
        let proxy =
            ProxyAuth::resolve(manifest, &generated, langs).map_err(InstallError::Storage)?;
        let effective =
            manifest.effective_permissions(&super::super::store::permission_set(enabled));
        let mut ctx = self
            .install_context(manifest, secret.as_deref(), generated, &effective, langs)
            .await?;
        ctx.gpu = gpu;
        ctx.lan = lan;
        ctx.settings = settings;
        self.check_network_names(&ctx).await?;
        let default_bot = self.provision_default_bot(package, undo).await?;
        if let Some(models) = self.provision_models(package, langs).await? {
            ctx.models = Some(models);
            undo.models = Some(manifest.id.clone());
        }
        if let Some(bot) = &default_bot {
            self.fill_default_bot_vars(manifest, bot, &mut ctx);
        }
        Ok((ctx, secret, secrets, proxy))
    }
}
