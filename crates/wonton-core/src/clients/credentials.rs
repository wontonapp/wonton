//! 应用自己验证的专用密码（ADR-0021「专用密码」）：签发、验证、吊销、换值。
//!
//! 值只在核心里发，命令在应用的容器里跑：退出码 0 才算生效，失败和超时都是“结果未知”，
//! 记录保持待完成、下次再重试（见 [`super::pending`]）。执行走窄 trait，嵌入测试按表给结果。

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use wonton_protocol::manifest::CredentialsAuth;

use super::passwords::Snapshot;
use crate::plugin::PluginId;
use crate::plugin::docker::EngineError;

/// 一条命令最多跑多久；超时按结果未知重试，不当失败扔掉。
pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// 命令标准输出里取出来的值最长多少字节，超了截断。
const MAX_VALUE: usize = 4096;

/// 执行结果（标准输出只用来取值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecOutput {
    pub exit_code: i64,
    pub stdout: String,
    pub stderr: String,
}

/// 没拿到退出码的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExecError {
    /// 没在 [`COMMAND_TIMEOUT`] 内跑完：应用里可能已经做了，按待完成重试。
    Timeout,
    /// Docker 连不上、容器没了等；`0` 退出码拿不到。
    Engine(String),
}

/// 在插件容器里跑一条命令。生产实现是 [`EngineExec`]，测试按队列给结果。
pub(crate) trait CredentialExec: Send + Sync {
    fn exec<'a>(
        &'a self,
        container: &'a str,
        argv: &'a [String],
        stdin: Option<&'a [u8]>,
        timeout: Duration,
    ) -> BoxFuture<'a, Result<ExecOutput, ExecError>>;
}

/// 走 Docker：值走标准输入，标准输出留给 `generates`。
pub(crate) struct EngineExec {
    engine: Arc<crate::plugin::docker::Engine>,
}

impl EngineExec {
    pub(crate) fn new(engine: std::sync::Arc<crate::plugin::docker::Engine>) -> Self {
        Self { engine }
    }
}

impl CredentialExec for EngineExec {
    fn exec<'a>(
        &'a self,
        container: &'a str,
        argv: &'a [String],
        stdin: Option<&'a [u8]>,
        timeout: Duration,
    ) -> BoxFuture<'a, Result<ExecOutput, ExecError>> {
        Box::pin(async move {
            self.engine
                .exec_output(container, argv, stdin, timeout)
                .await
                .map(|output| ExecOutput {
                    exit_code: output.exit_code,
                    stdout: output.stdout,
                    stderr: output.stderr,
                })
                .map_err(|error| match error {
                    EngineError::TimedOut(_) => ExecError::Timeout,
                    other => ExecError::Engine(other.to_string()),
                })
        })
    }
}

/// 一个插件现在能不能跑凭据命令，命令在哪个容器里。
pub(crate) struct CredentialTarget {
    pub container: String,
    pub credentials: CredentialsAuth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    Issue,
    Verify,
    Revoke,
    Rotate,
}

/// 命令失败或没跑成；记录保持原样等下次。
#[derive(Debug)]
pub(crate) enum CredentialError {
    /// 插件不在跑、没有路由，或者这个插件不用 `credentials`。
    Unavailable,
    /// 清单里没有这个操作（`revoke`/`rotate` 只在对应模式下必写）。
    Missing(Op),
    /// 非 0 退出码，附标准错误输出。
    Failed { code: i64, stderr: String },
    /// 超时：结果未知，按待完成重试。
    Timeout,
}

/// 验证的结果；未知时先不签发，下次启动再看。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verify {
    Valid,
    Invalid,
    Unknown,
}

impl CredentialError {
    /// 给 App 看的说明；没有可写的时候为空。
    pub(crate) fn detail(&self) -> String {
        match self {
            Self::Unavailable => String::new(),
            Self::Missing(op) => format!("{op:?}"),
            Self::Failed { stderr, .. } => stderr.clone(),
            Self::Timeout => String::new(),
        }
    }
}

impl std::fmt::Display for CredentialError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(op) => write!(formatter, "{op:?} 命令没写"),
            Self::Failed { code, stderr } => write!(formatter, "退出码 {code}：{stderr}"),
            Self::Unavailable => write!(formatter, "插件没在跑"),
            Self::Timeout => write!(formatter, "命令超时"),
        }
    }
}

/// 一条命令跑完了、退出码 0；`generated` 是 `generates` 的签发/换值从标准输出第一行取到的值。
#[derive(Debug, Default)]
pub(crate) struct Ran {
    pub generated: Option<String>,
}

/// 清单里的命令，`{name}` 换成密码记录 ID（`shared` 是唯一那条）。
pub(super) fn argv(
    credentials: &CredentialsAuth,
    op: Op,
    name: &str,
) -> Result<Vec<String>, CredentialError> {
    let template = match op {
        Op::Issue => Some(&credentials.issue),
        Op::Verify => credentials.verify.as_ref(),
        Op::Revoke => credentials.revoke.as_ref(),
        Op::Rotate => credentials.rotate.as_ref(),
    }
    .ok_or(CredentialError::Missing(op))?;
    Ok(template
        .iter()
        .map(|argument| argument.replace("{name}", name))
        .collect())
}

/// 标准输出第一行；超过 [`MAX_VALUE`] 截断（值时按字符边界切）。
pub(super) fn first_line(stdout: &str) -> String {
    let line = stdout.lines().next().unwrap_or("").trim();
    let mut end = line.len().min(MAX_VALUE);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].to_owned()
}

impl crate::plugin::PluginManager {
    /// 换上测试用的执行实现。
    #[cfg(test)]
    pub(crate) fn set_credential_exec(&self, exec: Arc<dyn CredentialExec>) {
        *self
            .credential_exec
            .write()
            .expect("credential exec lock poisoned") = exec;
    }

    /// 跑一条凭据命令。`stdin` 是一条秘密值，会补一个换行。
    pub(crate) async fn run_credential(
        &self,
        plugin: &PluginId,
        op: Op,
        name: &str,
        stdin: Option<&str>,
    ) -> Result<Ran, CredentialError> {
        let target = self
            .credential_target(plugin)
            .ok_or(CredentialError::Unavailable)?;
        let argv = argv(&target.credentials, op, name)?;
        let input = stdin.map(|value| format!("{value}\n").into_bytes());
        let exec = self
            .credential_exec
            .read()
            .expect("credential exec lock poisoned")
            .clone();
        let output = exec
            .exec(&target.container, &argv, input.as_deref(), COMMAND_TIMEOUT)
            .await;
        match output {
            Ok(output) if output.exit_code == 0 => Ok(Ran {
                generated: matches!(op, Op::Issue | Op::Rotate).then(|| first_line(&output.stdout)),
            }),
            Ok(output) => Err(CredentialError::Failed {
                code: output.exit_code,
                stderr: output.stderr,
            }),
            Err(ExecError::Timeout) => Err(CredentialError::Timeout),
            Err(ExecError::Engine(message)) => Err(CredentialError::Failed {
                code: -1,
                stderr: message,
            }),
        }
    }

    /// 签发一个值（`generates` 时没有任何标准输入，应用写回标准输出）。
    pub(crate) async fn issue_password(
        &self,
        plugin: &PluginId,
        snapshot: &Snapshot,
        generates: bool,
    ) -> Result<Option<String>, CredentialError> {
        let stdin = (!generates).then_some(snapshot.value.as_str());
        let ran = self
            .run_credential(plugin, Op::Issue, &snapshot.id, stdin)
            .await?;
        if !generates {
            return Ok(None);
        }
        generated_or_empty(ran)
    }

    /// 验证一个值：退出码 0 有效；非 0 无效；超时和 Docker 错误是未知。
    pub(crate) async fn verify_password(&self, plugin: &PluginId, snapshot: &Snapshot) -> Verify {
        match self
            .run_credential(plugin, Op::Verify, &snapshot.id, Some(&snapshot.value))
            .await
        {
            Ok(_) => Verify::Valid,
            Err(CredentialError::Failed { code, .. }) if code > 0 => Verify::Invalid,
            // `verify` 只在 generates 时必填；没写的应用就用核心自己发的值作准。
            Err(CredentialError::Missing(_)) => Verify::Valid,
            Err(_) => Verify::Unknown,
        }
    }

    pub(crate) async fn revoke_password(
        &self,
        plugin: &PluginId,
        snapshot: &Snapshot,
    ) -> Result<(), CredentialError> {
        self.run_credential(plugin, Op::Revoke, &snapshot.id, None)
            .await?;
        Ok(())
    }

    /// 换值：`next` 有值时走标准输入（核心生成），没有时由应用生成（`generates`）。
    pub(crate) async fn rotate_password(
        &self,
        plugin: &PluginId,
        snapshot: &Snapshot,
        next: Option<&str>,
    ) -> Result<Option<String>, CredentialError> {
        let ran = self
            .run_credential(plugin, Op::Rotate, &snapshot.id, next)
            .await?;
        if next.is_some() {
            return Ok(None);
        }
        generated_or_empty(ran)
    }
}

/// `generates` 成功却没输出：写空值会让入口打开却谁也登不进，按失败留在待完成。
pub(super) fn generated_or_empty(ran: Ran) -> Result<Option<String>, CredentialError> {
    let generated = ran.generated.unwrap_or_default();
    if generated.is_empty() {
        return Err(CredentialError::Failed {
            code: 0,
            stderr: "标准输出为空".into(),
        });
    }
    Ok(Some(generated))
}
