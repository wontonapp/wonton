//! 模型网关、MCP 服务、托管能力的入口，以及宿主网络的中继套接字。

use std::collections::BTreeMap;
use std::path::Path;

use super::super::host_relay::{self, HostRelay, Purpose};
use super::super::manager::{InstallError, PluginManager};
use super::super::manifest::Manifest;
use super::super::models::ModelsRoute;
use super::super::names;
use super::super::proxy_auth;
use super::super::secrets::Secrets;
use super::Endpoint;
use super::McpTarget;
use crate::l10n;

/// MCP 服务没写 `path` 时的路径。
const DEFAULT_MCP_PATH: &str = "/mcp";

impl PluginManager {
    /// 托管能力的提供方（ADR-0080）：网关把托管插件的请求转到这些地址。容器不在时少了那一项，
    /// 网关如实回“提供方没有在运行”。
    pub(in crate::plugin) async fn hosted_upstreams(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<BTreeMap<String, Endpoint>, InstallError> {
        let mut found = BTreeMap::new();
        let Some(provides) = &manifest.provides else {
            return Ok(found);
        };
        let slug = manifest.id.slug();
        for (capability, spec) in &provides.capabilities {
            if super::super::capabilities::lookup(capability).is_none_or(|c| c.hosted.is_none()) {
                continue;
            }
            let name = self.cfg.instance.container(&slug, &spec.service);
            let host = names::service_host(&slug, &spec.service);
            if let Some(address) = self
                .service_address(&name, &host, spec.port, network)
                .await?
            {
                found.insert(capability.clone(), Endpoint::Tcp(address));
            }
        }
        Ok(found)
    }
    /// 是模型网关时：核心转发模型请求、执行凭据命令的目标。
    pub(in crate::plugin) async fn models_upstream(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<Option<ModelsRoute>, InstallError> {
        let Some(spec) = manifest.provides.as_ref().and_then(|p| p.models.as_ref()) else {
            return Ok(None);
        };
        let slug = manifest.id.slug();
        let name = self.cfg.instance.container(&slug, &spec.service);
        let Some(upstream) = self
            .service_address(
                &name,
                &names::service_host(&slug, &spec.service),
                spec.port,
                network,
            )
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(ModelsRoute {
            id: manifest.id.clone(),
            spec: spec.clone(),
            upstream,
            credentials_container: self
                .cfg
                .instance
                .container(&slug, spec.credentials_service()),
        }))
    }
    /// 声明了 `mcp` 时：MCP 总线连插件 MCP 服务的地址（`http://主机:端口/路径`），和按插件密钥解析好的
    /// 请求头。密钥还没有值、不是文本或者填不出合法的头值时安装失败：总线的头填不出来，宁可不上线；必填的
    /// 用户填写密钥没填时例外，头先空着，总线按未就绪拒绝（ADR-0064）。
    pub(in crate::plugin) async fn mcp_upstream(
        &self,
        manifest: &Manifest,
        network: &str,
        secrets: &Secrets,
    ) -> Result<Option<McpTarget>, InstallError> {
        let Some(mcp) = &manifest.mcp else {
            return Ok(None);
        };
        // 插件交的东西不对（密钥没写下、不是文本、填不出合法头值）：不是核心存储的错，按输入错误报。
        let langs = self.owner_languages();
        let headers = proxy_auth::resolve_mcp_headers_or_pending(manifest, secrets, &langs)
            .map_err(|e| {
                InstallError::Invalid(l10n::text(
                    &langs,
                    "plugin-install-upstream-mcp-headers",
                    &[
                        ("id", manifest.id.to_string().into()),
                        ("error", e.to_string().into()),
                    ],
                ))
            })?;
        let path = mcp.path.as_deref().unwrap_or(DEFAULT_MCP_PATH);
        if let Some(endpoint) = self
            .host_relay_endpoint(manifest, Purpose::Mcp, &mcp.service, mcp.port)
            .await?
        {
            return Ok(Some(McpTarget {
                url: endpoint.url(path),
                socket: endpoint.socket().map(Path::to_path_buf),
                headers,
            }));
        }
        let slug = manifest.id.slug();
        let name = self.cfg.instance.container(&slug, &mcp.service);
        let Some(upstream) = self
            .service_address(
                &name,
                &names::service_host(&slug, &mcp.service),
                mcp.port,
                network,
            )
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(McpTarget {
            url: format!("http://{upstream}{path}"),
            socket: None,
            headers,
        }))
    }

    /// 服务是宿主网络、又该有中继时套接字地址；普通网络为空。
    pub(in crate::plugin) async fn host_relay_endpoint(
        &self,
        manifest: &Manifest,
        purpose: Purpose,
        service: &str,
        port: u16,
    ) -> Result<Option<Endpoint>, InstallError> {
        let name = self.cfg.instance.container(&manifest.id.slug(), service);
        let host = self
            .engine
            .inspect_container(&name)
            .await?
            .and_then(|info| info.host_config)
            .and_then(|config| config.network_mode)
            .is_some_and(|mode| mode == "host");
        if !host {
            return Ok(None);
        }
        let relay = HostRelay {
            purpose,
            service: service.to_owned(),
            port,
        };
        Ok(Some(Endpoint::unix(host_relay::socket_path(
            &self.cfg.relay_dir,
            &self.cfg.instance,
            &manifest.id.slug(),
            &relay,
        ))))
    }
}
