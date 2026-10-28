//! 部署、恢复与卸载：把部署计划交给 Docker，失败时按相反顺序回滚。

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use tokio::time::Instant;

use super::configs;
use super::health::{self, Checked, Readiness, Target};
use super::hostdir::{self, Owner};
use super::manager::{Deployed, Health, InstallError, PluginManager, UninstallReport};
use super::manifest::{Manifest, Permission, PluginId, Sso};
use super::package::PluginPackage;
use super::render::{Deployment, InstallContext, LABEL_PLUGIN, OidcVars, container_name, render};
use super::secrets::{self, Secrets};
use super::store::InstalledRecord;
use crate::config::PublicUrls;
use crate::oidc::{ClientSpec, Hydra};
use crate::secret::random_token;

/// 插件管理器需要的那部分核心配置。
#[derive(Debug, Clone)]
pub struct LifecycleConfig {
    pub public: PublicUrls,
    pub apps_dir: PathBuf,
    pub shares_dir: PathBuf,
    pub core_container: Option<String>,
    pub tz: String,
    pub puid: u32,
    pub pgid: u32,
    pub install_timeout: Duration,
}

/// 本次安装已经创建的资源，失败时据此回滚。
#[derive(Default)]
struct Undo {
    containers: Vec<String>,
    volumes: Vec<String>,
    network: Option<String>,
    core_attached: bool,
    oidc_client: Option<String>,
    app_dir_created: Option<PathBuf>,
    /// 本次安装之前插件的密钥（`None` 表示原来没有），回滚时恢复成这样。
    secrets_before: Option<(PluginId, Option<Secrets>)>,
}

/// 插件声明的命名卷怎么处理。
#[derive(Debug, PartialEq, Eq)]
enum VolumeAction {
    Create,
    Reuse,
}

/// Docker 创建同名卷时直接返回已有的卷，所以先看它属于谁：不存在就新建（安装失败时回滚删除）；
/// 属于本插件（上次不删数据卸载后保留的）就复用，回滚时不删；属于其他插件或不是核心创建的，拒绝安装。
fn volume_action(
    volume: &str,
    labels: Option<&HashMap<String, String>>,
    plugin: &PluginId,
) -> Result<VolumeAction, InstallError> {
    let Some(labels) = labels else {
        return Ok(VolumeAction::Create);
    };
    match labels.get(LABEL_PLUGIN) {
        Some(owner) if owner == plugin.as_str() => Ok(VolumeAction::Reuse),
        Some(owner) => Err(InstallError::Conflict(format!(
            "卷 {volume} 属于插件 {owner}，和本插件的卷重名"
        ))),
        None => Err(InstallError::Conflict(format!(
            "卷 {volume} 已存在，且不是核心为插件创建的"
        ))),
    }
}

fn uses_sso(manifest: &Manifest) -> bool {
    manifest.webui.as_ref().is_some_and(|w| w.sso == Sso::Oidc)
}

impl PluginManager {
    fn hydra(&self) -> Result<&Hydra, InstallError> {
        self.oidc.as_ref().ok_or_else(|| {
            InstallError::OidcUnavailable(
                "核心没有配置身份提供方（WONTON_OIDC_ADMIN_URL / WONTON_OIDC_PUBLIC_URL）".into(),
            )
        })
    }

    /// 部署插件并写下安装记录，返回核心访问插件网页的地址。任一步失败（包括写记录）都回滚本次创建的
    /// 资源：没有记录的容器、网络和 OIDC 客户端，核心重启后就没人管了。
    pub(super) async fn deploy(
        &self,
        package: &PluginPackage,
        granted: &[Permission],
    ) -> Result<Deployed, InstallError> {
        let mut undo = Undo::default();
        match self.deploy_inner(package, granted, &mut undo).await {
            Ok(deployed) => Ok(deployed),
            Err(error) => {
                let failures = self.rollback(undo).await;
                if failures.is_empty() {
                    Err(error)
                } else {
                    Err(error
                        .with_note(&format!("回滚未完成，可能有残留：{}", failures.join("；"))))
                }
            }
        }
    }

    async fn deploy_inner(
        &self,
        package: &PluginPackage,
        granted: &[Permission],
        undo: &mut Undo,
    ) -> Result<Deployed, InstallError> {
        let manifest = &package.manifest;
        let secret = uses_sso(manifest).then(random_token);
        let mut secrets = self.prepare_secrets(manifest, undo)?;
        let generated =
            secrets::generated_values(manifest, &secrets).map_err(InstallError::Storage)?;
        let ctx = self
            .install_context(manifest, secret.as_deref(), generated)
            .await?;
        let plan = render(package, &ctx)?;
        for container in &plan.containers {
            self.engine
                .ensure_image(&container.image, container.pull_policy)
                .await?;
        }
        self.prepare_dirs(&ctx, &plan.dirs, undo)?;
        self.engine
            .create_network(&plan.network, &plan.labels)
            .await?;
        undo.network = Some(plan.network.clone());
        if let Some(core) = &self.cfg.core_container {
            self.attach_core(&plan.network, core, manifest).await?;
            undo.core_attached = true;
        }
        for volume in &plan.volumes {
            let labels = self.engine.volume_labels(volume).await?;
            if volume_action(volume, labels.as_ref(), &manifest.id)? == VolumeAction::Create {
                self.engine.create_volume(volume, &plan.labels).await?;
                undo.volumes.push(volume.clone());
            }
        }
        if let Some(secret) = &secret {
            self.hydra()?
                .upsert_client(&client_spec(manifest, &ctx, secret))
                .await?;
            undo.oidc_client = Some(manifest.id.to_string());
        }
        let deadline = Instant::now() + self.cfg.install_timeout;
        let health = self.start_containers(&plan, undo, deadline).await?;
        if secrets::collect(&self.engine, manifest, &mut secrets).await? {
            self.store.secrets().save(&manifest.id, &secrets)?;
        }
        let upstream = self.webui_upstream(manifest, &plan.network).await?;
        if let (Some(upstream), Some(webui)) = (&upstream, &manifest.webui) {
            let url = format!("http://{upstream}{}", webui.path);
            health::wait_webui(&self.http, &url, deadline).await?;
        }
        self.store.save(&InstalledRecord {
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            source: package.source.clone(),
            files: package.files().clone(),
            granted_permissions: granted.to_vec(),
            oidc_client_secret: secret,
            installed_at: super::manager::now_secs(),
        })?;
        Ok(Deployed { upstream, health })
    }

    /// 读出插件的密钥（上次不删数据卸载时保留的也在），给还没有值的 `generate` 项生成并保存。
    fn prepare_secrets(
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

    async fn install_context(
        &self,
        manifest: &Manifest,
        secret: Option<&str>,
        generated: BTreeMap<String, String>,
    ) -> Result<InstallContext, InstallError> {
        let public = &self.cfg.public;
        let slug = manifest.id.slug();
        let app_url = manifest
            .webui
            .as_ref()
            .map(|_| public.origin(&public.plugin_host(&slug)));
        let oidc = match secret {
            None => None,
            Some(secret) => {
                let discovery = self.hydra()?.discovery().await?;
                Some(OidcVars {
                    issuer: discovery.issuer,
                    client_id: manifest.id.to_string(),
                    client_secret: secret.to_owned(),
                    auth_url: discovery.authorization_endpoint,
                    token_url: discovery.token_endpoint,
                    userinfo_url: discovery.userinfo_endpoint,
                })
            }
        };
        // 核心不在容器里时，插件经宿主网关地址访问核心的身份提供方入口。
        let extra_hosts = if self.cfg.core_container.is_none() && oidc.is_some() {
            vec![format!("{}:host-gateway", public.auth_host())]
        } else {
            Vec::new()
        };
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
                let dir = self.cfg.shares_dir.display();
                InstallError::Storage(format!("创建共享文件夹目录 {dir} 失败：{e}"))
            })?;
        }
        for path in shares.values() {
            hostdir::ensure_owned_dir(path, self.owner())
                .map_err(|e| InstallError::Storage(format!("准备共享文件夹失败：{e}")))?;
        }
        Ok(InstallContext {
            app_data: self.cfg.apps_dir.join(&slug),
            slug,
            shares,
            app_url,
            oidc,
            secrets: generated,
            tz: self.cfg.tz.clone(),
            puid: self.cfg.puid,
            pgid: self.cfg.pgid,
            extra_hosts,
        })
    }

    /// 主人的用户（PUID:PGID）：共享文件夹、应用数据目录本身和中间各级目录都交给它。
    fn owner(&self) -> Owner {
        Owner {
            uid: self.cfg.puid,
            gid: self.cfg.pgid,
        }
    }

    /// 绑定挂载的源目录由核心预先创建并交给挂载它的服务的属主，否则 Docker 会以 root 身份创建。
    /// 逐级检查、不跟随软链接：应用数据目录里可能有上次安装的插件留下的软链接。
    fn prepare_dirs(
        &self,
        ctx: &InstallContext,
        dirs: &BTreeMap<PathBuf, Owner>,
        undo: &mut Undo,
    ) -> Result<(), InstallError> {
        if dirs.is_empty() {
            return Ok(());
        }
        let storage =
            |e: &dyn std::fmt::Display| InstallError::Storage(format!("准备应用数据目录失败：{e}"));
        fs::create_dir_all(&self.cfg.apps_dir).map_err(|e| storage(&e))?;
        if fs::symlink_metadata(&ctx.app_data)
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            undo.app_dir_created = Some(ctx.app_data.clone());
        }
        hostdir::prepare(&ctx.app_data, self.owner(), dirs).map_err(|e| storage(&e))
    }

    async fn attach_core(
        &self,
        network: &str,
        core: &str,
        manifest: &Manifest,
    ) -> Result<(), InstallError> {
        // 插件容器经这个别名访问核心网关上的身份提供方（令牌、用户信息端点）。
        let aliases = if uses_sso(manifest) {
            vec![self.cfg.public.auth_host()]
        } else {
            Vec::new()
        };
        match self.engine.connect_network(network, core, aliases).await {
            Err(super::docker::EngineError::Rejected { message, .. })
                if message.contains("already exists") =>
            {
                Ok(())
            }
            other => other.map_err(Into::into),
        }
    }

    async fn start_containers(
        &self,
        plan: &Deployment,
        undo: &mut Undo,
        deadline: Instant,
    ) -> Result<Health, InstallError> {
        for container in &plan.containers {
            for (dependency, condition) in &container.depends_on {
                health::wait_for(
                    &self.engine,
                    dependency,
                    Target::Depend(*condition),
                    deadline,
                )
                .await?;
            }
            self.engine
                .create_container(&container.name, container.body.clone())
                .await?;
            undo.containers.push(container.name.clone());
            if !container.files.is_empty() {
                let archive = configs::archive(&container.files).map_err(|e| {
                    InstallError::Invalid(format!("打包 {} 的配置文件失败：{e}", container.name))
                })?;
                self.engine
                    .upload_archive(&container.name, "/", archive)
                    .await?;
            }
            self.engine.start_container(&container.name).await?;
        }
        // 一次性服务已经在被依赖时等到以 0 退出；其余容器都必须健康或稳定运行。
        let mut healthy = Vec::new();
        for container in plan.containers.iter().filter(|c| !c.one_shot) {
            let readiness =
                health::wait_for(&self.engine, &container.name, Target::Ready, deadline).await?;
            healthy.push(readiness == Readiness::Healthy);
        }
        Ok(Health::of(healthy))
    }

    /// 核心访问插件网页的地址：核心在容器里时用容器名（同一网络内的 DNS），否则用容器 IP。
    async fn webui_upstream(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<Option<String>, InstallError> {
        let Some(webui) = &manifest.webui else {
            return Ok(None);
        };
        let name = container_name(&manifest.id.slug(), &webui.service);
        if self.cfg.core_container.is_some() {
            return Ok(Some(format!("{name}:{}", webui.port)));
        }
        let info = self
            .engine
            .inspect_container(&name)
            .await?
            .ok_or_else(|| InstallError::Docker(format!("容器 {name} 不存在")))?;
        let ip = info
            .network_settings
            .and_then(|s| s.networks)
            .and_then(|mut n| n.remove(network))
            .and_then(|e| e.ip_address)
            .filter(|ip| !ip.is_empty())
            .ok_or_else(|| {
                InstallError::Docker(format!("容器 {name} 在网络 {network} 中没有地址"))
            })?;
        Ok(Some(format!("{ip}:{}", webui.port)))
    }

    async fn rollback(&self, undo: Undo) -> Vec<String> {
        let mut failures = Vec::new();
        let mut note = |what: String, result: Result<(), String>| {
            if let Err(e) = result {
                tracing::error!(%what, error = %e, "回滚失败");
                failures.push(format!("{what}：{e}"));
            }
        };
        for name in undo.containers.iter().rev() {
            note(
                format!("容器 {name}"),
                self.engine
                    .remove_container(name)
                    .await
                    .map_err(|e| e.to_string()),
            );
        }
        if let (Some(id), Some(hydra)) = (&undo.oidc_client, &self.oidc) {
            note(
                format!("OIDC 客户端 {id}"),
                hydra.delete_client(id).await.map_err(|e| e.to_string()),
            );
        }
        for volume in &undo.volumes {
            note(
                format!("卷 {volume}"),
                self.engine
                    .remove_volume(volume)
                    .await
                    .map_err(|e| e.to_string()),
            );
        }
        if let Some(network) = &undo.network {
            if let (true, Some(core)) = (undo.core_attached, &self.cfg.core_container) {
                let result = self.engine.disconnect_network(network, core).await;
                note(
                    format!("核心退出网络 {network}"),
                    result.map_err(|e| e.to_string()),
                );
            }
            note(
                format!("网络 {network}"),
                self.engine
                    .remove_network(network)
                    .await
                    .map_err(|e| e.to_string()),
            );
        }
        if let Some(dir) = &undo.app_dir_created {
            note(
                format!("目录 {}", dir.display()),
                fs::remove_dir_all(dir).map_err(|e| e.to_string()),
            );
        }
        if let Some((id, before)) = &undo.secrets_before {
            let store = self.store.secrets();
            let restored = match before {
                Some(secrets) => store.save(id, secrets),
                None => store.remove(id),
            };
            note(
                format!("插件 {id} 的密钥"),
                restored.map_err(|e| e.to_string()),
            );
        }
        failures
    }

    /// 核心重启后核对插件：一次性服务已以 0 退出，长期运行的容器在运行、健康检查没有失败；核心在插件
    /// 网络里、OIDC 客户端已登记、插件密钥都在，并重新收存一次插件写下的密钥。
    pub(super) async fn reconcile(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
    ) -> Result<Deployed, InstallError> {
        let manifest = &package.manifest;
        let slug = manifest.id.slug();
        let one_shot = package.one_shot_services();
        let mut healthy = Vec::new();
        for service in package.service_names() {
            let name = container_name(&slug, service);
            match health::check_now(&self.engine, &name, one_shot.contains(service)).await? {
                Checked::Completed => {}
                Checked::Healthy => healthy.push(true),
                Checked::Running => healthy.push(false),
                Checked::Failed(reason) => return Err(InstallError::Unhealthy(reason)),
            }
        }
        let generated = self.check_secrets(manifest).await?;
        let network = format!("wonton-{slug}");
        if let Some(core) = &self.cfg.core_container {
            self.attach_core(&network, core, manifest).await?;
        }
        if let Some(secret) = &record.oidc_client_secret {
            let ctx = self
                .install_context(manifest, Some(secret), generated)
                .await?;
            self.hydra()?
                .upsert_client(&client_spec(manifest, &ctx, secret))
                .await?;
        }
        Ok(Deployed {
            upstream: self.webui_upstream(manifest, &network).await?,
            health: Health::of(healthy),
        })
    }

    /// 核对插件密钥：生成的都还在（返回它们），插件写下的重新收存一次，变了就更新。
    async fn check_secrets(
        &self,
        manifest: &Manifest,
    ) -> Result<BTreeMap<String, String>, InstallError> {
        if manifest.secrets.is_empty() {
            return Ok(BTreeMap::new());
        }
        let store = self.store.secrets();
        let mut secrets = store.load(&manifest.id)?.unwrap_or_default();
        let generated =
            secrets::generated_values(manifest, &secrets).map_err(InstallError::Storage)?;
        if secrets::collect(&self.engine, manifest, &mut secrets).await? {
            store.save(&manifest.id, &secrets)?;
        }
        Ok(generated)
    }

    pub(super) async fn teardown(
        &self,
        id: &PluginId,
        purge: bool,
    ) -> Result<UninstallReport, InstallError> {
        let found = self.engine.labeled(LABEL_PLUGIN, id.as_str()).await?;
        for container in &found.containers {
            self.engine.remove_container(container).await?;
        }
        for network in &found.networks {
            if let Some(core) = &self.cfg.core_container {
                self.engine.disconnect_network(network, core).await?;
            }
            self.engine.remove_network(network).await?;
        }
        let mut report = UninstallReport {
            containers: found.containers.clone(),
            networks: found.networks.clone(),
            ..Default::default()
        };
        if let Some(hydra) = &self.oidc {
            hydra.delete_client(id.as_str()).await?;
            report.oidc_client_removed = Some(!hydra.client_exists(id.as_str()).await?);
        }
        if purge {
            for volume in &found.volumes {
                self.engine.remove_volume(volume).await?;
            }
            report.volumes = found.volumes.clone();
            let dir = self.cfg.apps_dir.join(id.slug());
            match fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(InstallError::Storage(format!(
                        "删除 {} 失败：{e}",
                        dir.display()
                    )));
                }
            }
            self.store.secrets().remove(id)?;
            report.data_removed = true;
        }
        self.store.remove(id)?;
        let left = self.engine.labeled(LABEL_PLUGIN, id.as_str()).await?;
        if !left.containers.is_empty()
            || !left.networks.is_empty()
            || (purge && !left.volumes.is_empty())
        {
            return Err(InstallError::Docker(format!("卸载后仍有残留：{left:?}")));
        }
        Ok(report)
    }
}

fn client_spec(manifest: &Manifest, ctx: &InstallContext, secret: &str) -> ClientSpec {
    let webui = manifest.webui.as_ref();
    let base = ctx.app_url.clone().unwrap_or_default();
    let oidc = webui.and_then(|w| w.oidc.as_ref());
    ClientSpec {
        id: manifest.id.to_string(),
        secret: secret.to_owned(),
        name: manifest
            .title
            .get("en")
            .cloned()
            .unwrap_or_else(|| manifest.id.to_string()),
        redirect_uris: oidc
            .map(|o| {
                o.redirect_paths
                    .iter()
                    .map(|p| format!("{base}{p}"))
                    .collect()
            })
            .unwrap_or_default(),
        token_endpoint_auth_method: oidc
            .map(|o| o.token_endpoint_auth_method.as_str())
            .unwrap_or("client_secret_basic"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned_by(owner: Option<&str>) -> HashMap<String, String> {
        owner
            .map(|o| HashMap::from([(LABEL_PLUGIN.to_owned(), o.to_owned())]))
            .unwrap_or_default()
    }

    #[test]
    fn named_volumes_are_created_reused_or_refused_by_owner() {
        let plugin = PluginId::parse("app.foo-bar").unwrap();
        let action = |labels: Option<HashMap<String, String>>| {
            volume_action("wonton-app-foo-bar--data", labels.as_ref(), &plugin)
        };
        assert_eq!(action(None).unwrap(), VolumeAction::Create);
        assert_eq!(
            action(Some(owned_by(Some("app.foo-bar")))).unwrap(),
            VolumeAction::Reuse
        );
        // app.foo.bar 和 app.foo-bar 的 slug 相同，它卸载时保留的 data 卷同名：不能挂上别的插件的数据。
        let other = action(Some(owned_by(Some("app.foo.bar")))).unwrap_err();
        assert_eq!(other.kind(), "conflict", "{other}");
        let unlabeled = action(Some(owned_by(None))).unwrap_err();
        assert_eq!(unlabeled.kind(), "conflict", "{unlabeled}");
    }
}
