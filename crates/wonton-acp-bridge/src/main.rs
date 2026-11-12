#![recursion_limit = "256"]
//! Wonton ACP bridge: Matrix 普通文本与 ACP session 之间的薄适配层。

mod acp;
mod bridge;
mod commands;
mod config;
mod error;
mod login;
mod matrix;
mod permission;
mod state;
// ADR-0058 第 1 步：模块先入库、只随测试编译；第 2 步接线时改为普通 mod。
#[cfg(test)]
mod topics;

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
    match bridge::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = ?error, "ACP bridge 退出");
            ExitCode::FAILURE
        }
    }
}
