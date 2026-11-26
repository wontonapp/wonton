//! 入口错误：对外报出 ADR-0017 的错误类别和给主人看的一句人话。

use std::fmt;

use crate::plugin::docker::EngineError;

#[derive(Debug)]
pub enum EntryError {
    /// 核心跑在宿主上：没有 compose 的 `clients`/`edge`，做不了入口。
    CoreNotInContainer,
    /// 同名容器是别的核心或主人的，标签不是我们的。
    NameTaken(String),
    /// compose 该渲染的网络不在。
    NetworkMissing(String),
    /// 入口容器不唯一，热加载不知道该发给谁。
    NotOneEntry(usize),
    /// 入口热加载失败：新配置被主进程拒绝（带启动日志）或一直没完成。
    Reload(String),
    /// 域名设置读不了：不知道要不要入口，这一轮什么都不删。
    Settings(String),
    Docker(EngineError),
}

impl EntryError {
    /// ADR-0017 的错误类别（C2 定了 `core_not_in_container` 等几条）。
    pub fn category(&self) -> &'static str {
        match self {
            Self::CoreNotInContainer | Self::NetworkMissing(_) => "core_not_in_container",
            Self::NameTaken(_) | Self::NotOneEntry(_) => "conflict",
            Self::Reload(_) => "upstream_error",
            Self::Settings(_) => "invalid_input",
            Self::Docker(EngineError::PortTaken(_)) => "conflict",
            Self::Docker(EngineError::Rejected { .. }) => "invalid_input",
            Self::Docker(EngineError::NotFound(_)) => "not_found",
            Self::Docker(EngineError::Pull { .. }) => "source_unavailable",
            Self::Docker(EngineError::Unavailable(_) | EngineError::TimedOut(_)) => {
                "docker_unavailable"
            }
            Self::Docker(EngineError::Unsupported(_)) => "invalid_input",
        }
    }
}

impl fmt::Display for EntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CoreNotInContainer => write!(
                f,
                "{}：核心不在容器里，没有入口要的 clients、edge 网络",
                self.category()
            ),
            Self::NameTaken(name) => write!(f, "{}：{name} 已被别的资源占用", self.category()),
            Self::NetworkMissing(network) => {
                write!(
                    f,
                    "{}：compose 该渲染的网络 {network} 不在",
                    self.category()
                )
            }
            Self::NotOneEntry(count) => write!(
                f,
                "{}：找到 {count} 个入口容器，热加载不知道发给谁",
                self.category()
            ),
            Self::Reload(logs) => write!(f, "{}：入口热加载失败：{logs}", self.category()),
            Self::Settings(error) => write!(f, "{}：{error}", self.category()),
            Self::Docker(error) => write!(f, "{}：{error}", self.category()),
        }
    }
}

impl std::error::Error for EntryError {}

impl From<EngineError> for EntryError {
    fn from(error: EngineError) -> Self {
        Self::Docker(error)
    }
}

pub(super) fn config_error(context: &str, error: anyhow::Error) -> EntryError {
    EntryError::Docker(EngineError::Rejected {
        context: context.to_owned(),
        message: format!("{error:#}"),
    })
}
