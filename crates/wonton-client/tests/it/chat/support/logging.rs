//! 测试的日志订阅：`WONTON_TEST_LOG` 有值时按它过滤核心日志。

use tracing_subscriber::Layer as _;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// `WONTON_TEST_LOG` 有值时打开核心的日志（值是过滤规则，例如 `wonton_tunnel=debug,matrix_sdk=debug`），
/// 写到标准错误；失败的时候按时间顺序留下当时发生了什么。
///
/// 不论开不开日志都装一层捕获：时间线重建的原因按房间记下来（见 [`take_rebuild_reason`]），断言失败时带进
/// panic 消息，否则只看到“被清空了”、不知道是哪条路。
pub fn init_test_logging() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let subscriber =
            tracing_subscriber::registry().with(super::super::rebuild_reason::Capture.with_filter(
                filter_fn(|m| super::super::rebuild_reason::TARGETS.contains(&m.target())),
            ));
        match std::env::var("WONTON_TEST_LOG").or_else(|_| Ok::<_, ()>("info".to_owned())) {
            Ok(filter) => {
                let filter = tracing_subscriber::EnvFilter::try_new(&filter)
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
                subscriber
                    .with(
                        tracing_subscriber::fmt::layer()
                            .with_ansi(false)
                            .with_writer(std::io::stderr)
                            .with_filter(filter),
                    )
                    .try_init()
                    .ok();
            }
            Err(_) => {
                subscriber.try_init().ok();
            }
        };
    });
}
