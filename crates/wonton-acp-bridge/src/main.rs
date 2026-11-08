#![recursion_limit = "256"]
//! Wonton ACP bridge: Matrix 普通文本与 ACP session 之间的薄适配层。

mod acp;
mod bridge;
mod commands;
mod config;
mod error;
mod l10n;
mod locale;
mod login;
mod matrix;
mod permission;
mod room;
mod room_owner;
mod state;
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
