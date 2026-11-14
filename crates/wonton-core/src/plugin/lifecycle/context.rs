//! 安装上下文：组装 [`InstallContext`]、准备宿主目录和注入 OIDC 端点。

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use super::super::core_address;
use super::super::hostdir::{self, Owner};
use super::super::languages::Languages;
use super::super::manager::{InstallError, PluginManager};
use super::super::manifest::{Manifest, Permission, Sso};
use super::super::render::InstallContext;
use super::super::vars::{OidcVars, OwnerVars};
use super::Undo;
use crate::l10n;

impl PluginManager {
    /// 注入给 SSO 插件的端点：签发方和后台端点在插件网络里，授权端点给浏览器（ADR-0063）。
    async fn oidc_vars(
        &self,
        manifest: &Manifest,
        secret: Option<&str>,
        langs: &Languages,
    ) -> Result<Option<OidcVars>, InstallError> {
        let Some(secret) = secret else {
            return Ok(None);
        };
        let public = &self.cfg.public;
        let discovery = self
            .hydra(langs)?
            .resolved_discovery(
                &public.origin(&public.auth_host()),
                &public.auth_internal_origin(),
            )
            .await?;
        Ok(Some(OidcVars {
            issuer: discovery.issuer,
            client_id: manifest.id.to_string(),
            client_secret: secret.to_owned(),
            auth_url: discovery.authorization_endpoint,
            token_url: discovery.token_endpoint,
            userinfo_url: discovery.userinfo_endpoint,
        }))
    }

    pub(in crate::plugin) async fn install_context(
        &self,
        manifest: &Manifest,
        secret: Option<&str>,
        generated: BTreeMap<String, String>,
        granted: &[Permission],
        langs: &Languages,
    ) -> Result<InstallContext, InstallError> {
        let public = &self.cfg.public;
        let slug = manifest.id.slug();
        let app_url = manifest
            .webui
            .as_ref()
            .map(|_| public.origin(&public.plugin_host(&slug)));
        let oidc = self.oidc_vars(manifest, secret, langs).await?;
        let shares = manifest
            .permissions
            .iter()
            .filter_map(|p| match p {
                Permission::StorageShare { name, .. } => {
                    Some((name.clone(), self.cfg.shares_dir.join(name)))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        if !shares.is_empty() {
            fs::create_dir_all(&self.cfg.shares_dir).map_err(|e| {
                InstallError::Storage(l10n::text(
                    langs,
                    "plugin-install-shared-folder-mkdir-failed",
                    &[
                        ("dir", self.cfg.shares_dir.display().to_string().into()),
                        ("error", e.to_string().into()),
                    ],
                ))
            })?;
        }
        for path in shares.values() {
            hostdir::ensure_owned_dir(path, self.owner()).map_err(|e| {
                InstallError::Storage(l10n::text(
                    langs,
                    "plugin-install-shared-folder-prepare-failed",
                    &[("error", e.to_string().into())],
                ))
            })?;
        }
        let capabilities = {
            let mut vars = self.dependency_vars(manifest, None)?;
            vars.extend(self.matrix_rtc_vars(manifest, &self.cfg.apps_dir.join(&slug), langs)?);
            vars.extend(self.agent_acp_provider_vars(manifest)?);
            vars
        };
        self.prepare_hosted_roots(manifest, langs)?;
        Ok(InstallContext {
            app_data: self.cfg.apps_dir.join(&slug),
            hosted_root: self.hosted_dirs().root().to_path_buf(),
            relay_dir: self.cfg.relay_dir.clone(),
            config_store: self.cfg.data_dir.join("core").join("configs"),
            slug,
            instance: self.cfg.instance.clone(),
            shares,
            app_url,
            oidc,
            owner: manifest.gets_owner_identity().then(|| OwnerVars {
                name: self.cfg.owner.clone(),
                email: crate::owner::internal_email(&self.cfg.owner),
            }),
            matrix: None,
            bots: Default::default(),
            homes: self.private_home_mounts(manifest, granted)?,
            models: None,
            capabilities,
            mcp_url: manifest.permissions.contains(&Permission::Tools).then(|| {
                let port = self.cfg.gateway_port;
                format!("http://{}:{port}/mcp", crate::config::PLUGIN_MCP_HOST)
            }),
            notify_url: manifest.permissions.contains(&Permission::Notify).then(|| {
                let port = self.cfg.gateway_port;
                format!(
                    "http://{}:{port}/notices",
                    crate::config::PLUGIN_NOTIFY_HOST
                )
            }),
            secrets: generated,
            devices: None,
            tz: self.cfg.tz.clone(),
            puid: self.cfg.puid,
            pgid: self.cfg.pgid,
            extra_hosts: Vec::new(),
            gpu: None,
            lan: None,
            egress: self.egress_vars(manifest)?,
            core_address: (manifest.webui.as_ref().is_some_and(|w| w.sso == Sso::Proxy))
                .then(|| core_address::PLACEHOLDER.to_owned()),
            limits: Default::default(),
            memory_total: self.memory_total().await,
            host_name: self.host_name().await,
            cgroup_parent: self.cfg.plugin_slice.clone(),
            granted: granted.to_vec(),
            settings: Default::default(),
            install_limit: self.install_limit(manifest),
        })
    }

    /// 主人的用户（PUID:PGID）：共享文件夹、应用数据目录本身和中间各级目录都交给它。
    pub(in crate::plugin) fn owner(&self) -> Owner {
        Owner {
            uid: self.cfg.puid,
            gid: self.cfg.pgid,
        }
    }

    /// 绑定挂载的源目录由核心预先创建并交给挂载它的服务的属主，否则 Docker 会以 root 身份创建。
    /// 逐级检查、不跟随软链接：应用数据目录里可能有上次安装的插件留下的软链接。
    pub(in crate::plugin) fn prepare_dirs(
        &self,
        ctx: &InstallContext,
        dirs: &BTreeMap<PathBuf, Owner>,
        undo: &mut Undo,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        if dirs.is_empty() {
            return Ok(());
        }
        let storage = |e: &dyn std::fmt::Display| {
            InstallError::Storage(l10n::text(
                langs,
                "plugin-install-app-data-prepare-failed",
                &[("error", e.to_string().into())],
            ))
        };
        fs::create_dir_all(&self.cfg.apps_dir).map_err(|e| storage(&e))?;
        if fs::symlink_metadata(&ctx.app_data)
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            undo.app_dir_created = Some(ctx.app_data.clone());
        }
        hostdir::prepare(&ctx.app_data, self.owner(), dirs).map_err(|e| storage(&e))
    }

    /// 中继套接字目录：`/run` 下由 root 创建（安装脚本会先建好），核心是 root 时在这里补建并交给 PUID。
    pub(in crate::plugin) fn prepare_relay_dir(
        &self,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        if let Some(parent) = self.cfg.relay_dir.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                InstallError::Storage(l10n::text(
                    langs,
                    "plugin-install-relay-prepare-dir-failed",
                    &[("error", e.to_string().into())],
                ))
            })?;
        }
        hostdir::ensure_owned_dir(&self.cfg.relay_dir, self.owner()).map_err(|e| {
            InstallError::Storage(l10n::text(
                langs,
                "plugin-install-relay-prepare-dir-failed",
                &[("error", e.to_string().into())],
            ))
        })
    }
}
