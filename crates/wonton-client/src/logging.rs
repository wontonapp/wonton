use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, Once};

/// 日志文件路径。panic hook 直接往它追加，不经 tracing 的过滤：日志级别开得再低，panic 也留得下。
static LOG_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// 打开核心的日志。整个进程只生效一次，之后的调用不起作用。`filter` 是 tracing 的过滤规则，例如
/// `wonton_tunnel=info,warn`。`file` 为空时写到标准错误（iOS 上 Xcode 和 `devicectl --console` 看得到）；
/// Android 的标准错误不进 logcat，给一个 App 数据目录里的文件，再用 `adb shell run-as` 读。
#[uniffi::export]
pub fn init_logging(filter: String, file: Option<String>) {
    install_panic_hook();
    let filter = tracing_subscriber::EnvFilter::try_new(&filter)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false);
    let opened = file.map(|path| {
        *LOG_FILE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(PathBuf::from(&path));
        OpenOptions::new().create(true).append(true).open(path)
    });
    match opened {
        Some(Ok(file)) => builder.with_writer(Mutex::new(file)).try_init().ok(),
        Some(Err(error)) => {
            eprintln!("could not open the log file; writing to stderr instead: {error}");
            builder.with_writer(std::io::stderr).try_init().ok()
        }
        None => builder.with_writer(std::io::stderr).try_init().ok(),
    };
}

/// 壳自己的 hook 不回传，Android 上标准错误也进不了 logcat：panic 不写进日志就看不见。
/// 只装一次：`init_logging` 可能被调用多次，每次都装会把同一个 panic 写好几遍。
fn install_panic_hook() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| write_panic(info, &*previous)));
    });
}

type PanicHook = dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send;

fn write_panic(info: &std::panic::PanicHookInfo<'_>, previous: &PanicHook) {
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("<unnamed>").to_owned();
    let location = info
        .location()
        .map(|location| location.to_string())
        .unwrap_or_else(|| "<unknown>".to_owned());
    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<not a string>".to_owned());
    let line = format!(
        "PANIC on thread {name} at {location}: {payload}\n{:?}\n",
        std::backtrace::Backtrace::force_capture()
    );
    let path = LOG_FILE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let mut appended = false;
    if let Some(path) = path
        && let Ok(mut file) = OpenOptions::new().append(true).open(path)
    {
        appended = file.write_all(line.as_bytes()).is_ok();
    }
    if !appended {
        tracing::error!("{line}");
    }
    previous(info);
}
