//! 一条消息回什么、什么时候回：普通文字立刻回；`/delay <秒> <文字>` 到点再回。延迟回的消息只在内存里
//! 计时，Bot 重启后没到点的不补发。

use std::time::Duration;

use tokio::time::sleep;

/// 要回的文字和时机。
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    /// 立刻回。
    Now(String),
    /// 过这么久再回。
    After(Duration, String),
}

/// 解析一条消息：整条是 `/delay <秒> <文字>` 才延迟回声，其余原样立刻回。秒数不是整数或没跟文字时
/// 当普通文字回，打错一个字也还能听到回声。
pub fn parse(body: &str) -> Reply {
    match delay(body.trim()) {
        Some(reply) => reply,
        None => Reply::Now(body.to_owned()),
    }
}

fn delay(body: &str) -> Option<Reply> {
    let rest = body.strip_prefix("/delay")?;
    // `/delayfoo` 不是命令，后面要紧跟空白。
    let rest = rest.strip_prefix(char::is_whitespace)?;
    let (seconds, text) = rest.trim_start().split_once(char::is_whitespace)?;
    let text = text.trim();
    let seconds = seconds.parse().ok()?;
    (!text.is_empty()).then(|| Reply::After(Duration::from_secs(seconds), text.to_owned()))
}

/// 按时机把回声交给 `send` 发出去：延迟的放进后台任务等，不挡同步。`send` 自己处理发送失败。
pub async fn deliver<F, Fut>(reply: Reply, send: F)
where
    F: Fn(String) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    match reply {
        Reply::Now(text) => send(text).await,
        Reply::After(delay, text) => {
            tokio::spawn(async move {
                sleep(delay).await;
                send(text).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

    fn channel() -> (UnboundedSender<String>, UnboundedReceiver<String>) {
        unbounded_channel()
    }

    #[test]
    fn 解析延迟命令() {
        assert_eq!(
            parse("/delay 310  到 点 了 "),
            Reply::After(Duration::from_secs(310), "到 点 了".to_owned())
        );
        assert_eq!(
            parse("/delay\t0 hi"),
            Reply::After(Duration::ZERO, "hi".to_owned())
        );
    }

    #[test]
    fn 不像命令的原样回() {
        for body in [
            "/delay",
            "/delay 310",
            "/delay 310 ",
            "/delay abc hi",
            "/delayfoo 1 hi",
        ] {
            assert_eq!(parse(body), Reply::Now(body.to_owned()), "{body}");
        }
    }

    #[tokio::test]
    async fn 普通文字立刻回() {
        let (tx, mut rx) = channel();
        deliver(parse("你好"), move |text| {
            let tx = tx.clone();
            async move { tx.send(text).unwrap() }
        })
        .await;
        assert_eq!(rx.try_recv().unwrap(), "你好");
        assert!(rx.try_recv().is_err(), "只该回一条");
    }

    #[tokio::test(start_paused = true)]
    async fn 到点才回() {
        let (tx, mut rx) = channel();
        deliver(parse("/delay 310 到点了"), move |text| {
            let tx = tx.clone();
            async move { tx.send(text).unwrap() }
        })
        .await;
        tokio::task::yield_now().await;
        assert!(rx.try_recv().is_err(), "还没到点就回了");
        tokio::time::advance(Duration::from_secs(309)).await;
        tokio::task::yield_now().await;
        assert!(rx.try_recv().is_err(), "309 秒就回了");
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(rx.try_recv().unwrap(), "到点了");
        assert!(rx.try_recv().is_err(), "只该回一条");
    }
}
