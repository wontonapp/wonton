use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use wonton_matrix::{OwnedRoomId, OwnedUserId};

pub const MAX_FRAME_BYTES: usize = 512 * 1024;

/// 桥会发的工作模式全集（负责人的定）：写死已知安全的值，挡住 Claude Code 的
/// `bypassPermissions`、Codex 的 `full-access` 这类不问就做的档位；`auto` 在 Codex
/// 的沙箱里写文件不问，按主人的决定保留。Agent 在 `availableModes` 里报出别的取值
/// 也不跟。
pub const ALLOWED_MODES: &[&str] = &["default", "plan", "acceptEdits", "read-only", "auto"];

fn checked_mode(name: &str, value: String) -> Result<String> {
    if ALLOWED_MODES.contains(&value.as_str()) {
        Ok(value)
    } else {
        bail!("{name} 配的 {value:?} 不在桥的模式白名单 {ALLOWED_MODES:?} 里")
    }
}

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_queue: usize,
    pub turn_timeout: Duration,
    pub idle_timeout: Duration,
    /// 空闲定时器的周期。
    pub idle_tick: Duration,
    /// 话题空闲多久判结束（ADR-0058：30 分钟）。
    pub topic_idle: Duration,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub homeserver: String,
    pub access_token: String,
    pub allowed_users: Vec<OwnedUserId>,
    /// 主房间的主人，即私聊的对方：`WONTON_MATRIX_ALLOWED_USERS` 的第一个（私人 Bot 就是归属人）。
    pub owner: OwnedUserId,
    pub home_room: OwnedRoomId,
    pub matrix_store: PathBuf,
    pub bridge_data: PathBuf,
    pub socket: PathBuf,
    pub workspace: String,
    pub mcp_url: String,
    pub mode: String,
    /// 话题会话想切的“改前先问”模式（ADR-0058）；Agent 报不出这个取值时退回 `mode`。
    pub topic_mode: Option<String>,
    pub limits: Limits,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bot_data = env_path("WONTON_BOT_DATA", "/data")?;
        let bridge_data = env_path(
            "WONTON_BRIDGE_DATA",
            &bot_data.join("bridge").display().to_string(),
        )?;
        // Bot 只听名单里的人（ADR-0045）：私人 Bot 是归属人，共享 Bot 是一家人。
        let allowed_users = required("WONTON_MATRIX_ALLOWED_USERS")?
            .split(',')
            .filter(|id| !id.is_empty())
            .map(|id| {
                id.parse()
                    .context("WONTON_MATRIX_ALLOWED_USERS 里有不合法的 Matrix 用户 ID")
            })
            .collect::<Result<Vec<OwnedUserId>, _>>()?;
        let home_room = required("WONTON_MATRIX_HOME_ROOM")?
            .parse()
            .context("WONTON_MATRIX_HOME_ROOM 不是合法的 Matrix 房间 ID")?;
        let mcp_url = required("WONTON_MCP_URL")?;
        if !mcp_url.starts_with("http://") && !mcp_url.starts_with("https://") {
            bail!("WONTON_MCP_URL 必须是 HTTP URL");
        }
        let owner = allowed_users
            .first()
            .context("WONTON_MATRIX_ALLOWED_USERS 是空的，不知道主房间归谁")?
            .clone();
        Ok(Self {
            homeserver: required("WONTON_MATRIX_HOMESERVER")?,
            access_token: required("WONTON_MATRIX_ACCESS_TOKEN")?,
            allowed_users,
            owner,
            home_room,
            matrix_store: bridge_data.join("matrix"),
            bridge_data,
            socket: env_path("WONTON_ACP_SOCKET", "/run/wonton/acp.sock")?,
            workspace: std::env::var("WONTON_AGENT_WORKSPACE")
                .unwrap_or_else(|_| "/workspace".into()),
            mcp_url,
            mode: checked_mode(
                "WONTON_ACP_MODE",
                std::env::var("WONTON_ACP_MODE").unwrap_or_else(|_| "plan".into()),
            )?,
            topic_mode: std::env::var("WONTON_ACP_TOPIC_MODE")
                .ok()
                .filter(|mode| !mode.trim().is_empty())
                .map(|mode| checked_mode("WONTON_ACP_TOPIC_MODE", mode))
                .transpose()?,
            limits: Limits {
                max_input_bytes: env_usize("WONTON_ACP_MAX_INPUT_BYTES", 16 * 1024)?,
                max_output_bytes: env_usize("WONTON_ACP_MAX_OUTPUT_BYTES", 64 * 1024)?,
                max_queue: env_usize("WONTON_ACP_MAX_QUEUE", 8)?,
                turn_timeout: Duration::from_secs(env_u64("WONTON_ACP_TURN_SECONDS", 10 * 60)?),
                idle_timeout: Duration::from_secs(env_u64("WONTON_ACP_IDLE_SECONDS", 10 * 60)?),
                idle_tick: Duration::from_secs(env_u64("WONTON_ACP_IDLE_TICK_SECONDS", 30)?),
                topic_idle: Duration::from_secs(env_u64("WONTON_ACP_TOPIC_IDLE_SECONDS", 30 * 60)?),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_modes_pass_the_whitelist() {
        for mode in ALLOWED_MODES {
            assert_eq!(
                checked_mode("WONTON_ACP_MODE", (*mode).into()).unwrap(),
                *mode
            );
        }
    }

    #[test]
    fn unsafe_modes_fail_startup_with_the_reason() {
        for mode in ["bypassPermissions", "full-access", "yolo"] {
            let error = checked_mode("WONTON_ACP_MODE", mode.into())
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(mode) && error.contains("白名单"),
                "启动错误要说清原因：{error}"
            );
        }
    }
}
