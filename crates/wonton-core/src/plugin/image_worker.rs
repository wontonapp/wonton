//! 在子进程里处理不可信的图片（[`super::images`]）：`wonton-core image <icon.png|icon.svg|screenshot|thumbnail|preview>` 从标准输入读
//! 原文件，把结果写到标准输出。图片是不可信的输入，预检挡不住所有费时费内存的写法（比如很细的虚线），所以解码和渲染放进单独的
//! 进程：它先给自己设内存和 CPU 时间的上限，到时没做完由核心杀掉。超时、内存不够、崩溃都算这张图处理不了。
//!
//! 子进程只限资源，不是沙箱：它和核心是同一个用户，文件和网络的权限也一样。

use std::io::{Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, ExitStatus, Stdio};
use std::time::Duration;

use rustix::process::{Resource, Rlimit, Signal, getrlimit, setrlimit};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::Semaphore;

use super::images::{self, Icon, IconFormat, Screenshot};

/// 子命令（`wonton-core image …`），核心内部用。
pub const COMMAND: &str = "image";
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

/// 图片处理不了的原因。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImageError {
    /// 图片本身的问题：不合规格，或者处理时超时、内存不够、崩溃。同一张图再处理一遍还是这样。
    #[error("{0}")]
    Refused(String),
    /// 核心这边起不了或用不了处理图片的进程，和图片无关。
    #[error("处理图片的进程不可用：{0}")]
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
        }
    }

    /// 最多从标准输入读这么多。多读一个字节：超了由各自的大小检查报出来。
    fn input_limit(self) -> u64 {
        let limit = match self {
            Self::Thumbnail | Self::Preview => images::MAX_THUMBNAIL_INPUT,
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
        ]
        .into_iter()
        .find(|job| job.arg() == arg)
    }

    fn run(self, input: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        match self {
            Self::Icon(format) => put_icon(&mut out, images::icon(format, input)?),
            Self::TabIcon(format) => put_icon(&mut out, images::tab_icon(format, input)?),
            Self::Screenshot => {
                let shot = images::screenshot(input)?;
                put(&mut out, &shot.jpeg);
                put(&mut out, &shot.width.to_le_bytes());
                put(&mut out, &shot.height.to_le_bytes());
            }
            Self::Thumbnail => put(&mut out, &images::thumbnail(input)?),
            Self::Preview => put(&mut out, &images::preview(input)?),
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
    let job = match args {
        [arg] => Job::parse(arg),
        _ => None,
    };
    let Some(job) = job else {
        eprintln!(
            "用法：wonton-core {COMMAND} <icon.png|icon.svg|tab-icon.png|tab-icon.svg|screenshot|thumbnail|preview>（核心内部用）"
        );
        return ExitCode::from(BROKEN);
    };
    if let Err(e) = limit_resources() {
        eprintln!("设不了资源上限：{e}");
        return ExitCode::from(BROKEN);
    }
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin()
        .take(job.input_limit())
        .read_to_end(&mut input)
    {
        eprintln!("读标准输入失败：{e}");
        return ExitCode::from(BROKEN);
    }
    let output = match job.run(&input) {
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
            eprintln!("写标准输出失败：{e}");
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

/// 核心这边：起子进程处理图片，限时，限并发。
pub struct ImageWorker {
    program: Program,
    timeout: Duration,
    running: Semaphore,
}

enum Program {
    Path(PathBuf),
    /// 单元测试里没有 `wonton-core` 可执行文件：在线程池里处理，不限资源。
    #[cfg(test)]
    InProcess,
}

impl ImageWorker {
    /// 用正在运行的 `wonton-core` 自己处理。
    pub fn current() -> std::io::Result<Self> {
        Ok(Self::new(std::env::current_exe()?))
    }

    /// 用给定的 `wonton-core` 可执行文件处理。
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self::with_program(Program::Path(program.into()))
    }

    #[cfg(test)]
    pub(crate) fn in_process() -> Self {
        Self::with_program(Program::InProcess)
    }

    fn with_program(program: Program) -> Self {
        Self {
            program,
            timeout: TIMEOUT,
            running: Semaphore::new(MAX_RUNNING),
        }
    }

    /// 等一个子进程最多多久（默认 10 秒）。
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub async fn icon(&self, format: IconFormat, data: &[u8]) -> Result<Icon, ImageError> {
        let output = self.run(Job::Icon(format), data).await?;
        parse_icon(&output)
    }

    pub async fn tab_icon(&self, format: IconFormat, data: &[u8]) -> Result<Icon, ImageError> {
        let output = self.run(Job::TabIcon(format), data).await?;
        parse_icon(&output)
    }

    pub async fn screenshot(&self, data: &[u8]) -> Result<Screenshot, ImageError> {
        let output = self.run(Job::Screenshot, data).await?;
        let mut fields = Fields(&output);
        let shot = Screenshot {
            jpeg: fields.bytes()?.to_vec(),
            width: fields.number()?,
            height: fields.number()?,
        };
        Ok(shot)
    }

    pub async fn thumbnail(&self, data: &[u8]) -> Result<Vec<u8>, ImageError> {
        let output = self.run(Job::Thumbnail, data).await?;
        Ok(Fields(&output).bytes()?.to_vec())
    }

    pub async fn preview(&self, data: &[u8]) -> Result<Vec<u8>, ImageError> {
        let output = self.run(Job::Preview, data).await?;
        Ok(Fields(&output).bytes()?.to_vec())
    }

    async fn run(&self, job: Job, input: &[u8]) -> Result<Vec<u8>, ImageError> {
        let _permit = self.running.acquire().await.expect("信号量不会关闭");
        self.execute(job, input).await
    }

    async fn execute(&self, job: Job, input: &[u8]) -> Result<Vec<u8>, ImageError> {
        match &self.program {
            Program::Path(program) => self.spawn(program, job, input).await,
            #[cfg(test)]
            Program::InProcess => {
                let input = input.to_vec();
                tokio::task::spawn_blocking(move || job.run(&input))
                    .await
                    .expect("处理图片的线程没有崩溃")
                    .map_err(ImageError::Refused)
            }
        }
    }

    async fn spawn(&self, program: &Path, job: Job, input: &[u8]) -> Result<Vec<u8>, ImageError> {
        let mut child = Command::new(program)
            .args([COMMAND, job.arg()])
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                ImageError::Unavailable(format!("启动 {} 失败：{e}", program.display()))
            })?;
        let (Some(mut stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            unreachable!("标准输入输出都设成了管道");
        };
        let (mut output, mut message) = (Vec::new(), Vec::new());
        let finished = tokio::time::timeout(self.timeout, async {
            let write = async {
                // 子进程提前退出了（比如文件太大，读到上限就不读了）就写不进去，结果看退出状态。
                let _ = stdin.write_all(input).await;
                drop(stdin);
            };
            let (_, out, err) = tokio::join!(
                write,
                read_capped(stdout, MAX_OUTPUT, &mut output),
                read_capped(stderr, MAX_STDERR, &mut message),
            );
            out.and(err)?;
            child.wait().await
        })
        .await;
        let status = match finished {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                return Err(ImageError::Unavailable(format!("和子进程通信失败：{e}")));
            }
            Err(_) => {
                // kill 会等进程退出并回收，返回时子进程已经不在了。
                if let Err(e) = child.kill().await {
                    tracing::warn!("结束处理图片的子进程失败：{e}");
                }
                return Err(ImageError::Refused(format!(
                    "处理超过 {} 秒，已停止",
                    self.timeout.as_secs_f32()
                )));
            }
        };
        outcome(status, output, &String::from_utf8_lossy(&message))
    }
}

fn parse_icon(output: &[u8]) -> Result<Icon, ImageError> {
    let mut fields = Fields(output);
    Ok(Icon {
        small: fields.bytes()?.to_vec(),
        large: fields.bytes()?.to_vec(),
        transparent_corner: fields.bytes()? == [1],
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
fn outcome(status: ExitStatus, output: Vec<u8>, stderr: &str) -> Result<Vec<u8>, ImageError> {
    let stderr = stderr.trim();
    if let Some(signal) = status.signal() {
        return Err(ImageError::Refused(killed_by(signal, stderr)));
    }
    match status.code() {
        Some(0) => Ok(output),
        Some(code) if code == i32::from(REFUSED) => Err(ImageError::Refused(stderr.to_owned())),
        Some(code) if code == i32::from(BROKEN) => Err(ImageError::Unavailable(stderr.to_owned())),
        // panic 的退出码是 101。
        code => Err(ImageError::Refused(format!(
            "处理时崩溃（退出码 {}）",
            code.unwrap_or_default()
        ))),
    }
}

fn killed_by(signal: i32, stderr: &str) -> String {
    if signal == Signal::XCPU.as_raw() {
        format!("处理超过 {CPU_SECONDS} 秒 CPU 时间，已停止")
    } else if stderr.contains("memory allocation of") {
        // Rust 分配内存失败时先打印这句再 abort。
        format!("处理时要的内存超过 {} MiB", MEMORY_LIMIT >> 20)
    } else if signal == Signal::KILL.as_raw() {
        "处理图片的进程被杀掉了（SIGKILL），多半是内存不够".to_owned()
    } else {
        format!("处理图片的进程被信号 {signal} 结束")
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
    fn bytes(&mut self) -> Result<&'a [u8], ImageError> {
        let (len, rest) = self.0.split_first_chunk::<4>().ok_or_else(malformed)?;
        let len = u32::from_le_bytes(*len) as usize;
        if rest.len() < len {
            return Err(malformed());
        }
        let (field, rest) = rest.split_at(len);
        self.0 = rest;
        Ok(field)
    }

    fn number(&mut self) -> Result<u32, ImageError> {
        let bytes: [u8; 4] = self.bytes()?.try_into().map_err(|_| malformed())?;
        Ok(u32::from_le_bytes(bytes))
    }
}

/// 子进程说处理好了，输出却读不懂：多半是 `wonton-core` 的版本对不上。
fn malformed() -> ImageError {
    ImageError::Unavailable("处理图片的进程输出的格式不对".to_owned())
}

#[cfg(test)]
mod tests;
