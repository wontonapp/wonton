//! 渲染前的 dry run：用占位环境渲染一遍，不接触 Docker 就检查插件包能否安装。
//!
//! 占位值都指向 `*.wonton.invalid` 一类的假地址；渲染和正式安装走同一条路。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::super::core_address;
use super::super::gpu::GpuGrant;
use super::super::lan::LanGrant;
use super::super::languages::Languages;
use super::super::manifest::{Manifest, ManifestExt, Permission, Sso};
use super::super::names::Instance;
use super::super::package::{PackageError, PluginPackage};
use super::bot_services::BotVars;
use super::requirements::GpuVendor;
use super::{
    Deployment, InstallContext, MatrixVars, ModelVars, OidcVars, OwnerVars, RenderError, render,
};

/// 引用了主人的身份变量却没申请身份权限时，说明缺少的声明。
pub(super) fn explain_owner_vars(
    error: RenderError,
    ctx: &InstallContext,
    langs: &Languages,
) -> RenderError {
    let variable = match &error {
        RenderError::Package(
            PackageError::Interpolate { source, .. }
            | PackageError::ConfigInterpolate { source, .. },
        ) => source.variable(),
        _ => None,
    };
    match (variable, &ctx.owner) {
        (Some(name @ ("WONTON_OWNER_NAME" | "WONTON_OWNER_EMAIL")), None) => {
            RenderError::NoOwnerIdentity(crate::l10n::text(
                langs,
                "plugin-install-render-no-owner-identity",
                &[("var", name.into())],
            ))
        }
        _ => error,
    }
}

/// 用占位环境渲染一遍，在不接触 Docker 的情况下检查插件包能否安装。
pub fn dry_run(package: &PluginPackage, langs: &Languages) -> Result<Deployment, RenderError> {
    render(package, &dry_run_context(package), langs)
}

/// dry run 的显卡占位：只为了 `${WONTON_GPU_VENDOR}` 能插值，真选哪家要在安装时看本机的显卡。
fn dry_run_gpu(manifest: &Manifest) -> Option<GpuGrant> {
    manifest.gpu.as_ref().map(|_| {
        let vendor = manifest
            .requirements
            .as_ref()
            .and_then(|requirements| requirements.gpu.as_ref())
            .and_then(|gpu| gpu.vendors.first())
            .copied()
            .unwrap_or(GpuVendor::Amd);
        GpuGrant::images_only(vendor, Vec::new())
    })
}

/// [`dry_run`] 用的占位环境。
pub(in crate::plugin) fn dry_run_context(package: &PluginPackage) -> InstallContext {
    let manifest = &package.manifest;
    let slug = manifest.id.slug();
    let root = PathBuf::from("/wonton-dry-run");
    let shares = dry_run_shares(manifest, &root);
    let webui = manifest.webui.as_ref();
    let url = format!("http://{slug}.wonton.invalid");
    // 所有密钥项都占位：`collect` 项也会在容器 env 里被引用，漏给会误报“变量未定义”（#775）。
    let secrets = manifest
        .secrets
        .keys()
        .map(|name| (name.clone(), "dry-run".to_owned()))
        .collect();
    let app_data = root.join("apps").join(&slug);
    let capabilities = dry_run_capabilities(manifest, &app_data);
    InstallContext {
        app_data,
        homes: super::private_homes::dry_run_homes(manifest),
        hosted_root: root.join("hosted"),
        relay_dir: PathBuf::from(crate::config::DEFAULT_RELAY_DIR),
        config_store: root.join("core").join("configs"),
        shares,
        app_url: webui.map(|_| url.clone()),
        external_url: crate::plugin::manifest_clients::served(manifest).then(|| url.clone()),
        oidc: webui.filter(|w| w.sso == Sso::Oidc).map(|_| OidcVars {
            issuer: "http://auth.wonton.invalid".into(),
            client_id: manifest.id.to_string(),
            client_secret: "dry-run".into(),
            auth_url: "http://auth.wonton.invalid/auth".into(),
            token_url: "http://auth.wonton.invalid/token".into(),
            userinfo_url: "http://auth.wonton.invalid/userinfo".into(),
        }),
        owner: manifest.gets_owner_identity().then(|| OwnerVars {
            name: "owner".into(),
            email: crate::owner::internal_email("owner"),
        }),
        matrix: manifest
            .permissions
            .contains(&Permission::MatrixBot)
            .then(|| MatrixVars {
                homeserver: "http://matrix.wonton.internal".into(),
                user_id: "@dry-run:wonton.invalid".into(),
                access_token: "dry-run".into(),
                allowed_users: "@owner:wonton.invalid".into(),
                home_room: "!dry-run:wonton.invalid".into(),
            }),
        bots: dry_run_bots(manifest, &capabilities),
        models: dry_run_models(manifest),
        capabilities,
        mcp_url: manifest
            .permissions
            .contains(&Permission::Tools)
            .then(|| "http://mcp.wonton.internal/mcp".into()),
        notify_url: manifest
            .permissions
            .contains(&Permission::Notify)
            .then(|| "http://notify.wonton.internal/notices".into()),
        secrets,
        settings: super::settings::defaults(package),
        instance: Instance::default(),
        tz: "UTC".into(),
        puid: 1000,
        pgid: 1000,
        install_limit: crate::config::DEFAULT_INSTALL_TIMEOUT,
        extra_hosts: Vec::new(),
        gpu: dry_run_gpu(manifest),
        lan: dry_run_lan(manifest, &slug),
        egress: None,
        core_address: webui
            .filter(|w| w.sso == Sso::Proxy)
            .map(|_| core_address::PLACEHOLDER.to_owned()),
        limits: Default::default(),
        memory_total: None,
        host_name: String::new(),
        cgroup_parent: None,
        slug,
        devices: None,
        granted: manifest.effective_permissions(
            &manifest
                .optional_permissions
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
        ),
    }
}

fn dry_run_models(manifest: &Manifest) -> Option<ModelVars> {
    super::models::uses_models(manifest).then(|| ModelVars {
        base_url: "http://models.wonton.internal/v1".into(),
        anthropic_url: Some("http://models.wonton.internal".into()),
        gemini_url: Some("http://models.wonton.internal".into()),
        plugin_key: Some("dry-run".into()),
        bot_keys: manifest
            .bots
            .as_ref()
            .map(|_| {
                BTreeMap::from([(
                    super::bot_services::DEFAULT_BOT.to_owned(),
                    "dry-run".into(),
                )])
            })
            .unwrap_or_default(),
        default_model: "dry-run".into(),
    })
}

fn dry_run_lan(manifest: &Manifest, slug: &str) -> Option<LanGrant> {
    // 192.0.2.1 是文档用的地址（RFC 5737）。
    manifest.lan.as_ref().map(|lan| LanGrant {
        addresses: vec![[192, 0, 2, 1].into()],
        announce: vec![[192, 0, 2, 1].into()],
        network: (!manifest.permissions.contains(&Permission::NetworkEgress))
            .then(|| Instance::default().lan_network(slug)),
        ports: lan.ports.clone(),
        webui: false,
    })
}

fn dry_run_capabilities(manifest: &Manifest, app_data: &Path) -> BTreeMap<String, String> {
    let provider = super::matrix_rtc::provides_matrix_rtc(manifest)
        .then(|| {
            [
                (
                    super::capabilities::MATRIX_SERVER_NAME_VAR.to_owned(),
                    "wonton.invalid".to_owned(),
                ),
                (
                    super::matrix_rtc_ca::CA_DIR_VAR.to_owned(),
                    app_data
                        .join(super::matrix_rtc_ca::CA_DIR)
                        .display()
                        .to_string(),
                ),
            ]
        })
        .into_iter()
        .flatten();
    super::dependencies::declarations(manifest)
        .flat_map(|capability| {
            [
                (
                    super::capabilities::var_url(capability),
                    format!("http://{}.wonton.internal", capability),
                ),
                (
                    super::capabilities::var_key(capability),
                    "dry-run".to_owned(),
                ),
            ]
        })
        .chain(provider)
        .collect()
}

/// dry run 里声明过的共享文件夹占位：指向虚构根下的同名目录。
fn dry_run_shares(manifest: &Manifest, root: &Path) -> BTreeMap<String, PathBuf> {
    manifest
        .permissions
        .iter()
        .filter_map(|p| match p {
            Permission::StorageShare { name, .. } => {
                Some((name.clone(), root.join("shares").join(name)))
            }
            _ => None,
        })
        .collect()
}

/// dry run 里每个 Bot 的占位值：渲染和正式安装走同一条路，多 Bot 的插件每个 Bot 一套。
fn dry_run_bots(
    manifest: &Manifest,
    capabilities: &BTreeMap<String, String>,
) -> BTreeMap<String, BotVars> {
    if manifest.bots.is_none() || !manifest.permissions.contains(&Permission::MatrixBot) {
        return BTreeMap::new();
    }
    let vars = BotVars {
        dependencies: capabilities.clone(),
        model_key: super::models::uses_models(manifest).then(|| "dry-run".into()),
        matrix: Some(MatrixVars {
            homeserver: "http://matrix.wonton.internal".into(),
            user_id: "@dry-run:wonton.invalid".into(),
            access_token: "dry-run".into(),
            allowed_users: "@owner:wonton.invalid".into(),
            home_room: "!dry-run:wonton.invalid".into(),
        }),
        mcp_url: manifest
            .permissions
            .contains(&Permission::Tools)
            .then(|| "http://mcp.wonton.internal/mcp/dry-run".into()),
        ..Default::default()
    };
    BTreeMap::from([(super::bot_services::DEFAULT_BOT.to_owned(), vars)])
}
