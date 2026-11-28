//! 在插件容器里执行命令（Docker exec）：模型网关的凭据命令就这样执行，秘密值走标准输入，不进命令行参数。

use std::time::Duration;

use bollard::container::LogOutput;
use bollard::exec::StartExecResults;
use bollard::models::ExecConfig;
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use super::docker::{Engine, EngineError, from_bollard};
use crate::l10n;

/// 标准错误只留这么多，写进错误信息够用了。
const STDERR_LIMIT: usize = 4 << 10;
/// 标准输出只留这么多，防止 provider 命令把核心内存撑爆。
// 插件命令的标准输出按模型接入命令的上限收，多收 1 字节：调用方据此分得清“正好到上限”和“超了”。
const STDOUT_LIMIT: usize = wonton_protocol::MAX_MODELS_OUTPUT_BYTES + 1;

/// 命令的结果。凭据命令只看退出码；provider 命令还要读取标准输出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutput {
    pub exit_code: i64,
    pub stdout: String,
    pub stderr: String,
}

impl Engine {
    /// 在容器里执行 `argv`，只保留标准错误。秘密值走标准输入，不进命令行参数。
    pub async fn exec(
        &self,
        container: &str,
        argv: &[String],
        stdin: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<ExecOutput, EngineError> {
        self.exec_inner(container, argv, stdin, timeout, false, |_| Ok(()))
            .await
    }

    /// 在容器里执行 `argv`，同时保留标准输出供 provider 命令解析。
    pub async fn exec_output(
        &self,
        container: &str,
        argv: &[String],
        stdin: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<ExecOutput, EngineError> {
        self.exec_inner(container, argv, stdin, timeout, true, |_| Ok(()))
            .await
    }

    pub(super) async fn exec_tracked(
        &self,
        container: &str,
        argv: &[String],
        stdin: Option<&[u8]>,
        timeout: Duration,
        prepare: impl FnOnce(&str) -> Result<(), EngineError> + Send,
    ) -> Result<ExecOutput, EngineError> {
        self.exec_inner(container, argv, stdin, timeout, false, prepare)
            .await
    }

    /// 命令的记录没了吗（容器重启时 Docker 会丢掉它），或者创建后根本没启动过（没有进程、也没有退出码）：
    /// 这条命令不会再自行改变，剩下的事实由调用方重新验证凭据确定。
    pub(super) async fn exec_finished(&self, id: &str) -> Result<bool, EngineError> {
        match self.docker.inspect_exec(id).await {
            Ok(info) => Ok(info.running == Some(false)
                && (info.exit_code.is_some() || !info.pid.is_some_and(|pid| pid > 0))),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(true),
            Err(error) => Err(from_bollard(&self.owner_languages(), "inspect exec", error)),
        }
    }

    async fn exec_inner(
        &self,
        container: &str,
        argv: &[String],
        stdin: Option<&[u8]>,
        timeout: Duration,
        capture_stdout: bool,
        prepare: impl FnOnce(&str) -> Result<(), EngineError> + Send,
    ) -> Result<ExecOutput, EngineError> {
        let langs = self.owner_languages();
        let context = l10n::text(
            &langs,
            "plugin-install-docker-context-exec",
            &[
                ("container", container.into()),
                ("argv", argv.join(" ").into()),
            ],
        );
        let config = ExecConfig {
            cmd: Some(argv.to_vec()),
            attach_stdin: Some(stdin.is_some()),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..Default::default()
        };
        let created = self
            .docker
            .create_exec(container, config)
            .await
            .map_err(|e| from_bollard(&langs, &context, e))?;
        prepare(&created.id)?;
        let run = async {
            let started = self
                .docker
                .start_exec(&created.id, None)
                .await
                .map_err(|e| from_bollard(&langs, &context, e))?;
            let StartExecResults::Attached {
                mut output,
                mut input,
            } = started
            else {
                return Err(EngineError::Rejected {
                    context: context.clone(),
                    message: l10n::text(&langs, "plugin-install-docker-exec-no-io", &[]),
                });
            };
            let io = |e: std::io::Error| {
                let joined = l10n::text(
                    &langs,
                    "plugin-install-docker-error-join",
                    &[
                        ("context", context.clone().into()),
                        ("error", e.to_string().into()),
                    ],
                );
                EngineError::Unavailable(l10n::text(
                    &langs,
                    "plugin-install-docker-unavailable",
                    &[("error", joined.into())],
                ))
            };
            if let Some(data) = stdin {
                input.write_all(data).await.map_err(io)?;
            }
            input.shutdown().await.map_err(io)?;
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            while let Some(chunk) = output.next().await {
                match chunk.map_err(|e| from_bollard(&langs, &context, e))? {
                    LogOutput::StdOut { message } if capture_stdout => {
                        collect(&mut stdout, &message, STDOUT_LIMIT)
                    }
                    LogOutput::StdErr { message } => collect(&mut stderr, &message, STDERR_LIMIT),
                    _ => {}
                }
            }
            stdout.truncate(STDOUT_LIMIT);
            stderr.truncate(STDERR_LIMIT);
            Ok((stdout, stderr))
        };
        let (stdout, stderr) = tokio::time::timeout(timeout, run).await.map_err(|_| {
            EngineError::TimedOut(l10n::text(
                &langs,
                "plugin-install-docker-exec-timed-out",
                &[
                    ("context", context.clone().into()),
                    ("seconds", timeout.as_secs().into()),
                ],
            ))
        })??;
        let inspected = self
            .docker
            .inspect_exec(&created.id)
            .await
            .map_err(|e| from_bollard(&langs, &context, e))?;
        Ok(ExecOutput {
            exit_code: inspected.exit_code.unwrap_or(-1),
            stdout: String::from_utf8_lossy(&stdout).trim().to_owned(),
            stderr: String::from_utf8_lossy(&stderr).trim().to_owned(),
        })
    }
}

fn collect(buffer: &mut Vec<u8>, message: &[u8], limit: usize) {
    let remaining = limit.saturating_sub(buffer.len());
    buffer.extend_from_slice(&message[..message.len().min(remaining)]);
}
