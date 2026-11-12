//! 在子进程里处理不可信的图片和 PDF（[`super::images`]、[`super::pdf`]）：`wonton-core image
//! <icon.png|icon.svg|screenshot|thumbnail|preview|pdf>` 从标准输入读原文件，把结果写到标准输出。文件是不可信的输入，
//! 预检挡不住所有费时费内存的写法（比如很细的虚线），所以解码和渲染放进单独的进程：它先给自己设内存和 CPU 时间的上限，
//! 到时没做完由核心杀掉。超时、内存不够、崩溃都算这个文件处理不了。
//!
//! 这些子进程不直接从核心起，而是从一个常驻的助手进程（`wonton-core image-host`）起，见 [`host`]：核心
//! 起子进程时，fork 到 exec 之间子进程手里有隧道 UDP 套接字的副本，正好撞上 iroh 在原端口重绑就
//! `AddrInUse`，之后不会自己恢复。助手在核心绑隧道之前就起好，图片任务的 fork 全在它里面。
//!
//! 子进程只限资源，不是沙箱：它和核心是同一个用户，文件和网络的权限也一样。

use std::io::{Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::{ExitCode, ExitStatus};
use std::time::Duration;

use rustix::process::{Resource, Rlimit, Signal, getrlimit, setrlimit};
use tokio::io::{AsyncRead, AsyncReadExt};

use super::images::{self, Icon, IconFormat};
use super::languages::Languages;

mod client;
mod host;
mod protocol;
pub use client::ImageWorker;
pub use host::host;

/// 子命令（`wonton-core image …`），核心内部用。
pub const COMMAND: &str = "image";
/// 常驻图片助手的子命令（`wonton-core image-host <程序>`），核心内部用。
pub const HOST_COMMAND: &str = protocol::HOST_COMMAND;
/// 核心等助手起来最多这么久。
const READY_TIMEOUT: Duration = Duration::from_secs(2);
/// 核心里比子进程的时限多等这么久：助手到点会杀掉子进程并回答，超过这个数是助手本身出了毛病。
const HELPER_GRACE: Duration = Duration::from_secs(2);
/// 子进程的地址空间上限。上限内最大的插件图片（4096×4096 的 16 位 PNG 截图）处理时要两三百 MB；再大的照片做不了缩略图。
const MEMORY_LIMIT: u64 = 512 << 20;
/// 子进程的 CPU 时间上限。核心按墙钟时间先杀掉它；核心自己没了时由这个兜底。
const CPU_SECONDS: u64 = 10;
/// 核心等子进程最多这么久，到了就杀掉。
const TIMEOUT: Duration = Duration::from_secs(10);
/// 同时最多这么多个子进程。
const MAX_RUNNING: usize = 2;
/// 子进程的输出最多这么多：两张 PNG 图标或一张 JPEG 截图，远到不了。
const MAX_OUTPUT: usize = 8 << 20;
/// 标准错误留这么多，用来报原因。
const MAX_STDERR: usize = 4 << 10;
/// 子进程的退出码：图片不合规格，原因在标准错误。
const REFUSED: u8 = 1;
/// 子进程的退出码：用法不对、设不了资源上限、读写不了标准输入输出，和图片无关。
const BROKEN: u8 = 2;
/// 助手回答里的结果码：到时限没做完（子进程没有这个退出码）。
const TIMED_OUT: u8 = 3;

/// 文件处理不了的原因。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImageError {
    /// 文件本身的问题：不合规格，或者处理时内存不够、崩溃。同一份文件再处理一遍还是这样。
    #[error("{0}")]
    Refused(String),
    /// 到时限还没处理完：机器忙，同一份文件过一会儿可能就好了。
    #[error("{0}")]
    TimedOut(String),
    /// 核心这边起不了或用不了处理文件的进程，和文件本身无关。
    #[error("{0}")]
    Unavailable(String),
}

/// 子进程做的事，也是它的命令行参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    Icon(IconFormat),
    TabIcon(IconFormat),
    Screenshot,
    Thumbnail,
    Preview,
    Pdf,
}

impl Job {
    fn arg(self) -> &'static str {
        match self {
            Self::Icon(IconFormat::Png) => "icon.png",
            Self::Icon(IconFormat::Svg) => "icon.svg",
            Self::TabIcon(IconFormat::Png) => "tab-icon.png",
            Self::TabIcon(IconFormat::Svg) => "tab-icon.svg",
            Self::Screenshot => "screenshot",
            Self::Thumbnail => "thumbnail",
            Self::Preview => "preview",
            Self::Pdf => "pdf",
        }
    }

    /// 最多从标准输入读这么多。多读一个字节：超了由各自的大小检查报出来。
    fn input_limit(self) -> u64 {
        let limit = match self {
            Self::Thumbnail | Self::Preview | Self::Pdf => images::MAX_THUMBNAIL_INPUT,
            Self::Icon(_) | Self::TabIcon(_) | Self::Screenshot => images::MAX_FILE_BYTES,
        };
        limit as u64 + 1
    }

    fn parse(arg: &str) -> Option<Self> {
        [
            Self::Icon(IconFormat::Png),
            Self::Icon(IconFormat::Svg),
            Self::TabIcon(IconFormat::Png),
            Self::TabIcon(IconFormat::Svg),
            Self::Screenshot,
            Self::Thumbnail,
            Self::Preview,
            Self::Pdf,
        ]
        .into_iter()
        .find(|job| job.arg() == arg)
    }

    fn run(self, input: &[u8], langs: &Languages) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        match self {
            Self::Icon(format) => put_icon(&mut out, images::icon(format, input, langs)?),
            Self::TabIcon(format) => put_icon(&mut out, images::tab_icon(format, input, langs)?),
            Self::Screenshot => {
                let shot = images::screenshot(input, langs)?;
                put(&mut out, &shot.jpeg);
                put(&mut out, &shot.width.to_le_bytes());
                put(&mut out, &shot.height.to_le_bytes());
            }
            Self::Thumbnail => put(&mut out, &images::thumbnail(input, langs)?),
            Self::Preview => put(&mut out, &images::preview(input, langs)?),
            Self::Pdf => {
                let extracted = super::pdf::extract(input, langs)?;
                put(&mut out, extracted.text.as_bytes());
                put(&mut out, &extracted.pages_read.to_le_bytes());
                put(&mut out, &extracted.pages_total.to_le_bytes());
                put(&mut out, &[u8::from(extracted.truncated)]);
            }
        }
        Ok(out)
    }
}

fn put_icon(out: &mut Vec<u8>, icon: Icon) {
    put(out, &icon.small);
    put(out, &icon.large);
    put(out, &[u8::from(icon.transparent_corner)]);
}

/// 子进程的入口。退出码 0：处理好了，结果在标准输出；[`REFUSED`]：图片不合规格；[`BROKEN`]：别的问题。
pub fn child(args: &[String]) -> ExitCode {
    // 语言来自父进程的环境变量（见 [`ImageWorker::spawn`]）。
    let langs = Languages::from_header(std::env::var("WONTON_LANG").ok().as_deref());
    let job = match args {
        [arg] => Job::parse(arg),
        _ => None,
    };
    let Some(job) = job else {
        eprintln!(
            "{}",
            crate::l10n::text(
                &langs,
                "plugin-install-img-worker-usage",
                &[("command", COMMAND.into())],
            )
        );
        return ExitCode::from(BROKEN);
    };
    if let Err(e) = limit_resources() {
        eprintln!(
            "{}",
            crate::l10n::text(
                &langs,
                "plugin-install-img-worker-rlimit",
                &[("error", e.to_string().into())],
            )
        );
        return ExitCode::from(BROKEN);
    }
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin()
        .take(job.input_limit())
        .read_to_end(&mut input)
    {
        eprintln!(
            "{}",
            crate::l10n::text(
                &langs,
                "plugin-install-img-worker-stdin",
                &[("error", e.to_string().into())],
            )
        );
        return ExitCode::from(BROKEN);
    }
    let output = match job.run(&input, &langs) {
        Ok(output) => output,
        Err(reason) => {
            eprintln!("{reason}");
            return ExitCode::from(REFUSED);
        }
    };
    let mut stdout = std::io::stdout().lock();
    match stdout.write_all(&output).and_then(|()| stdout.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!(
                "{}",
                crate::l10n::text(
                    &langs,
                    "plugin-install-img-worker-stdout",
                    &[("error", e.to_string().into())],
                )
            );
            ExitCode::from(BROKEN)
        }
    }
}

/// 只降不升：核心自己的硬上限更低时（比如容器设了 ulimit）用那个。CPU 时间的硬上限多一秒：到软上限时进程收到
/// SIGXCPU，核心据此说清原因；软硬上限相等时内核直接发 SIGKILL。不转储内存：几百 MB 的内存不该写到磁盘上，而
/// core_pattern 是管道时（如 systemd-coredump）内核不看 RLIMIT_CORE，照样叫起转储程序，所以 Linux 上还把进程设成不可转储。
fn limit_resources() -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)?;
    let limits = [
        (Resource::As, MEMORY_LIMIT, MEMORY_LIMIT),
        (Resource::Cpu, CPU_SECONDS, CPU_SECONDS + 1),
        (Resource::Core, 0, 0),
    ];
    for (resource, soft, hard) in limits {
        let ceiling = getrlimit(resource).maximum;
        let lower = |value: u64| Some(ceiling.map_or(value, |max| max.min(value)));
        setrlimit(
            resource,
            Rlimit {
                current: lower(soft),
                maximum: lower(hard),
            },
        )?;
    }
    Ok(())
}

fn parse_icon(output: &[u8], langs: &Languages) -> Result<Icon, ImageError> {
    let mut fields = Fields(output);
    Ok(Icon {
        small: fields.bytes(langs)?.to_vec(),
        large: fields.bytes(langs)?.to_vec(),
        transparent_corner: fields.bytes(langs)? == [1],
    })
}

/// 读到上限为止，多出来的读掉丢弃：不读的话子进程写满管道就卡住了。
async fn read_capped(
    mut from: impl AsyncRead + Unpin,
    cap: usize,
    into: &mut Vec<u8>,
) -> std::io::Result<()> {
    (&mut from).take(cap as u64).read_to_end(into).await?;
    tokio::io::copy(&mut from, &mut tokio::io::sink()).await?;
    Ok(())
}

/// 子进程的退出状态对应的结果。
fn outcome(
    status: ExitStatus,
    output: Vec<u8>,
    stderr: &str,
    langs: &Languages,
) -> Result<Vec<u8>, ImageError> {
    let stderr = stderr.trim();
    if let Some(signal) = status.signal() {
        let message = killed_by(signal, stderr, langs);
        // CPU 时间用完就是超时；别的信号和崩溃一样是这份文件处理不了。
        return Err(if signal == Signal::XCPU.as_raw() {
            ImageError::TimedOut(message)
        } else {
            ImageError::Refused(message)
        });
    }
    match status.code() {
        Some(0) => Ok(output),
        Some(code) if code == i32::from(REFUSED) => Err(ImageError::Refused(stderr.to_owned())),
        Some(code) if code == i32::from(BROKEN) => Err(ImageError::Unavailable(stderr.to_owned())),
        Some(code) if code == i32::from(TIMED_OUT) => Err(ImageError::TimedOut(stderr.to_owned())),
        // panic 的退出码是 101。
        code => Err(ImageError::Refused(crate::l10n::text(
            langs,
            "plugin-install-img-worker-crashed",
            &[("code", code.unwrap_or_default().into())],
        ))),
    }
}

fn killed_by(signal: i32, stderr: &str, langs: &Languages) -> String {
    if signal == Signal::XCPU.as_raw() {
        crate::l10n::text(
            langs,
            "plugin-install-img-worker-cpu-timeout",
            &[("seconds", CPU_SECONDS.into())],
        )
    } else if stderr.contains("memory allocation of") {
        // Rust 分配内存失败时先打印这句再 abort。
        crate::l10n::text(
            langs,
            "plugin-install-img-worker-memory-limit",
            &[("mib", (MEMORY_LIMIT >> 20).into())],
        )
    } else if signal == Signal::KILL.as_raw() {
        crate::l10n::text(langs, "plugin-install-img-worker-killed", &[])
    } else {
        crate::l10n::text(
            langs,
            "plugin-install-img-worker-signal",
            &[("signal", signal.into())],
        )
    }
}

/// 子进程的输出是一串字段，每个字段前面是 4 字节（小端）的长度。
fn put(out: &mut Vec<u8>, field: &[u8]) {
    let len = u32::try_from(field.len()).expect("字段远小于 4 GiB");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(field);
}

struct Fields<'a>(&'a [u8]);

impl<'a> Fields<'a> {
    fn bytes(&mut self, langs: &Languages) -> Result<&'a [u8], ImageError> {
        let (len, rest) = self
            .0
            .split_first_chunk::<4>()
            .ok_or_else(|| malformed(langs))?;
        let len = u32::from_le_bytes(*len) as usize;
        if rest.len() < len {
            return Err(malformed(langs));
        }
        let (field, rest) = rest.split_at(len);
        self.0 = rest;
        Ok(field)
    }

    fn number(&mut self, langs: &Languages) -> Result<u32, ImageError> {
        let bytes: [u8; 4] = self
            .bytes(langs)?
            .try_into()
            .map_err(|_| malformed(langs))?;
        Ok(u32::from_le_bytes(bytes))
    }
}

/// 子进程说处理好了，输出却读不懂：多半是 `wonton-core` 的版本对不上。
fn malformed(langs: &Languages) -> ImageError {
    ImageError::Unavailable(crate::l10n::text(
        langs,
        "plugin-install-img-worker-unavailable",
        &[(
            "error",
            crate::l10n::text(langs, "plugin-install-img-worker-malformed", &[]).into(),
        )],
    ))
}

/// 子进程和子进程里的消息用哪个语言标签：按主人的语言从目录里协商。
fn owner_language_tag(langs: &Languages) -> String {
    let tags: Vec<&str> = crate::l10n::RESOURCES.iter().map(|(tag, _)| *tag).collect();
    wonton_l10n::choose_language(langs.requested(), &tags)
        .map_or("en".to_owned(), |index| tags[index].to_owned())
}

#[cfg(test)]
mod tests;
