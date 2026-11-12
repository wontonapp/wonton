use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use wonton_matrix::{OwnedRoomId, OwnedUserId};

pub const MAX_FRAME_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_queue: usize,
    pub turn_timeout: Duration,
    pub idle_timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub homeserver: String,
    pub access_token: String,
    pub owner: OwnedUserId,
    pub home_room: OwnedRoomId,
    pub matrix_store: PathBuf,
    pub bridge_data: PathBuf,
    pub socket: PathBuf,
    pub workspace: String,
    pub mcp_url: String,
    pub mode: String,
    pub limits: Limits,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bot_data = env_path("WONTON_BOT_DATA", "/data")?;
        let bridge_data = env_path(
            "WONTON_BRIDGE_DATA",
            &bot_data.join("bridge").display().to_string(),
        )?;
        let owner = required("WONTON_MATRIX_OWNER")?
            .parse()
            .context("WONTON_MATRIX_OWNER 不是合法的 Matrix 用户 ID")?;
        let home_room = required("WONTON_MATRIX_HOME_ROOM")?
            .parse()
            .context("WONTON_MATRIX_HOME_ROOM 不是合法的 Matrix 房间 ID")?;
        let mcp_url = required("WONTON_MCP_URL")?;
        if !mcp_url.starts_with("http://") && !mcp_url.starts_with("https://") {
            bail!("WONTON_MCP_URL 必须是 HTTP URL");
        }
        Ok(Self {
            homeserver: required("WONTON_MATRIX_HOMESERVER")?,
            access_token: required("WONTON_MATRIX_ACCESS_TOKEN")?,
            owner,
            home_room,
            matrix_store: bridge_data.join("matrix"),
            bridge_data,
            socket: env_path("WONTON_ACP_SOCKET", "/run/wonton/acp.sock")?,
            workspace: std::env::var("WONTON_AGENT_WORKSPACE")
                .unwrap_or_else(|_| "/workspace".into()),
            mcp_url,
            mode: std::env::var("WONTON_ACP_MODE").unwrap_or_else(|_| "plan".into()),
            limits: Limits {
                max_input_bytes: env_usize("WONTON_ACP_MAX_INPUT_BYTES", 16 * 1024)?,
                max_output_bytes: env_usize("WONTON_ACP_MAX_OUTPUT_BYTES", 64 * 1024)?,
                max_queue: env_usize("WONTON_ACP_MAX_QUEUE", 8)?,
                turn_timeout: Duration::from_secs(env_u64("WONTON_ACP_TURN_SECONDS", 10 * 60)?),
                idle_timeout: Duration::from_secs(env_u64("WONTON_ACP_IDLE_SECONDS", 10 * 60)?),
            },
        })
    }
}

fn required(name: &str) -> Result<String> {
    let value = std::env::var(name).with_context(|| format!("缺少环境变量 {name}"))?;
    if value.trim().is_empty() {
        bail!("环境变量 {name} 不能为空");
    }
    Ok(value)
}

fn env_path(name: &str, default: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var(name).unwrap_or_else(|_| default.into()),
    ))
}

fn env_u64(name: &str, default: u64) -> Result<u64> {
    let value = std::env::var(name).unwrap_or_else(|_| default.to_string());
    value
        .parse()
        .with_context(|| format!("{name} 不是合法的正整数"))
}

fn env_usize(name: &str, default: usize) -> Result<usize> {
    let value = std::env::var(name).unwrap_or_else(|_| default.to_string());
    value
        .parse()
        .with_context(|| format!("{name} 不是合法的正整数"))
}
