//! Agent 插件的模型接入（ADR-0129）：按 Bot 执行清单 `agent.models` 声明的命令，一次一个操作。
//!
//! 核心只转发和把关：不存 key、不带服务商名单，插件输出查过才转给 App。填的 key 只经标准输入交给插件，
//! 插件的输出和标准错误里出现它时先抹掉，日志只记操作名。

use std::time::Duration;

use serde::de::DeserializeOwned;
use wonton_protocol::{
    AgentModelsFailure, BotKeeper, EXIT_INVALID_INPUT, EXIT_NOT_READY, EXIT_UNKNOWN_TARGET,
    KeyCheck, MAX_MODELS_TEXT_CHARS, MODELS_TEST_TIMEOUT_SECS, MODELS_TIMEOUT_SECS, ModelCatalog,
    ModelList, ModelLogin, ModelLoginState, ModelSelection, ModelTest, ModelsCommandInput,
    ModelsInvalidInput, ModelsOp, ModelsOutputProblem,
};

use super::docker::EngineError;
use super::languages::Languages;
use super::{PluginId, PluginManager, PluginStatus};

/// 插件的标准错误放进“详情”时最多留多少字符。
const MAX_DETAIL_CHARS: usize = 1000;
/// 输出和标准错误里替掉 key 的文字。
const REDACTED: &str = "[key]";

/// 一次操作没做成：类别给壳挑文案，`message` 是核心写的一句，`detail` 收进“详情”。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentModelsError {
    pub kind: AgentModelsFailure,
    pub message: String,
    /// `invalid_input` 时哪一项不对。
    pub field: Option<String>,
    pub detail: Option<String>,
}

/// 命令在哪个容器里跑、跑什么，以及这个 Bot 归谁（谁能改它的模型由网关按它判断）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelsTarget {
    pub bot: String,
    pub container: String,
    pub command: Vec<String>,
    pub keeper: BotKeeper,
}

/// 一种操作的输出：命令退出 0 时按它解析，查过才转给 App。不合格的整份不要。
pub(crate) trait ModelsOutput: DeserializeOwned {
    fn checked(self) -> Result<Self, ModelsOutputProblem>;
}

impl ModelsOutput for ModelCatalog {
    fn checked(self) -> Result<Self, ModelsOutputProblem> {
        self.check().map(|()| self)
    }
}

impl ModelsOutput for ModelList {
    fn checked(self) -> Result<Self, ModelsOutputProblem> {
        self.check().map(|()| self)
    }
}

impl ModelsOutput for ModelLogin {
    fn checked(self) -> Result<Self, ModelsOutputProblem> {
        ModelLogin::checked(self)
    }
}

impl ModelsOutput for ModelSelection {
    fn checked(self) -> Result<Self, ModelsOutputProblem> {
        if let Some(current) = &self.current {
            current.check()?;
        }
        message(self.confirm.as_deref())?;
        Ok(self)
    }
}

impl ModelsOutput for KeyCheck {
    fn checked(self) -> Result<Self, ModelsOutputProblem> {
        message(self.message.as_deref()).map(|()| self)
    }
}

impl ModelsOutput for ModelLoginState {
    fn checked(self) -> Result<Self, ModelsOutputProblem> {
        message(self.message.as_deref()).map(|()| self)
    }
}

impl ModelsOutput for ModelTest {
    fn checked(self) -> Result<Self, ModelsOutputProblem> {
        message(self.message.as_deref()).map(|()| self)
    }
}

fn message(text: Option<&str>) -> Result<(), ModelsOutputProblem> {
    match text {
        Some(text) if text.chars().count() > MAX_MODELS_TEXT_CHARS => {
            Err(ModelsOutputProblem("说明太长".into()))
        }
        _ => Ok(()),
    }
}

fn op_name(op: &ModelsOp) -> &'static str {
    match op {
        ModelsOp::Catalog => "catalog",
        ModelsOp::SaveKey { .. } => "save_key",
        ModelsOp::Disconnect { .. } => "disconnect",
        ModelsOp::LoginStart { .. } => "login_start",
        ModelsOp::LoginPoll { .. } => "login_poll",
        ModelsOp::LoginFinish { .. } => "login_finish",
        ModelsOp::LoginCancel { .. } => "login_cancel",
        ModelsOp::Models { .. } => "models",
        ModelsOp::Select { .. } => "select",
        ModelsOp::Test => "test",
    }
}

pub(crate) fn failure(
    langs: &Languages,
    kind: AgentModelsFailure,
    message_id: &str,
    detail: Option<String>,
) -> AgentModelsError {
    AgentModelsError {
        kind,
        message: crate::l10n::text(langs, message_id, &[]),
        field: None,
        detail,
    }
}

fn upstream(langs: &Languages, detail: String) -> AgentModelsError {
    failure(
        langs,
        AgentModelsFailure::Upstream,
        "agent-models-upstream",
        Some(detail),
    )
}

/// 抹掉 key 再截断：插件出错时可能把收到的输入原样打出来。
fn scrub(text: &str, secret: Option<&str>) -> String {
    let text = match secret {
        Some(secret) if !secret.is_empty() => text.replace(secret, REDACTED),
        _ => text.to_owned(),
    };
    text.trim().chars().take(MAX_DETAIL_CHARS).collect()
}

fn engine_failure(langs: &Languages, error: EngineError) -> AgentModelsError {
    match error {
        // 容器不在（404）或者没在跑（409）：Bot 还没起来。
        EngineError::NotFound(detail)
        | EngineError::Rejected {
            message: detail, ..
        } => failure(
            langs,
            AgentModelsFailure::NotReady,
            "agent-models-not-ready",
            Some(detail),
        ),
        EngineError::TimedOut(detail) => failure(
            langs,
            AgentModelsFailure::Upstream,
            "agent-models-timeout",
            Some(detail),
        ),
        other => upstream(langs, other.to_string()),
    }
}

impl PluginManager {
    /// 找到这个 Bot 的模型命令。插件、Bot、声明、运行状态逐项核对，各报各的原因。
    pub(crate) fn agent_models_target(
        &self,
        plugin: &str,
        bot: &str,
        langs: &Languages,
    ) -> Result<ModelsTarget, AgentModelsError> {
        let not_installed = || {
            failure(
                langs,
                AgentModelsFailure::NotInstalled,
                "agent-models-not-installed",
                None,
            )
        };
        let not_ready = |detail: Option<String>| {
            failure(
                langs,
                AgentModelsFailure::NotReady,
                "agent-models-not-ready",
                detail,
            )
        };
        let id = PluginId::parse(plugin).map_err(|_| not_installed())?;
        let (manifest, status) = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            let entry = plugins.get(&id).ok_or_else(not_installed)?;
            // 记录读不出来的插件没有清单可看，当成没就绪，不当成不支持。
            let listed = entry.listed.as_ref().ok_or_else(|| not_ready(None))?;
            (listed.manifest.clone(), entry.status.clone())
        };
        let Some(models) = manifest
            .agent
            .as_ref()
            .and_then(|agent| agent.models.clone())
        else {
            return Err(failure(
                langs,
                AgentModelsFailure::Unsupported,
                "agent-models-unsupported",
                None,
            ));
        };
        let entry = self
            .store
            .bots()
            .entry(&id, bot)
            .map_err(|error| upstream(langs, error.to_string()))?
            .ok_or_else(not_installed)?;
        if !matches!(
            status,
            PluginStatus::Running | PluginStatus::Degraded { .. }
        ) {
            return Err(not_ready(None));
        }
        let slug = id.slug();
        let container =
            if super::bot_services::bot_services(&manifest).contains(models.service.as_str()) {
                self.cfg.instance.bot_container(&slug, &models.service, bot)
            } else {
                self.cfg.instance.container(&slug, &models.service)
            };
        Ok(ModelsTarget {
            bot: bot.to_owned(),
            container,
            command: models.command,
            keeper: entry.keeper,
        })
    }

    /// 执行一个操作并按 `T` 解析、检查输出。
    pub(crate) async fn run_agent_models<T: ModelsOutput>(
        &self,
        target: &ModelsTarget,
        op: ModelsOp,
        langs: &Languages,
    ) -> Result<T, AgentModelsError> {
        let stdout = self.exec_agent_models(target, op, langs).await?;
        let value: T = serde_json::from_str(stdout.trim())
            .map_err(|error| upstream(langs, format!("输出不是这个操作的结果：{error}")))?;
        value
            .checked()
            .map_err(|problem| upstream(langs, problem.0))
    }

    /// 执行一个操作，退出 0 时返回抹过 key 的标准输出。断开、取消登录这类不看输出的操作直接用它。
    pub(crate) async fn exec_agent_models(
        &self,
        target: &ModelsTarget,
        op: ModelsOp,
        langs: &Languages,
    ) -> Result<String, AgentModelsError> {
        let secret = match &op {
            ModelsOp::SaveKey { key, .. } => Some(key.expose().to_owned()),
            _ => None,
        };
        let timeout = Duration::from_secs(if matches!(op, ModelsOp::Test) {
            MODELS_TEST_TIMEOUT_SECS
        } else {
            MODELS_TIMEOUT_SECS
        });
        let name = op_name(&op);
        let input = ModelsCommandInput {
            op,
            bot: target.bot.clone(),
            languages: langs.requested().to_vec(),
        };
        let mut line = serde_json::to_vec(&input).expect("命令输入总能序列化");
        line.push(b'\n');
        let result = self
            .engine
            .exec_output(&target.container, &target.command, Some(&line), timeout)
            .await;
        drop(line);
        let output = result.map_err(|error| engine_failure(langs, error))?;
        tracing::info!(
            op = name,
            container = %target.container,
            exit_code = output.exit_code,
            "Agent 模型接入命令"
        );
        let stdout = scrub(&output.stdout, secret.as_deref());
        let stderr = scrub(&output.stderr, secret.as_deref());
        match output.exit_code {
            0 => Ok(stdout),
            code if code == i64::from(EXIT_INVALID_INPUT) => {
                match serde_json::from_str::<ModelsInvalidInput>(&stdout) {
                    Ok(invalid) if message(Some(&invalid.message)).is_ok() => {
                        Err(AgentModelsError {
                            kind: AgentModelsFailure::InvalidInput,
                            message: invalid.message,
                            field: invalid.field,
                            detail: None,
                        })
                    }
                    _ => Err(upstream(
                        langs,
                        format!("退出码 {code}，输出不合格：{stderr}"),
                    )),
                }
            }
            code if code == i64::from(EXIT_UNKNOWN_TARGET) => Err(failure(
                langs,
                AgentModelsFailure::NotFound,
                "agent-models-not-found",
                (!stderr.is_empty()).then_some(stderr),
            )),
            code if code == i64::from(EXIT_NOT_READY) => Err(failure(
                langs,
                AgentModelsFailure::NotReady,
                "agent-models-not-ready",
                (!stderr.is_empty()).then_some(stderr),
            )),
            code => Err(upstream(langs, format!("退出码 {code}：{stderr}"))),
        }
    }
}

#[cfg(test)]
#[path = "agent_models_support.rs"]
pub(crate) mod support;
