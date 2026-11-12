//! 容器日志的读取：安装失败时给出原因，一次读的上限在 [`LOGS_MAX_BYTES`]。
//!
//! 失败现场也要进核心日志：安装失败后容器可能被回滚删掉，光有错误正文的调用方不至于连日志都没了（见
//! [`super::health`]）。

use bollard::query_parameters::LogsOptionsBuilder;
use futures_util::StreamExt;

use super::docker::Engine;

/// 一次读日志返回的上限（字节）。
const LOGS_MAX_BYTES: usize = 64 << 10;

/// 留下结尾 `limit` 字节，从字符边界切开。
fn keep_tail(text: &str, limit: usize) -> String {
    let mut start = text.len().saturating_sub(limit);
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_owned()
}

impl Engine {
    /// 容器最后若干行日志，用于在安装失败时给出原因。返回的正文最多 `LOGS_MAX_BYTES` 字节，超出时丢开头的
    /// 部分、留下结尾：原因通常在最后几行，一次拉起几十兆的日志也不该撑爆错误。
    pub async fn logs_tail(&self, name: &str, lines: usize) -> String {
        let options = LogsOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .tail(&lines.to_string())
            .build();
        let mut stream = self.docker.logs(name, Some(options));
        let mut out = String::new();
        while let Some(Ok(chunk)) = stream.next().await {
            out.push_str(&chunk.to_string());
            // 边读边丢开头，免得一次拉起几十兆：多留一倍，分块边界落在字符中间也不出错。
            if out.len() > LOGS_MAX_BYTES * 2 {
                out = keep_tail(&out, LOGS_MAX_BYTES);
            }
        }
        keep_tail(&out, LOGS_MAX_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::keep_tail;

    #[test]
    fn tails_keep_the_end_on_a_character_boundary() {
        assert_eq!(keep_tail("abcdef", 3), "def");
        assert_eq!(keep_tail("ab", 10), "ab");
        // 「原」是 3 字节：切在中间时往后挪，不出现半截字符。
        let text = format!("{}原", "x".repeat(5));
        assert_eq!(keep_tail(&text, 4), "x原");
        assert_eq!(keep_tail(&text, 3), "原");
    }
}
