//! 总线的行为：工具汇总、只读工具直接执行、会改数据的工具和装卸插件要主人确认、拒绝和过期都不执行。插件的 MCP
//! 服务和卡片通道用 [`test_support`] 里的替身；装卸插件用假 Docker。经网关的 HTTP 入口见 `gateway::mcp_tests`。

use std::fs;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, CallToolResult};
use serde_json::{Value, json};
use wonton_protocol::{Answer, CardChoice, ConfirmationState};

use super::AnswerError;
use super::cards::Draft;
use super::confirm::DEFAULT_WINDOW;
use super::test_support::{Notes, serve_notes};
use super::upstream::Upstreams;
use crate::plugin::docker::Engine;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, PluginPackage, PluginStatus};

pub(super) use crate::mcp::test_setup::*;
pub(super) use crate::mcp::tool_failure_tests;

pub(super) fn text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect()
}

pub(super) fn failed(result: &CallToolResult) -> bool {
    result.is_error == Some(true)
}

mod readiness;

mod uninstall;

mod install;

mod confirmations;

mod manifest;

mod timeouts;

mod tools;
