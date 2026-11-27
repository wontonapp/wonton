use rmcp::model::{PaginatedRequestParams, Tool};
use rmcp::service::ServiceError;
use tokio::time::Instant;

use super::Upstreams;
use super::error::CallFailure;
use super::limits::{MAX_TOOL_PAGES, MAX_TOOLS, MAX_TOOLS_BYTES};
use super::session::{Connection, Slot};
use crate::plugin::{McpServer, PluginId};

impl Upstreams {
    /// 插件现在提供的工具。卡住的插件最多让调用方等 [`super::TOOLS_TIMEOUT`]（包括没连上时的握手）。
    pub async fn tools(&self, server: &McpServer) -> Result<Vec<Tool>, String> {
        let (listed, problem) = allowlist(server, self.list_tools(server).await);
        let mut clients = self.clients.lock().expect("MCP 客户端表锁中毒");
        if let Some(slot) = clients.get_mut(&server.id) {
            slot.tools_problem = problem.or(listed.as_ref().err().cloned());
        }
        listed
    }

    /// 最近一次列工具为什么失败。
    pub fn tools_problem(&self, id: &PluginId) -> Option<String> {
        let clients = self.clients.lock().expect("MCP 客户端表锁中毒");
        clients.get(id)?.tools_problem.clone()
    }

    /// 不建连接，直接记下列工具的问题（这次没去连的插件用它，见总线列工具时对未配置插件的处理）。
    pub fn note_problem(&self, server: &McpServer, problem: String) {
        let mut clients = self.clients.lock().expect("MCP 客户端表锁中毒");
        clients
            .entry(server.id.clone())
            .or_insert_with(|| Slot::new(server.url.clone(), server.headers.clone()))
            .tools_problem = Some(problem);
    }

    async fn list_tools(&self, server: &McpServer) -> Result<Vec<Tool>, String> {
        let id = &server.id;
        let started = Instant::now();
        let deadline = started + self.tools_timeout;
        let too_slow = || format!("插件 {id} 在 {:?} 内没有列出工具", self.tools_timeout);
        let connection = tokio::time::timeout_at(deadline, self.connection(server))
            .await
            .map_err(|_| too_slow())??;
        let listed = tokio::time::timeout_at(deadline, list_pages(&connection, id)).await;
        match listed {
            Ok(Ok(tools)) => {
                self.answered(id, &connection);
                Ok(tools)
            }
            Ok(Err(ListFailure::Service(e))) => Err(match self.failure(id, &connection, e) {
                CallFailure::TooLarge(_) => format!("插件 {id} 的工具列表超过上限"),
                CallFailure::Replaced => format!("插件 {id} 的会话不回应，已被换掉"),
                CallFailure::Transport(e) | CallFailure::Plugin(e) => {
                    format!("插件 {id} 的 MCP 服务列不出工具：{e}")
                }
            }),
            Ok(Err(ListFailure::Plugin(why))) => {
                self.answered(id, &connection);
                Err(why)
            }
            Err(_) => {
                self.timed_out(id, &connection, started);
                Err(too_slow())
            }
        }
    }
}

/// 清单写了 `mcp.tools`（允许列表）时只提供列表里的工具：调用不在列表里的和调用不存在的工具一样报错
/// （`call_plugin` 和确认策略视图都经 [`Upstreams::tools`] 拿工具）。列表里写了但上游没有的名字不拦别的
/// 工具，缺哪些写在问题里，由 `wonton__list_plugins` 的 `tools_error` 显示；上游整个列不出来时如实报
/// 原来的错。
fn allowlist(
    server: &McpServer,
    listed: Result<Vec<Tool>, String>,
) -> (Result<Vec<Tool>, String>, Option<String>) {
    let Some(declared) = &server.tools else {
        let problem = listed.as_ref().err().cloned();
        return (listed, problem);
    };
    match listed {
        Ok(tools) => {
            let names: std::collections::HashSet<&str> =
                tools.iter().map(|tool| tool.name.as_ref()).collect();
            let missing: Vec<&str> = declared
                .iter()
                .map(String::as_str)
                .filter(|name| !names.contains(name))
                .collect();
            let problem = (!missing.is_empty())
                .then(|| format!("清单声明的工具上游没有：{}", missing.join("、")));
            let provided: std::collections::HashSet<&str> =
                declared.iter().map(String::as_str).collect();
            let kept = tools
                .into_iter()
                .filter(|tool| provided.contains(tool.name.as_ref()))
                .collect();
            (Ok(kept), problem)
        }
        Err(why) => (Err(why), None),
    }
}

/// 列工具为什么失败。
enum ListFailure {
    Service(ServiceError),
    /// 插件回答了，回答不合规矩。
    Plugin(String),
}

/// 自己翻页：限页数、工具数和总字节数，见 [`MAX_TOOL_PAGES`]。
async fn list_pages(connection: &Connection, id: &PluginId) -> Result<Vec<Tool>, ListFailure> {
    let mut tools = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut bytes = 0;
    let mut cursor = None;
    for _ in 0..MAX_TOOL_PAGES {
        let params = PaginatedRequestParams::default().with_cursor(cursor);
        let page = connection
            .peer()
            .list_tools(Some(params))
            .await
            .map_err(ListFailure::Service)?;
        bytes += serde_json::to_vec(&page.tools).map_or(0, |page| page.len());
        tools.extend(page.tools);
        if tools.len() > MAX_TOOLS || bytes > MAX_TOOLS_BYTES {
            return Err(ListFailure::Plugin(format!(
                "插件 {id} 的工具列表超过上限（{MAX_TOOLS} 个工具、{MAX_TOOLS_BYTES} 字节）"
            )));
        }
        let Some(next) = page.next_cursor else {
            return Ok(tools);
        };
        if !seen.insert(next.clone()) {
            return Err(ListFailure::Plugin(format!(
                "插件 {id} 的工具列表翻页翻回了同一个 cursor"
            )));
        }
        cursor = Some(next);
    }
    Err(ListFailure::Plugin(format!(
        "插件 {id} 的工具列表超过 {MAX_TOOL_PAGES} 页"
    )))
}
