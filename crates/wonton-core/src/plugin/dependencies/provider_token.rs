//! 提供方令牌（ADR-0061 第 3 节）：核心给每个提供 `agent_acp` 的插件发一把随机令牌，只注入它声明的
//! 网关服务（[`capabilities::AGENT_ACP_TOKEN_VAR`]）；核心转发连接时带在 `Authorization: Bearer <令牌>`
//! 里，网关不认没有它的连接。同一插件里的其他服务、伪造请求头都拿不到它。
//!
//! 令牌按（能力、提供方）存进能力记录（`connections.json` 的 `provider_tokens`），装或升级提供方时
//! 生成，卸载时删除（重装发新的，旧令牌随之作废）。

use std::collections::BTreeMap;

use super::capabilities::{self, AGENT_ACP_TOKEN_VAR};
use super::{InstallError, Manifest, PluginId, PluginManager};

/// 令牌长度：16 字节随机数的十六进制，32 个字符（同 Servarr 自生成 key 的形状）。
const TOKEN_BYTES: usize = 16;

impl PluginManager {
    /// 提供方的注入变量（ADR-0061 第 3 节）：提供 `agent_acp` 时带上 [`AGENT_ACP_TOKEN_VAR`]。
    /// compose 里哪个服务引用它，令牌就进哪个服务——网关服务引用、别的服务不引用。
    pub(in crate::plugin) fn agent_acp_provider_vars(
        &self,
        manifest: &Manifest,
    ) -> Result<BTreeMap<String, String>, InstallError> {
        if !capabilities::is_provided(&capabilities::AGENT_ACP, manifest) {
            return Ok(BTreeMap::new());
        }
        let token = self.ensure_provider_token(capabilities::AGENT_ACP.name, &manifest.id)?;
        Ok(BTreeMap::from([(AGENT_ACP_TOKEN_VAR.to_owned(), token)]))
    }

    /// 这个提供方在这项能力上的令牌。没有（提供方装在带令牌的版本之前）就 None，转发方要拒绝。
    pub(crate) fn provider_token(&self, capability: &str, provider: &PluginId) -> Option<String> {
        self.dependencies
            .inspect(|state| state.provider_token(capability, provider).cloned())
            .ok()
            .flatten()
    }

    /// 有则取，没有就生成一把。提供方的清单渲染（注入变量）在装、升级、重建时都走这里；
    /// 并发装同一个提供方时 `or_insert` 保留先到的那把，两边拿到同一个值。
    pub(crate) fn ensure_provider_token(
        &self,
        capability: &str,
        provider: &PluginId,
    ) -> Result<String, InstallError> {
        if let Some(token) = self.provider_token(capability, provider) {
            return Ok(token);
        }
        let token = new_token();
        let key = token_key(capability, provider);
        self.dependencies.change(|state| {
            state.provider_tokens.entry(key).or_insert(token.clone());
        })?;
        Ok(self
            .provider_token(capability, provider)
            .expect("刚写入的令牌一定读得到"))
    }
}

/// 记录键：`<能力>/<插件 ID>`。两类名字都不含 `/`，不会撞。
fn token_key(capability: &str, provider: &PluginId) -> String {
    format!("{capability}/{}", provider.as_str())
}

fn new_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).expect("操作系统随机数源不可用");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
