//! 插件侧支撑客户端入口（ADR-0021、ADR-0022）：`clients.http` 的转发地址解析，和 `setup_done`
//! 的执行与缓存。开关的状态在 [`crate::clients::state`]。

use super::manager::{ClientsRoute, InstallError, PluginManager};
use super::manifest::{Manifest, PluginId};
use super::models::COMMAND_TIMEOUT;
use super::names;
use super::upstreams::Endpoint;

/// `setup_done` 的一次结果（ADR-0022）：退出码 0 是完成，1 是还没有；其他退出码和超时算查不出，
/// 标准错误输出交给入口回 503 `plugin_unavailable`。
pub(crate) enum SetupCheck {
    Done,
    Pending,
    Undetermined(String),
}

impl PluginManager {
    /// 声明了 `clients.http` 时的转发地址：核心在容器里时按服务的稳定域名（`服务名.<slug>.wonton.internal`）
    /// 解析出地址。网页的宿主网络中继不适用：客户端入口只走插件网络（ADR-0021）。
    pub(super) async fn clients_upstream(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<Option<ClientsRoute>, InstallError> {
        let Some(http) = manifest
            .clients
            .as_ref()
            .and_then(|clients| clients.http.as_ref())
        else {
            return Ok(None);
        };
        let slug = manifest.id.slug();
        let name = self.cfg.instance.container(&slug, &http.service);
        let address = self
            .service_address(
                &name,
                &names::service_host(&slug, &http.service),
                http.port,
                network,
            )
            .await?;
        Ok(Some(ClientsRoute {
            http: http.clone(),
            upstream: address.map(Endpoint::Tcp),
        }))
    }

    /// ADR-0022 的 `setup_done`：在 `clients.http.service` 的容器里跑清单声明的命令（argv 不经 shell），
    /// 超时 30 秒。查到完成就缓存到插件重新部署（部署会换掉整个插件表项）；没有完成和查不出每次现查，
    /// 不缓存「还没有」：主人注册完管理员，下一个请求就要能过。
    pub(crate) async fn clients_setup_done(
        &self,
        id: &PluginId,
        service: &str,
        argv: &[String],
    ) -> SetupCheck {
        if self.setup_done_cached(id) == Some(true) {
            return SetupCheck::Done;
        }
        let container = self.cfg.instance.container(&id.slug(), service);
        let output = self
            .engine
            .exec(&container, argv, None, COMMAND_TIMEOUT)
            .await;
        match output {
            Ok(output) if output.exit_code == 0 => {
                let mut plugins = self.plugins.write().expect("插件表锁中毒");
                if let Some(entry) = plugins.get_mut(id) {
                    entry.setup_done = Some(true);
                }
                SetupCheck::Done
            }
            Ok(output) if output.exit_code == 1 => SetupCheck::Pending,
            Ok(output) => SetupCheck::Undetermined(output.stderr),
            Err(error) => SetupCheck::Undetermined(error.to_string()),
        }
    }

    fn setup_done_cached(&self, id: &PluginId) -> Option<bool> {
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .and_then(|entry| entry.setup_done)
    }
}
