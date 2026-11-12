//! HTTPS 证书校验器的就绪状态。
//!
//! matrix-sdk 和 reqwest 用 rustls-platform-verifier 按系统证书库校验证书。它在 Android 上要先经 JNI 拿到 JavaVM 和
//! Application（App 启动时由客户端核心初始化，见 `wonton-client` 的 `src/android.rs`）；没初始化就校验证书，0.7 版直接
//! panic，请求所在的任务死掉，调用方一直等不到结果。所以连 HTTPS 之前先查这里，没就绪就按传输不可用报错。
//! 其他平台的校验器不用初始化，总是就绪。

use std::sync::OnceLock;

static INIT: OnceLock<Result<(), String>> = OnceLock::new();

/// 记下 Android 上初始化证书校验器的结果；只认第一次。
pub fn record_init(result: Result<(), String>) {
    let _ = INIT.set(result);
}

/// 能不能校验 HTTPS 证书。不能时给出原因（英文，进错误的“详情”）。
pub fn check() -> Result<(), String> {
    readiness(cfg!(target_os = "android"), INIT.get())
}

/// 连 `url` 之前查：明文 HTTP（比如经本地代理连自己的服务器）不校验证书，不受影响。
pub(crate) fn check_url(url: &str) -> Result<(), String> {
    if url.starts_with("http://") {
        Ok(())
    } else {
        check()
    }
}

fn readiness(required: bool, init: Option<&Result<(), String>>) -> Result<(), String> {
    if !required {
        return Ok(());
    }
    match init {
        Some(Ok(())) => Ok(()),
        Some(Err(reason)) => Err(format!(
            "the system certificate verifier failed to initialize: {reason}"
        )),
        None => Err("the system certificate verifier is not initialized".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::readiness;

    #[test]
    fn platforms_without_initialization_are_always_ready() {
        assert_eq!(readiness(false, None), Ok(()));
    }

    #[test]
    fn android_refuses_https_until_initialized() {
        assert!(readiness(true, None).is_err());
        assert!(readiness(true, Some(&Err("no class loader".to_owned()))).is_err());
        assert_eq!(readiness(true, Some(&Ok(()))), Ok(()));
    }
}
