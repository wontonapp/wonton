use std::fs::OpenOptions;
use std::sync::Mutex;

/// 打开核心的日志。整个进程只生效一次，之后的调用不起作用。`filter` 是 tracing 的过滤规则，例如
/// `wonton_tunnel=info,warn`。`file` 为空时写到标准错误（iOS 上 Xcode 和 `devicectl --console` 看得到）；
/// Android 的标准错误不进 logcat，给一个 App 数据目录里的文件，再用 `adb shell run-as` 读。
#[uniffi::export]
pub fn init_logging(filter: String, file: Option<String>) {
    let filter = tracing_subscriber::EnvFilter::try_new(&filter)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false);
    let opened = file.map(|path| OpenOptions::new().create(true).append(true).open(path));
    match opened {
        Some(Ok(file)) => builder.with_writer(Mutex::new(file)).try_init().ok(),
        Some(Err(error)) => {
            eprintln!("打不开日志文件，改写标准错误：{error}");
            builder.with_writer(std::io::stderr).try_init().ok()
        }
        None => builder.with_writer(std::io::stderr).try_init().ok(),
    };
}
