//! 网关代登录（`sso: proxy`，插件规范的“网关代登录”）：清单声明的请求头和 Cookie 的值模板。网关认出请求的人后把它们
//! 加在转发给插件网页的请求上（见 `gateway::proxy_sso`），浏览器拿不到其中的密钥。
//!
//! 网页模板可用 `{user}`、`{name}`、`{email}`（用户名、显示名和固定用户名派生的内部邮箱）和 `{role}`，以及插件
//! 自己的 `{secret:<名字>}`（`secrets` 里由核心生成的一项），其余文字原样保留。
//! LAN 账号模板没有请求者角色，拒绝 `{role}`；MCP 只接受插件密钥，不接受人的身份。

mod mcp;
mod render;
mod spec;
mod validate;

pub use mcp::{
    McpHeaders, mcp_header_secret_refs, resolve_mcp_headers, resolve_mcp_headers_or_pending,
    validate_mcp_headers,
};
pub use render::{ProxyAuth, RenderError, check_template, fill_template, render_gateway_headers};
pub use spec::validate_header_spec;
pub use validate::{uses_identity, validate, validate_headers};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
