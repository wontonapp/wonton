//! 在插件容器里执行命令（Docker exec）：模型网关的凭据命令就这样执行，秘密值走标准输入，不进命令行参数。

use std::time::Duration;

use bollard::container::LogOutput;
use bollard::exec::StartExecResults;
use bollard::models::ExecConfig;
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use super::docker::{Engine, EngineError};

/// 标准错误只留这么多，写进错误信息够用了。
const STDERR_LIMIT: usize = 4 << 10;

/// 命令的结果：退出码和标准错误。标准输出不看：按凭据助手的约定，结果只看退出码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutput {
    pub exit_code: i64,
    pub stderr: String,
}

impl Engine {
    /// 在容器里执行 `argv`（不经 shell）。`stdin` 写进标准输入后关闭。超过 `timeout` 算失败。
    pub async fn exec(
        &self,
        container: &str,
        argv: &[String],
        stdin: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<ExecOutput, EngineError> {
        let context = format!("在容器 {container} 里执行 {}", argv.join(" "));
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
            .map_err(|e| EngineError::from_bollard(&context, e))?;
        let run = async {
            let started = self
                .docker
                .start_exec(&created.id, None)
                .await
                .map_err(|e| EngineError::from_bollard(&context, e))?;
            let StartExecResults::Attached {
                mut output,
                mut input,
            } = started
            else {
                return Err(EngineError::Rejected {
                    context: context.clone(),
                    message: "命令没有挂上输入输出".to_owned(),
                });
            };
            let io = |e: std::io::Error| EngineError::Unavailable(format!("{context}：{e}"));
            if let Some(data) = stdin {
                input.write_all(data).await.map_err(io)?;
            }
            input.shutdown().await.map_err(io)?;
            let mut stderr = Vec::new();
            while let Some(chunk) = output.next().await {
                if let LogOutput::StdErr { message } =
                    chunk.map_err(|e| EngineError::from_bollard(&context, e))?
                    && stderr.len() < STDERR_LIMIT
                {
                    stderr.extend_from_slice(&message);
                }
            }
            stderr.truncate(STDERR_LIMIT);
            Ok(stderr)
        };
        let stderr = tokio::time::timeout(timeout, run).await.map_err(|_| {
            EngineError::TimedOut(format!("{context}：{} 秒没有结束", timeout.as_secs()))
        })??;
        let inspected = self
            .docker
            .inspect_exec(&created.id)
            .await
            .map_err(|e| EngineError::from_bollard(&context, e))?;
        Ok(ExecOutput {
            exit_code: inspected.exit_code.unwrap_or(-1),
            stderr: String::from_utf8_lossy(&stderr).trim().to_owned(),
        })
    }
}
