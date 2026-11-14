//! 本次安装已创建资源的账本：失败时按它回滚（见 [`rollback`]）。

use std::path::PathBuf;

use super::super::manifest::PluginId;
use super::super::secrets::Secrets;
use crate::matrix::ProvisionedBot;
use wonton_protocol::InstallStage;

/// 本次安装已经创建的资源，失败时据此回滚。
#[derive(Default)]
pub(in crate::plugin) struct Undo {
    pub(super) containers: Vec<String>,
    pub(in crate::plugin) volumes: Vec<String>,
    pub(super) network: Option<String>,
    /// 发布局域网端口的网络（见 [`super::super::lan`]）。
    pub(in crate::plugin) lan_network: Option<String>,
    pub(super) core_attached: bool,
    pub(super) oidc_client: Option<String>,
    pub(super) app_dir_created: Option<PathBuf>,
    /// 本次安装之前插件的密钥（`None` 表示原来没有），回滚时恢复成这样。
    pub(super) secrets_before: Option<(PluginId, Option<Secrets>)>,
    /// 本次安装代 Bot 登录的设备，回滚时吊销（账号、主房间和数据保留给下次安装，见
    /// [`Undo::keep_bot_data`]）。
    pub(in crate::plugin) bot: Option<(PluginId, String, ProvisionedBot)>,
    pub(in crate::plugin) extra_bots: Vec<(PluginId, String, ProvisionedBot)>,
    pub(in crate::plugin) web_plugin: Option<PluginId>,
    /// 本次安装把主房间交给了主人（主人原来不在里面），回滚时收回。
    pub(in crate::plugin) handed_over: bool,
    /// 已登记插件网络的地址段，回滚时注销。
    pub(super) source: Option<PluginId>,
    /// 本次安装在模型网关里签发了凭据的插件，回滚时吊销。
    pub(in crate::plugin) models: Option<PluginId>,
    pub(in crate::plugin) record: Option<PluginId>,
    pub(in crate::plugin) hosted: Vec<(String, PluginId)>,
    pub(in crate::plugin) final_stage: Option<InstallStage>,
}
