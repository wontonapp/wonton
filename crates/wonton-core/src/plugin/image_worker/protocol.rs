//! 核心和图片助手之间的管道帧。
//!
//! 两个方向都是一行表头加原始字节，表头字段用空格分开、以换行结束：
//!
//! - 核心 → 助手：`<任务号> <时限毫秒> <任务> <语言> <输入长度>`，后面跟着这么长的输入。
//! - 助手 → 核心：`<任务号> <结果码> <输出长度> <原因长度>`，后面跟着输出和原因。结果码和
//!   [`super::child`] 的退出码一样（0 成功、1 文件不合规格、2 助手这边的问题），另有 3：到时限没做完。
//! - 助手起来先说一行 `0 <协议版本>`。
//!
//! 表头用文本，测试里的假助手脚本不用解析二进制也能说话。

use std::io;
use std::time::Duration;

use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt,
};

use super::{BROKEN, MAX_OUTPUT, MAX_STDERR, REFUSED, TIMED_OUT};

/// 助手子命令，核心内部用。
pub(super) const HOST_COMMAND: &str = "image-host";
/// 助手说的协议版本。核心和助手是同一个可执行文件，升级时可能对不上；启动时核对。
pub(super) const PROTOCOL: u32 = 1;
/// 表头最多这么长：输入长度、时限这些数字远到不了。
const MAX_HEADER: usize = 128;
/// 一个任务的输入最多这么大。上限是 [`super::Job::input_limit`] 里最大的一种加一。
const MAX_INPUT: u64 = (64 << 20) + 1;
/// 时限最多这么久，超过的按这么久算。
const MAX_TIMEOUT_MS: u64 = 10 * 60 * 1000;
/// 任务名和语言标签最多这么长。
const MAX_WORD: usize = 32;

/// 核心发给助手的任务。
pub(super) struct Request<'a> {
    pub id: u32,
    pub timeout: Duration,
    pub job: &'a str,
    pub lang: &'a str,
    pub input: &'a [u8],
}

/// 助手收到的任务。
pub(super) struct Incoming {
    pub id: u32,
    pub timeout: Duration,
    pub job: String,
    pub lang: String,
    pub input: Vec<u8>,
}

/// 助手对任务的回答。
pub(super) struct Reply {
    pub id: u32,
    pub code: u8,
    pub output: Vec<u8>,
    pub message: Vec<u8>,
}

impl Reply {
    pub(super) fn ok(id: u32, output: Vec<u8>) -> Self {
        Self {
            id,
            code: 0,
            output,
            message: Vec::new(),
        }
    }

    pub(super) fn refused(id: u32, message: String) -> Self {
        Self {
            id,
            code: REFUSED,
            output: Vec::new(),
            message: message.into_bytes(),
        }
    }

    pub(super) fn timed_out(id: u32, message: String) -> Self {
        Self {
            id,
            code: TIMED_OUT,
            output: Vec::new(),
            message: message.into_bytes(),
        }
    }

    pub(super) fn broken(id: u32, message: String) -> Self {
        Self {
            id,
            code: BROKEN,
            output: Vec::new(),
            message: message.into_bytes(),
        }
    }
}

pub(super) async fn write_request<W: AsyncWrite + Unpin>(
    to: &mut W,
    request: &Request<'_>,
) -> io::Result<()> {
    let header = format!(
        "{} {} {} {} {}\n",
        request.id,
        request.timeout.as_millis(),
        request.job,
        request.lang,
        request.input.len()
    );
    to.write_all(header.as_bytes()).await?;
    to.write_all(request.input).await?;
    to.flush().await
}

/// 读一个任务；核心那头到了结尾（退出）时给 `None`。
pub(super) async fn read_request<R: AsyncBufRead + Unpin>(
    from: &mut R,
) -> io::Result<Option<Incoming>> {
    let Some(header) = read_header(from).await? else {
        return Ok(None);
    };
    let fields: Vec<&str> = header.split_whitespace().collect();
    let [id, timeout, job, lang, input] = fields.as_slice() else {
        return Err(invalid("表头的字段数不对"));
    };
    let id: u32 = number(id, "任务号")?;
    if id == 0 {
        return Err(invalid("任务号不能是 0"));
    }
    let timeout = Duration::from_millis(number::<u64>(timeout, "时限")?.min(MAX_TIMEOUT_MS));
    for (word, what) in [(job, "任务"), (lang, "语言")] {
        if word.is_empty() || word.len() > MAX_WORD {
            return Err(invalid(&format!("{what}写错了")));
        }
    }
    let input: u64 = number(input, "输入长度")?;
    if input > MAX_INPUT {
        return Err(invalid("输入太长"));
    }
    Ok(Some(Incoming {
        id,
        timeout,
        job: (*job).to_owned(),
        lang: (*lang).to_owned(),
        input: read_bytes(from, input as usize).await?,
    }))
}

/// 取消一个还等着的任务：和请求一样的表头，任务写 `cancel`。
pub(super) async fn write_cancel<W: AsyncWrite + Unpin>(to: &mut W, id: u32) -> io::Result<()> {
    to.write_all(format!("{id} 0 cancel cancel 0\n").as_bytes())
        .await?;
    to.flush().await
}

pub(super) async fn write_reply<W: AsyncWrite + Unpin>(
    to: &mut W,
    reply: &Reply,
) -> io::Result<()> {
    let header = format!(
        "{} {} {} {}\n",
        reply.id,
        reply.code,
        reply.output.len(),
        reply.message.len()
    );
    to.write_all(header.as_bytes()).await?;
    to.write_all(&reply.output).await?;
    to.write_all(&reply.message).await?;
    to.flush().await
}

/// 读一个回答；助手退出（核心那头到了结尾）时给 `None`。
pub(super) async fn read_reply<R: AsyncBufRead + Unpin>(from: &mut R) -> io::Result<Option<Reply>> {
    let Some(header) = read_header(from).await? else {
        return Ok(None);
    };
    let fields: Vec<&str> = header.split_whitespace().collect();
    let [id, code, output, message] = fields.as_slice() else {
        return Err(invalid("表头的字段数不对"));
    };
    let id: u32 = number(id, "任务号")?;
    let code = number::<u8>(code, "结果码")?;
    let output: u64 = number(output, "输出长度")?;
    let message: u64 = number(message, "原因长度")?;
    if output > MAX_OUTPUT as u64 || message > MAX_STDERR as u64 {
        return Err(invalid("回答太长"));
    }
    Ok(Some(Reply {
        id,
        code,
        output: read_bytes(from, output as usize).await?,
        message: read_bytes(from, message as usize).await?,
    }))
}

pub(super) async fn write_ready<W: AsyncWrite + Unpin>(to: &mut W) -> io::Result<()> {
    to.write_all(format!("0 {PROTOCOL}\n").as_bytes()).await?;
    to.flush().await
}

/// 读助手起来的那一行。核心是在一个版本上编的，对不上说明磁盘上的可执行文件换了。
pub(super) async fn read_ready<R: AsyncBufRead + Unpin>(from: &mut R) -> io::Result<u32> {
    let Some(header) = read_header(from).await? else {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "助手起来就退出了",
        ));
    };
    let fields: Vec<&str> = header.split_whitespace().collect();
    let ["0", protocol] = fields.as_slice() else {
        return Err(invalid("助手的第一行不对"));
    };
    number::<u32>(protocol, "协议版本")
}

async fn read_header<R: AsyncBufRead + Unpin>(from: &mut R) -> io::Result<Option<String>> {
    let mut line = Vec::new();
    if from.read_until(b'\n', &mut line).await? == 0 {
        return Ok(None);
    }
    if line.len() > MAX_HEADER {
        return Err(invalid("表头太长"));
    }
    if line.pop() != Some(b'\n') {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "表头没有换行"));
    }
    String::from_utf8(line)
        .map(Some)
        .map_err(|_| invalid("表头不是 UTF-8"))
}

async fn read_bytes<R: AsyncRead + Unpin>(from: &mut R, len: usize) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; len];
    from.read_exact(&mut bytes).await?;
    Ok(bytes)
}

fn number<T: std::str::FromStr>(field: &str, what: &str) -> io::Result<T> {
    field.parse().map_err(|_| invalid(&format!("{what}写错了")))
}

fn invalid(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_owned())
}
