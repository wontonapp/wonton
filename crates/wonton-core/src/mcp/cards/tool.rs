use rmcp::model::Tool;
use serde_json::Value;
use wonton_protocol::{ConfirmationCategory, ConfirmationTool, ConfirmationToolImpact};

use super::{Draft, choices, line};
use crate::l10n;
use crate::mcp::risky;
use crate::plugin::languages::Languages;

pub fn tool_call(
    plugin: &str,
    tool: &Tool,
    arguments: &Value,
    highest: bool,
    langs: &Languages,
) -> Draft {
    let title = tool
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .or_else(|| {
            tool.annotations
                .as_ref()?
                .title
                .as_deref()
                .filter(|title| !title.trim().is_empty())
        })
        .or_else(|| {
            tool.description
                .as_deref()
                .filter(|text| !text.trim().is_empty())
        })
        .unwrap_or(&tool.name);
    let lines = tool
        .description
        .as_deref()
        .filter(|description| !description.trim().is_empty() && *description != title)
        .map(line)
        .into_iter()
        .collect();
    let impact = if risky::deletes(tool) {
        // MCP 不声明删除参数和读对象工具、资源之间的对应关系，不能从 name/id 猜目标。
        ConfirmationToolImpact::Destructive { target: None }
    } else if tool.annotations.as_ref().and_then(|a| a.read_only_hint) == Some(true) {
        ConfirmationToolImpact::ReadOnly
    } else {
        ConfirmationToolImpact::ChangesData
    };
    Draft {
        title: title.to_owned(),
        lines,
        choices: choices(
            &l10n::text(langs, "chat-choice-allow", &[]),
            &l10n::text(langs, "chat-choice-deny", &[]),
        ),
        scope: None,
        plugin_id: None,
        secret_inputs: Vec::new(),
        settings: None,
        category: Some(ConfirmationCategory::ToolCall {
            tool: ConfirmationTool {
                name: tool.name.to_string(),
                plugin_title: plugin.to_owned(),
                arguments: serde_json::to_string_pretty(arguments).expect("JSON value serializes"),
                impact,
                runs_as_root: highest,
            },
        }),
    }
}
