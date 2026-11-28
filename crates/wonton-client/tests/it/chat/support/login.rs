//! 登录令牌：假核心的失败注入和签发次数。

use std::sync::atomic::Ordering;

use super::Home;

impl Home {
    /// 接下来 `count` 次要登录令牌都回 502，隧道照常连着。
    pub fn fail_login_tokens(&self, count: usize) {
        self.core
            .failing_login_tokens
            .store(count, Ordering::SeqCst);
    }

    /// Tuwunel 在重启：核心的网关回 503（Matrix JSON），和 [`Self::set_matrix_down`] 的断连接不同，隧道和令牌照常。
    pub fn set_matrix_unavailable(&self, unavailable: bool) {
        self.core
            .matrix_unavailable
            .store(unavailable, Ordering::SeqCst);
    }

    /// 核心签发过几次登录令牌。
    pub fn login_tokens(&self) -> usize {
        self.core.login_tokens.load(Ordering::SeqCst)
    }
}
