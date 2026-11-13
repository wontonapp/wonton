use std::{fmt, io, num::TryFromIntError};

use thiserror::Error;
use wonton_l10n::{Arg, Lang};
use wonton_matrix::Error as MatrixError;

use crate::l10n::{log_lang, text};

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// 错误里的详情：桥自己写的整句（Fluent 消息，可带参数），或原样转给用户的上游数据
/// （Agent 原文、IO 错误、HTTP 状态）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Detail {
    Msg {
        id: &'static str,
        args: Vec<(&'static str, String)>,
    },
    Data(String),
}

impl Detail {
    pub fn msg(id: &'static str) -> Self {
        Self::Msg {
            id,
            args: Vec::new(),
        }
    }

    pub fn msg_with(
        id: &'static str,
        args: impl IntoIterator<Item = (&'static str, String)>,
    ) -> Self {
        Self::Msg {
            id,
            args: args.into_iter().collect(),
        }
    }

    pub fn data(value: impl Into<String>) -> Self {
        Self::Data(value.into())
    }

    /// 按给定语言写出来；数据原样。
    pub fn format(&self, lang: Lang) -> String {
        match self {
            Detail::Msg { id, args } => {
                let args: Vec<(&str, Arg<'_>)> = args
                    .iter()
                    .map(|(name, value)| (*name, Arg::from(value.clone())))
                    .collect();
                text(lang, id, &args)
            }
            Detail::Data(value) => value.clone(),
        }
    }
}

/// 给日志看的中文（日志不跟房间语言走）。
impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format(log_lang()))
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Agent 需要登录；发送 /wonton login 查看登录方式")]
    AuthenticationRequired,
    #[error("Agent 未就绪：{0}")]
    AgentNotReady(Detail),
    #[error("ACP 传输不可用：{0}")]
    TransportUnavailable(Detail),
    #[error("ACP 会话不可用：{0}")]
    SessionUnavailable(Detail),
    #[error("ACP 上游出错：{0}")]
    Upstream(Detail),
    #[error("输入非法：{0}")]
    InvalidInput(Detail),
    #[error("确认卡片不可用：{0}")]
    ConfirmationUnavailable(Detail),
    /// 核心的调用链接口拒了这一轮（ADR-0061 第 10 节）：上限和已停有各自的说明，
    /// 这里是其余类别（权限、输入非法、未知），详情是核心写的。
    #[error("调用链拒绝了这一轮：{0}")]
    ChainRefused(Detail),
    #[error("队列已满：{0}")]
    Busy(Detail),
    #[error("Matrix 操作失败：{0}")]
    Matrix(Detail),
    /// 几个 Bot 在房间里来回太多，网关停下了本 Bot 的 `count` 条消息（ADR-0045 的跳数）。
    #[error("跳数到顶，网关停下了 {count} 条消息")]
    HopLimit { count: usize },
}

/// 聊天里显示的一条错误：类别前缀在前，详情按语言写或原样嵌进句子（ADR-0043）。
pub fn format_error(error: &Error, lang: Lang) -> String {
    let (template, detail) = match error {
        Error::AuthenticationRequired => return text(lang, "err-auth-required", &[]),
        Error::HopLimit { count } => {
            return text(lang, "err-hop-limit", &[("count", Arg::from(*count))]);
        }
        Error::AgentNotReady(detail) => ("err-not-ready", detail),
        Error::TransportUnavailable(detail) => ("err-transport", detail),
        Error::SessionUnavailable(detail) => ("err-session", detail),
        Error::Upstream(detail) => ("err-upstream", detail),
        Error::InvalidInput(detail) => ("err-invalid", detail),
        Error::ConfirmationUnavailable(detail) => ("err-confirmation", detail),
        Error::ChainRefused(detail) => ("err-chain-refused", detail),
        Error::Busy(detail) => ("err-busy", detail),
        Error::Matrix(detail) => ("err-matrix", detail),
    };
    let message = detail.format(lang);
    text(lang, template, &[("message", Arg::from(message))])
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::TransportUnavailable(Detail::data(error.to_string()))
    }
}

impl From<TryFromIntError> for Error {
    fn from(error: TryFromIntError) -> Self {
        Self::InvalidInput(Detail::data(error.to_string()))
    }
}

impl From<MatrixError> for Error {
    fn from(error: MatrixError) -> Self {
        match error {
            MatrixError::Transport(message) => Self::TransportUnavailable(Detail::data(message)),
            MatrixError::InvalidInput(message) => Self::InvalidInput(Detail::data(message)),
            MatrixError::Unauthorized { message, .. } | MatrixError::Forbidden { message, .. } => {
                Self::Matrix(Detail::data(message))
            }
            MatrixError::NotReady(message) | MatrixError::Upstream(message) => {
                Self::Matrix(Detail::data(message))
            }
        }
    }
}
