//! 插件管理 API（ADR-0017 的“接口：App 和核心”）。接口按权限项分（ADR-0088，[`super::permissions::require`]）：
//! 装卸、升级和可选权限开关要“装、删、升级应用”，读插件、看详情和每个人自己的接口只认已配对设备
//! （[`plugin_denied`] 认出设备后由处理器按人过滤）。响应是 `wonton-protocol` 的类型，文字按 `Accept-Language` 挑语言。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/plugins` | 已安装的插件（`InstalledPlugins`） |
//! | GET | `/_wonton/api/plugins/{id}` | 已安装插件的详情（`PluginDetail`） |
//! | POST | `/_wonton/api/plugins/preview` | 请求体是来源（`InstallSource`）或插件包，返回要主人确认的内容（`InstallPreview`）；上传的包暂存到确认，`source` 是 `upload:sha256:…` |
//! | POST | `/_wonton/api/plugins` | 按来源安装（`InstallRequest`），或者上传插件包加 `?grant=权限,权限`（可加 `&lan_webui=true`）；`grant` 含全部必需权限，可选项放在 `enabled_permissions`（201 `InstalledPlugin`） |
//! | POST | `/_wonton/api/plugins/{id}/retry` | 安装失败且回滚完成时，按核心保留的原包及确认的输入重新安装（204）；原决定不可用或清理未完成时拒绝 |
//! | POST | `/_wonton/api/plugins/{id}/start` | 启动已停止的插件；已运行时不重复启动，就绪并核对完成后返回（200 `InstalledPlugin`）。容器不在时按安装记录照安装的路径重建再启动（网络不在也重建），做不到按原因报类别：镜像拉不到 `image_pull_failed`，安装记录读不出 `record_unusable` |
//! | GET | `/_wonton/api/plugins/{id}/lifecycle` | 与启动接口相同的鉴权，返回持久化意图和恢复日志（200 `PluginLifecycle`），不把容器观测状态当作主人的意图 |
//! | POST | `/_wonton/api/plugins/{id}/stop` | 与启动接口相同的鉴权，持久化明确停止意图；幂等停止后返回（200 `InstalledPlugin`），忙时拒绝，不在后台重新启动 |
//! | PUT | `/_wonton/api/plugins/{id}/permissions/{permission}` | 开关一项可选权限（`PermissionUpdate`），重建完返回详情（200 `PluginDetail`） |
//! | PUT | `/_wonton/api/plugins/{id}/scope` | 主人提交 `PluginScope`；不支持范围设置时返回 501 `unsupported`，不保存 |
//! | GET、PUT | `/_wonton/api/bots/{id}/{bot}/plugin-access/{target}` | 插件 `{id}` 的 Bot `{bot}` 能不能用 `{target}` 的工具（`BotPluginAccess`）；主人 `PUT` 一个 `BotPluginAccessUpdate` 单独开关，总线按它过滤这个 Bot 的工具（见 [`super::bot_plugin_access`]）。Bot 不在 404 `not_found`，`{target}` 没装 `not_installed`，工具策略读不出 503 `policy_unavailable` |
//! | DELETE | `/_wonton/api/plugins/{id}?purge=true` | 卸载；`purge` 时删除数据 |
//! | POST | `/_wonton/api/plugins/{id}/upgrade/preview` | 请求体为空或 `{}` 时取能升到的版本，带 `source` 或者插件包时换到主人选的，可以比装着的旧；返回要主人确认的内容（`InstallPreview`） |
//! | POST | `/_wonton/api/plugins/{id}/upgrade` | 按预览升级（`UpgradeRequest`），或者上传插件包加 `?grant=权限,权限`；换完容器才返回（`UpgradeResult`） |
//! | GET | `/_wonton/api/images/{sha256}/{文件名}` | 插件的图标（PNG），地址由上面的响应给出 |
//!
//! 手动启动把意图设为运行并重置恢复额度。新安装默认要运行；旧记录没有意图时只观察。中断日志保留
//! `category`、`reason`、`stage`、`attempts` 和 Unix 秒 `next_retry_at`，进程中断的类别是 `interrupted`。
//! 恢复最多后台尝试五次，间隔 30、60、120、240、300 秒；核心重启不重置额度。空的重试时间表示执行中或已耗尽。
//!
//! 生命周期查询和停止沿用启动接口的 `InstallApps` 权限项：主人及获授此项的家人都能调用，
//! 未获授的家人不能调用；不另加仅限主人的策略，网页来源限制也与启动相同。
//!
//! 停止先保存 `stopped` 意图和 `stopping` 日志；Docker 停止失败或超时保留原因并显示降级，不安排自动启动。
//!
//! 商店的接口在 [`super::store_api`]，服务器设置和插件资源上限的接口在 [`super::settings_api`]，镜像加速在
//! [`super::registry_mirror`]，模型设置在 [`super::model_settings`]，引导在 [`super::onboarding`]。
//!
//! 插件包是 compose 文本，或者 tar、tar.gz、zip 压缩包（按内容认）；JSON 请求体（`Content-Type: application/json`）按来源预览和安装。

pub(super) use super::api_body::{MAX_UPLOAD_BYTES, invalid_body, json_body, package};
use super::{
    Gateway, Person, TunnelPeer, api_switch, bot_projection, install_gate_checks, install_gates,
    isolation, pages, permissions, sharing,
};
use hyper::HeaderMap;
use hyper::header::ACCEPT_LANGUAGE;
use serde::Deserialize;

pub use super::api_switch::set_plugin_permission;
use crate::l10n;
use crate::plugin::languages::Languages;

mod auth;
mod errors;
mod icon;
mod install;
mod listing;
mod upgrade;

pub(super) use auth::{
    acted_by, current_permitted_requester, current_requester, response_for_requester,
};
pub(crate) use auth::{denied, from_another_page, has_owner_token, plugin_denied, requester};
pub(crate) use errors::{error_response, install_error, retry_later};
pub use icon::image;
pub use install::{install, preview, retry, uninstall};
pub use listing::{detail, list};
pub use upgrade::{preview_upgrade, upgrade};

pub(super) fn languages(headers: &HeaderMap) -> Languages {
    Languages::from_header(headers.get(ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok()))
}

/// 带一个 `{error}` 参数的接口文字。
pub(super) fn text1(langs: &Languages, id: &'static str, error: &str) -> String {
    l10n::text(langs, id, &[("error", error.into())])
}

#[derive(Clone, Deserialize)]
pub struct InstallQuery {
    #[serde(default)]
    pub(super) grant: String,
    /// 主人打开了开关 `lan_webui`。
    #[serde(default)]
    pub(super) lan_webui: bool,
    /// 主人打开了的可选项，逗号分隔（ADR-0057）。
    #[serde(default)]
    pub(super) enabled_permissions: String,
    /// 主人选的设备 ID，逗号分隔（ADR-0036）；上传插件包时用。
    #[serde(default)]
    pub(super) devices: String,
}

#[derive(Deserialize)]

pub struct UninstallQuery {
    #[serde(default)]
    purge: bool,
}

/// 查询串里逗号分隔的列表（设备 ID、可选项）。
pub(super) fn comma_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}
