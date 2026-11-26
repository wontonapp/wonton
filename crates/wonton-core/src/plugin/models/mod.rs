//! 模型网关（插件规范的“模型网关”）。提供方是声明了 `provides.models` 的插件，同一时间只能装一个；声明了
//! `models` 的插件经核心在插件网络里的别名 `models.wonton.internal` 访问它，各自拿一个只属于自己的凭据。
//!
//! 凭据按 Docker 凭据助手的约定签发：核心在网关的容器里执行声明的命令，值走标准输入，只看退出码；签发后用这个
//! 凭据请求一次 `GET <openai>/models`，返回 200 才算成功。值存在插件记录目录下的 `model-keys/`，网关的数据丢了或
//! 重装了，核心用同一个值重新签发。

use std::time::Duration;

use super::capabilities;
use super::health_gate::Gate;
use super::languages::Languages;
use super::manager::{InstallError, PluginManager, PluginStatus};
use super::manifest::{Manifest, Permission, PluginId};
use super::model_health;
use super::package::PluginPackage;
use super::provides::{ModelsProvider, NAME_PLACEHOLDER};
use super::render;
use super::store::{
    InstalledRecord, StoreError, create_private_dir, read_json, remove_file, write_json,
};
use super::vars::ModelVars;
use crate::config::PLUGIN_MODELS_HOST;
use crate::secret::random_token;

mod errors;
mod issue;
mod keys;
mod reconcile;
mod route;
mod settings;
mod vars;

/// 凭据命令的超时（规范：30 秒）。
pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) use errors::{gateway_absent, gateway_not_running};
pub(super) use errors::{reconcile_failed, unavailable};
pub(super) use keys::KeyFile;
pub use keys::ModelKeyStore;
pub(super) use keys::uses_models;
use keys::{Keys, credential_names};
use route::base_url;
pub use route::{ModelsRoute, ProviderState};
use vars::empty_model_vars;
