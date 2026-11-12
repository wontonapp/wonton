use tokio::io::{AsyncBufRead, AsyncBufReadExt};

use crate::config::MAX_FRAME_BYTES;

/// [`read_frame`] 的一次结果。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum FrameRead {
    /// 读到一帧，在 `frame` 里。
    Frame,
    /// 流正常结束。
    Eof,
    /// 一行超过 `MAX_FRAME_BYTES`：按现有超长帧的处理断开会话。
    Oversize,
    /// 传输出错。
    Transport,
}

/// 超长帧：留一条 warn 作排查线索，返回 [`FrameRead::Oversize`]。
fn oversize() -> FrameRead {
    tracing::warn!(
        limit = MAX_FRAME_BYTES,
        "Agent 一行输出超过单帧上限，断开等待重连"
    );
    FrameRead::Oversize
}

/// 读一帧进 `frame`（一行 JSON-RPC，不含换行）。
///
/// 边读边限长：缓冲超过 [`MAX_FRAME_BYTES`] 还见不到换行就立即返回
/// [`FrameRead::Oversize`]，不把整行读进内存。缓冲上界是 `MAX_FRAME_BYTES + 1`，
/// 多出的 1 字节留给回车，让「内容正好上限、以 `\r\n` 结尾」的行照旧按去回车后的
/// 长度算一帧。
///
/// 流结束时和 `BufReader::lines` 一样，残留的最后一行（无换行）照常算一帧。
pub(super) async fn read_frame<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    frame: &mut Vec<u8>,
) -> FrameRead {
    frame.clear();
    loop {
        // 一次 fill_buf 的借用里把要留的字节复制进 frame；consume 放在借用外。
        let (consumed, done) = {
            let Ok(available) = reader.fill_buf().await else {
                return FrameRead::Transport;
            };
            if available.is_empty() {
                return if frame.is_empty() {
                    FrameRead::Eof
                } else if frame.len() > MAX_FRAME_BYTES {
                    oversize()
                } else {
                    FrameRead::Frame
                };
            }
            match available.iter().position(|&byte| byte == b'\n') {
                Some(pos) => {
                    if frame.len() + pos > MAX_FRAME_BYTES + 1 {
                        return oversize();
                    }
                    frame.extend_from_slice(&available[..pos]);
                    if frame.ends_with(b"\r") {
                        frame.pop();
                    }
                    let done = if frame.len() > MAX_FRAME_BYTES {
                        oversize()
                    } else {
                        FrameRead::Frame
                    };
                    (pos + 1, Some(done))
                }
                None => {
                    if frame.len() + available.len() > MAX_FRAME_BYTES + 1 {
                        return oversize();
                    }
                    frame.extend_from_slice(available);
                    (available.len(), None)
                }
            }
        };
        reader.consume(consumed);
        if let Some(done) = done {
            return done;
        }
    }
}
