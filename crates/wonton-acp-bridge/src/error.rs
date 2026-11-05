use std::{io, num::TryFromIntError};

use thiserror::Error;
use wonton_matrix::Error as MatrixError;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Agent 需要登录；发送 /wonton login 查看登录方式")]
    AuthenticationRequired,
    #[error("Agent 未就绪：{0}")]
    AgentNotReady(String),
    #[error("ACP 传输不可用：{0}")]
    TransportUnavailable(String),
    #[error("ACP 会话不可用：{0}")]
    SessionUnavailable(String),
    #[error("ACP 上游出错：{0}")]
    Upstream(String),
    #[error("输入非法：{0}")]
    InvalidInput(String),
    #[error("确认卡片不可用：{0}")]
    ConfirmationUnavailable(String),
    #[error("队列已满：{0}")]
    Busy(String),
    #[error("Matrix 操作失败：{0}")]
    Matrix(String),
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::TransportUnavailable(error.to_string())
    }
}

impl From<TryFromIntError> for Error {
    fn from(error: TryFromIntError) -> Self {
        Self::InvalidInput(error.to_string())
    }
}

impl From<MatrixError> for Error {
    fn from(error: MatrixError) -> Self {
        match error {
            MatrixError::Transport(message) => Self::TransportUnavailable(message),
            MatrixError::InvalidInput(message) => Self::InvalidInput(message),
            MatrixError::Unauthorized { message, .. } => Self::Matrix(message),
            MatrixError::NotReady(message) | MatrixError::Upstream(message) => {
                Self::Matrix(message)
            }
        }
    }
}
