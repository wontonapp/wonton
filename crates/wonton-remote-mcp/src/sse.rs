use axum::body::Bytes;
use futures_util::{Stream, StreamExt, stream};
use sse_stream::{Sse, SseStream};
use std::fmt::{self, Write};

// Raw UTF-8 bytes, including field names, spaces, comments and the initial BOM.
// CR/LF delimiters are excluded from both limits; an empty line resets the event.
pub(crate) const MAX_LINE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_EVENT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EventError {
    TooLarge,
    Transport,
    Invalid,
}

impl fmt::Display for EventError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "SSE event exceeds size limit",
            Self::Transport => "SSE transport failed",
            Self::Invalid => "invalid SSE event",
        })
    }
}

impl std::error::Error for EventError {}

pub(crate) fn events(
    response: reqwest::Response,
) -> impl Stream<Item = Result<Sse, EventError>> + Send + 'static {
    decode(response.bytes_stream())
}

fn decode<S, E>(input: S) -> impl Stream<Item = Result<Sse, EventError>> + Send + 'static
where
    S: Stream<Item = Result<Bytes, E>> + Send + 'static,
    E: Send + 'static,
{
    SseStream::from_bytes_stream(bounded(input)).scan(false, |failed, result| {
        let next = if *failed {
            None
        } else {
            let result = result.map_err(|error| match error {
                sse_stream::Error::Body(error) => error
                    .downcast_ref::<EventError>()
                    .copied()
                    .unwrap_or(EventError::Transport),
                _ => EventError::Invalid,
            });
            *failed = result.is_err();
            Some(result)
        };
        std::future::ready(next)
    })
}

fn bounded<S, E>(input: S) -> impl Stream<Item = Result<Bytes, EventError>> + Send + 'static
where
    S: Stream<Item = Result<Bytes, E>> + Send + 'static,
    E: Send + 'static,
{
    let state = (Box::pin(input), Bytes::new(), Guard::default(), false);
    stream::unfold(
        state,
        |(mut input, mut chunk, mut guard, failed)| async move {
            if failed {
                return None;
            }
            while chunk.is_empty() {
                match input.next().await? {
                    Ok(bytes) => chunk = bytes,
                    Err(_) => {
                        return Some((Err(EventError::Transport), (input, chunk, guard, true)));
                    }
                }
            }
            match guard.prefix(&chunk) {
                Ok(length) => {
                    let bytes = chunk.split_to(length);
                    Some((Ok(bytes), (input, chunk, guard, false)))
                }
                Err(error) => Some((Err(error), (input, Bytes::new(), guard, true))),
            }
        },
    )
}

#[derive(Default)]
struct Guard {
    line_bytes: usize,
    event_bytes: usize,
    skip_lf: bool,
    first_line_done: bool,
    bom: [u8; 3],
}

impl Guard {
    // Yield at most one event boundary so a large transport chunk cannot cause
    // the upstream parser to allocate a queue containing the entire stream.
    fn prefix(&mut self, bytes: &[u8]) -> Result<usize, EventError> {
        for (index, &byte) in bytes.iter().enumerate() {
            if self.skip_lf {
                self.skip_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if matches!(byte, b'\r' | b'\n') {
                self.skip_lf = byte == b'\r';
                let blank = self.line_bytes == 0
                    || (!self.first_line_done
                        && self.line_bytes == 3
                        && self.bom == [0xef, 0xbb, 0xbf]);
                self.first_line_done = true;
                self.line_bytes = 0;
                if blank {
                    self.event_bytes = 0;
                    return Ok(index + 1);
                }
            } else {
                if !self.first_line_done && self.line_bytes < self.bom.len() {
                    self.bom[self.line_bytes] = byte;
                }
                if self.line_bytes == MAX_LINE_BYTES || self.event_bytes == MAX_EVENT_BYTES {
                    return Err(EventError::TooLarge);
                }
                self.line_bytes += 1;
                self.event_bytes += 1;
            }
        }
        Ok(bytes.len())
    }
}

pub(crate) fn encode(event: Sse) -> Bytes {
    let data_lines = event.data.as_ref().map_or(0, |data| {
        data.bytes().filter(|&byte| byte == b'\n').count() + 1
    });
    let capacity = 1
        + event.event.as_ref().map_or(0, |value| value.len() + 8)
        + event
            .data
            .as_ref()
            .map_or(0, |value| value.len() + data_lines * 7)
        + event.id.as_ref().map_or(0, |value| value.len() + 5)
        + if event.retry.is_some() { 28 } else { 0 };
    let mut output = String::with_capacity(capacity);
    if let Some(value) = event.event {
        field(&mut output, "event", &value);
    }
    if let Some(value) = event.data {
        // split, unlike lines(), preserves empty data and trailing empty lines.
        for line in value.split('\n') {
            field(&mut output, "data", line);
        }
    }
    if let Some(value) = event.id {
        field(&mut output, "id", &value);
    }
    if let Some(value) = event.retry {
        writeln!(output, "retry: {value}").expect("writing to a String cannot fail");
    }
    output.push('\n');
    Bytes::from(output)
}

fn field(output: &mut String, name: &str, value: &str) {
    output.push_str(name);
    output.push_str(": ");
    output.push_str(value);
    output.push('\n');
}

#[cfg(test)]
#[path = "sse_tests.rs"]
mod sse_tests;
