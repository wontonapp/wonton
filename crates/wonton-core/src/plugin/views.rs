//! 管理接口给 App 的视图（ADR-0017 的“接口：App 和核心”）：已安装的插件、详情页、安装预览和图标。文字按请求的
//! 语言挑好，App 原样显示。

use std::sync::Arc;

use wonton_protocol::{
    CardLine, InstallPreview, InstalledPlugin, InstalledPlugins, PermissionLine, PluginAuthor,
    PluginCategory, PluginDetail, PluginHealth, PluginKind as Kind, PluginState, PluginTrust,
};

use super::gallery::{IconSource, Variant};
use super::image_worker::ImageError;
use super::languages::Languages;
use super::manager::{Entry, Health, InstallError, PluginManager, PluginStatus, supported};
use super::manifest::{
    Access, Arch, GpuRequirement, Manifest, Permission, PluginId, PluginKind, Sso,
};
use super::package::PluginPackage;
use super::render::dry_run;
use super::store_info::Category;

/// 已安装插件在列表和详情页里显示的信息，装上时从插件包里取出。
#[derive(Debug)]
pub struct Listed {
    pub manifest: Manifest,
    pub icon: Option<IconSource>,
    /// 装上的时间（Unix 秒），列表按它排序。
    pub installed_at: u64,
}

impl Listed {
    pub fn new(package: &PluginPackage, installed_at: u64) -> Arc<Self> {
        Arc::new(Self {
            manifest: package.manifest.clone(),
            icon: IconSource::of(package),
            installed_at,
        })
    }

    #[cfg(test)]
    pub fn of_manifest(manifest: Manifest) -> Arc<Self> {
        Arc::new(Self {
            manifest,
            icon: None,
            installed_at: 0,
        })
    }
}

impl PluginManager {
    /// 已安装的插件：系统插件在前，其余按安装的先后。
    pub fn installed(&self, langs: &Languages) -> InstalledPlugins {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let mut listed: Vec<_> = plugins
            .iter()
            .map(|(id, entry)| {
                let installed_at = entry.listed.as_ref().map_or(0, |l| l.installed_at);
                (
                    (!entry.system, installed_at),
                    self.listing(id, entry, langs),
                )
            })
            .collect();
        drop(plugins);
        listed.sort_by(|(a, x), (b, y)| a.cmp(b).then_with(|| x.id.cmp(&y.id)));
        InstalledPlugins {
            plugins: listed.into_iter().map(|(_, p)| self.with_bot(p)).collect(),
            upgrade_check: None,
        }
    }

    /// 一个已安装的插件；没装时为空。
    pub fn installed_plugin(&self, id: &PluginId, langs: &Languages) -> Option<InstalledPlugin> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let plugin = self.listing(id, plugins.get(id)?, langs);
        drop(plugins);
        Some(self.with_bot(plugin))
    }

    /// 已安装插件的详情页（装着的版本）；没装时为空。上传的插件包没有来源，也就没有截图。
    pub fn installed_detail(&self, id: &PluginId, langs: &Languages) -> Option<PluginDetail> {
        let listed = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins.get(id)?.listed.clone()
        };
        let installed = self.installed_plugin(id, langs)?;
        let Some(listed) = listed else {
            return Some(broken_detail(installed));
        };
        let m = &listed.manifest;
        Some(PluginDetail {
            source: None,
            id: installed.id.clone(),
            title: installed.title.clone(),
            summary: installed.summary.clone(),
            about: m.about.as_ref().map(|t| langs.pick_owned(t)),
            icon: installed.icon.clone(),
            screenshots: Vec::new(),
            category: installed.category,
            tags: m.tags.clone(),
            kind: installed.kind,
            version: m.version.clone(),
            trust: installed.trust,
            author: author(m),
            homepage: m.homepage.clone(),
            repository: None,
            license: m.license.clone(),
            stars: None,
            release_notes: m.release_notes.as_ref().map(|t| langs.pick_owned(t)),
            permissions: m.permissions.iter().map(permission_line).collect(),
            requirements: requirement_lines(m),
            has_web: m.webui.is_some(),
            installed: Some(installed),
            blocked: None,
        })
    }

    /// 安装上传的插件包之前给主人确认的内容。插件包不合法时报错；合法但这台主机上装不了时，原因写在 `blocked`。
    pub async fn preview_install(
        &self,
        package: &PluginPackage,
        langs: &Languages,
    ) -> Result<InstallPreview, InstallError> {
        let plan = dry_run(package)?;
        let icon = self.check_icon(package).await?;
        if let Some(icon) = &icon {
            self.gallery.remember_preview(icon);
        }
        let m = &package.manifest;
        let blocked = match self.precheck(m).await {
            Ok(()) => None,
            Err(InstallError::Unsupported(reason) | InstallError::Conflict(reason)) => Some(reason),
            Err(e) => return Err(e),
        };
        // 显卡：本机会给什么、或者为什么给不了；required 而给不了也是这台主机上装不了的原因。
        let gpu = self.gpu_preview(m).await?;
        let blocked = blocked.or(gpu.blocked);
        let unpinned: Vec<String> = plan
            .containers
            .iter()
            .filter(|c| !c.image.contains("@sha256:"))
            .map(|c| format!("{}（{}）", c.service, c.image))
            .collect();
        let mut notices = Vec::new();
        if !unpinned.is_empty() {
            notices.push(line(format!("镜像没有按摘要固定：{}", unpinned.join("、"))));
        }
        if m.webui.as_ref().is_some_and(|w| w.sso != Sso::None) {
            notices.push(line("在 App 里打开它的网页时自动登录".to_owned()));
        }
        notices.extend(requirement_lines(m).into_iter().map(line));
        notices.extend(gpu.notice);
        Ok(InstallPreview {
            source: None,
            commit: None,
            digest: package.digest(),
            id: m.id.to_string(),
            title: langs.pick_owned(&m.title),
            summary: langs.pick_owned(&m.description),
            version: m.version.clone(),
            kind: kind(m.kind),
            icon: icon.map(|i| i.paths()),
            trust: PluginTrust::Local,
            author: author(m),
            permissions: m.permissions.iter().map(permission_line).collect(),
            notices,
            upgrade_from: None,
            release_notes: m.release_notes.as_ref().map(|t| langs.pick_owned(t)),
            blocked,
        })
    }

    /// 处理插件包的图标：不合规格时安装失败（插件规范的“图标和截图”）。没有图标时为空。
    pub(super) async fn check_icon(
        &self,
        package: &PluginPackage,
    ) -> Result<Option<IconSource>, InstallError> {
        let Some(icon) = IconSource::of(package) else {
            return Ok(None);
        };
        self.gallery.render(&icon).await.map_err(|e| match e {
            ImageError::Refused(reason) => {
                InstallError::Invalid(format!("图标 {}：{reason}", icon.path))
            }
            ImageError::Unavailable(reason) => InstallError::ImagesUnavailable(reason),
        })?;
        Ok(Some(icon))
    }

    /// 图标地址对应的 PNG：只有已安装的插件和最近预览过的插件包的图标。不认识的地址为 `Ok(None)`。
    pub async fn icon_png(
        &self,
        sha256: &str,
        variant: Variant,
    ) -> Result<Option<Vec<u8>>, ImageError> {
        let installed = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins
                .values()
                .filter_map(|entry| entry.listed.as_ref()?.icon.clone())
                .find(|icon| icon.sha256 == sha256)
        };
        let Some(source) = installed.or_else(|| self.gallery.preview(sha256)) else {
            return Ok(None);
        };
        let icon = self.gallery.render(&source).await?;
        Ok(Some(match variant {
            Variant::Small => icon.small.clone(),
            Variant::Large => icon.large.clone(),
        }))
    }

    /// 这台主机上装不了的原因，和安装时的检查一致，但什么也不改。
    async fn precheck(&self, m: &Manifest) -> Result<(), InstallError> {
        if m.kind == PluginKind::System {
            return Err(InstallError::Unsupported(
                "系统插件只能随核心预装".to_owned(),
            ));
        }
        {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            if let Some(entry) = plugins.get(&m.id) {
                return Err(InstallError::Conflict(format!(
                    "已经装了 {}（版本 {}）",
                    m.id, entry.version
                )));
            }
            let slug = m.id.slug();
            if let Some(other) = plugins.keys().find(|id| id.slug() == slug) {
                return Err(InstallError::Conflict(format!(
                    "和已安装的 {other} 用同一个网页地址"
                )));
            }
        }
        if let Some(p) = m.permissions.iter().find(|p| !supported(p)) {
            return Err(InstallError::Unsupported(format!("核心还不支持权限 {p}")));
        }
        self.check_requirements(m).await
    }

    /// 列表里的一项，还没填 Bot。
    fn listing(&self, id: &PluginId, entry: &Entry, langs: &Languages) -> InstalledPlugin {
        let manifest = entry.listed.as_ref().map(|l| &l.manifest);
        InstalledPlugin {
            id: id.to_string(),
            title: langs.pick_owned(&entry.title),
            summary: manifest.map_or_else(String::new, |m| langs.pick_owned(&m.description)),
            version: entry.version.clone(),
            kind: manifest.map_or(Kind::Other, |m| kind(m.kind)),
            category: manifest.and_then(|m| m.category).map(category),
            icon: entry
                .listed
                .as_ref()
                .and_then(|l| l.icon.as_ref())
                .map(IconSource::paths),
            trust: if entry.system {
                PluginTrust::System
            } else {
                PluginTrust::Local
            },
            source: None,
            status: state(&entry.status, entry.health),
            web_url: entry.has_webui.then(|| self.web_url(id)),
            bot_user_id: None,
            upgrade: None,
            removable: !entry.system,
            notices: Vec::new(),
        }
    }

    /// 填上插件的 Bot：读 Bot 记录要读文件，不在插件表的锁里做。
    fn with_bot(&self, mut plugin: InstalledPlugin) -> InstalledPlugin {
        let Ok(id) = PluginId::parse(&plugin.id) else {
            return plugin;
        };
        match self.store.bots().load(&id) {
            Ok(record) => plugin.bot_user_id = record.map(|r| r.account.user_id),
            Err(e) => tracing::warn!(plugin = %id, error = %e, "读 Bot 记录失败"),
        }
        plugin
    }
}

/// 权限的说明：安装确认页、Bot 的确认卡片和插件目录的搜索结果都用它。
pub fn permission_text(permission: &Permission) -> String {
    let access = |access: &Access| match access {
        Access::Read => "读",
        Access::Write => "读写",
    };
    match permission {
        Permission::StorageShare { name, access: a } => format!("{}共享文件夹 {name}", access(a)),
        Permission::StorageHome { access: a } => format!("{}你的私人空间", access(a)),
        Permission::Models => "使用模型网关".to_owned(),
        Permission::Tools => "调用核心和其他插件的工具".to_owned(),
        Permission::MatrixBot => "以 Bot 身份收发消息".to_owned(),
        Permission::Notify => "给你发通知".to_owned(),
        Permission::NetworkEgress => "访问外网和局域网".to_owned(),
        Permission::NetworkHost => "使用宿主的网络（高权限）".to_owned(),
        Permission::DevicesGpu => "使用 GPU（高权限）".to_owned(),
        Permission::Device { path } => format!("访问宿主设备 {path}（高权限）"),
        Permission::DockerSocket => "控制 Docker（最高权限）".to_owned(),
    }
}

fn permission_line(permission: &Permission) -> PermissionLine {
    PermissionLine {
        permission: permission.to_string(),
        text: permission_text(permission),
        high: permission.is_high(),
        supported: supported(permission),
        added: false,
    }
}

/// 对宿主的要求，一项一行。
fn requirement_lines(manifest: &Manifest) -> Vec<String> {
    let Some(r) = &manifest.requirements else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    if let Some(arch) = &r.arch {
        let names: Vec<&str> = arch
            .iter()
            .map(|a| match a {
                Arch::Amd64 => "amd64",
                Arch::Arm64 => "arm64",
            })
            .collect();
        lines.push(format!("只支持 {} 架构", names.join("、")));
    }
    if let Some(memory) = &r.memory {
        lines.push(format!("需要 {memory} 内存"));
    }
    match r.gpu {
        Some(GpuRequirement::Required) => lines.push("需要 GPU".to_owned()),
        Some(GpuRequirement::Optional) => lines.push("有 GPU 时会用".to_owned()),
        None => {}
    }
    lines
}

fn line(text: String) -> CardLine {
    CardLine {
        text,
        emphasis: false,
    }
}

fn author(manifest: &Manifest) -> Option<PluginAuthor> {
    manifest.author.as_ref().map(|a| PluginAuthor {
        name: a.name.clone(),
        url: a.url.clone(),
    })
}

/// 记录读不出来的插件：只有列表里的那些信息。
fn broken_detail(installed: InstalledPlugin) -> PluginDetail {
    PluginDetail {
        source: None,
        id: installed.id.clone(),
        title: installed.title.clone(),
        summary: installed.summary.clone(),
        about: None,
        icon: None,
        screenshots: Vec::new(),
        category: None,
        tags: Vec::new(),
        kind: installed.kind,
        version: installed.version.clone(),
        trust: installed.trust,
        author: None,
        homepage: None,
        repository: None,
        license: None,
        stars: None,
        release_notes: None,
        permissions: Vec::new(),
        requirements: Vec::new(),
        has_web: false,
        installed: Some(installed),
        blocked: None,
    }
}

fn state(status: &PluginStatus, health: Option<Health>) -> PluginState {
    match status {
        PluginStatus::Installing => PluginState::Installing,
        PluginStatus::Running => PluginState::Running {
            health: match health {
                Some(Health::Healthy) => PluginHealth::Healthy,
                Some(Health::Unknown) | None => PluginHealth::Unknown,
            },
        },
        PluginStatus::Degraded { reason } => PluginState::Degraded {
            reason: reason.clone(),
        },
        PluginStatus::Uninstalling => PluginState::Uninstalling,
    }
}

fn kind(kind: PluginKind) -> Kind {
    match kind {
        PluginKind::App => Kind::App,
        PluginKind::Agent => Kind::Agent,
        PluginKind::Channel => Kind::Channel,
        PluginKind::Model => Kind::Model,
        PluginKind::System => Kind::System,
    }
}

fn category(category: Category) -> PluginCategory {
    match category {
        Category::Ai => PluginCategory::Ai,
        Category::Agent => PluginCategory::Agent,
        Category::Photos => PluginCategory::Photos,
        Category::Media => PluginCategory::Media,
        Category::Files => PluginCategory::Files,
        Category::Documents => PluginCategory::Documents,
        Category::Productivity => PluginCategory::Productivity,
        Category::Communication => PluginCategory::Communication,
        Category::Home => PluginCategory::Home,
        Category::Network => PluginCategory::Network,
        Category::Security => PluginCategory::Security,
        Category::Developer => PluginCategory::Developer,
        Category::Downloads => PluginCategory::Downloads,
        Category::Tools => PluginCategory::Tools,
        Category::System => PluginCategory::System,
    }
}
