//! 壳帮核心打开上传来源：Rust 打不开 Android 的内容 URI，也解析不了 iOS 的书签。

/// 壳打开来源失败的原因，传输按它落到本机的失败类别上。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FileOpenError {
    /// 文件被删了、内容 URI 或书签失效了：[`FileFailure::LocalSourceUnavailable`](wonton_protocol::FileFailure)。
    #[error("{reason}")]
    Unavailable { reason: String },
    /// 系统不给读（Android 的 `SecurityException`）：[`FileFailure::LocalPermissionDenied`](wonton_protocol::FileFailure)。
    #[error("{reason}")]
    PermissionDenied { reason: String },
    /// 壳的回调抛了没声明的异常：壳的 bug，重新选文件也没用。[`FileFailure::LocalIo`](wonton_protocol::FileFailure)。
    #[error("{reason}")]
    Unexpected { reason: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for FileOpenError {
    fn from(err: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Unexpected { reason: err.reason }
    }
}

/// 壳实现，经 [`ClientCore::set_file_opener`](crate::ClientCore::set_file_opener) 交给核心。
#[uniffi::export(with_foreign)]
pub trait FileOpener: Send + Sync {
    /// 打开 [`UploadSource::Opaque`](wonton_protocol::UploadSource::Opaque) 的 `handle`，返回只读、能定位的文件描述符，
    /// 所有权交给核心，由核心关闭（Android 用 `detachFd`）。App 重启后接着传时会用同一个 `handle` 再调一次。在后台线程上
    /// 调用，可以阻塞。不叫 `open`：它是 Swift 的关键字，UniFFI 会把反引号写进 C 头文件，编译不过。
    fn open_source(&self, handle: String) -> Result<i32, FileOpenError>;
}
