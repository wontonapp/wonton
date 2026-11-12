//! 回声 Bot：最小的 Matrix Bot 示例插件（`plugins/examples/echo-bot`）。核心安装插件时代 Bot 登录、建好和主人的
//! 加密私聊，经 `WONTON_MATRIX_*` 把身份交给它；它把主人发来的文字原样回过去，加密房间里也一样。
//!
//! - `echo-bot run`（默认）：按 `WONTON_MATRIX_*` 恢复会话，同步并回声。加密存储在 `ECHO_STORE`（默认 `/data`）下，
//!   每台设备一个子目录：重装插件会换一台设备，旧设备的密钥不能拿来用。
//! - `echo-bot health`：最近 90 秒内同步成功过才算健康，给容器的健康检查用。
//! - `echo-bot probe …`：冒烟测试里扮演主人的一台设备，经手机的本地代理发一条加密消息，等 Bot 的加密回声。

mod bot;
mod probe;

use std::process::ExitCode;

use tracing_subscriber::EnvFilter;

/// 回声的前缀：Bot 回 `回声：<原文>`。
pub const PREFIX: &str = "回声：";

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                // 没配密钥备份时 matrix-sdk 每发一条消息都要警告一次，压掉。
                .unwrap_or_else(|_| {
                    EnvFilter::new("info,matrix_sdk=warn,matrix_sdk_crypto::backups=error")
                }),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None | Some("run") => bot::run().await,
        Some("health") => bot::health(),
        Some("probe") => probe::run(&args[1..]).await,
        Some(other) => Err(anyhow::anyhow!(
            "未知命令 {other}；用法：echo-bot [run|health|probe]"
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}
