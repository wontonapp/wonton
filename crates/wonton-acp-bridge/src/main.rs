#![recursion_limit = "256"]
//! Wonton ACP bridge: Matrix 普通文本与 ACP session 之间的薄适配层。

mod acp;
mod bridge;
mod commands;
mod config;
mod error;
mod health;
mod l10n;
mod locale;
mod login;
mod matrix;
mod permission;
mod room;
mod room_owner;
mod state;
mod topics;
mod undecryptable;

use std::process::ExitCode;

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,wonton_matrix=warn,matrix_sdk=warn")),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("health") {
        // 健康检查的原因要进探针的 stdout：Docker 收探针输出，核心把它原样放进插件的原因里。
        return match health::Files::from_env().run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                println!("{error:#}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some(other) = args.first() {
        eprintln!("未知命令 {other}；用法：wonton-acp-bridge [run|health]");
        return ExitCode::FAILURE;
    }
    match bridge::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = ?error, "ACP bridge 退出");
            ExitCode::FAILURE
        }
    }
}
