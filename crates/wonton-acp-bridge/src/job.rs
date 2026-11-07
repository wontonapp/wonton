//! 一条消息解析成的活：普通、排队、打断、插话。上限来自配置。

use wonton_l10n::{Arg, Lang};

use crate::{config::Limits, l10n::text};

#[derive(Clone, Debug)]
pub(crate) enum JobKind {
    Normal,
    Queue,
    Interrupt,
    Steer,
}

#[derive(Clone, Debug)]
pub(crate) struct Job {
    pub(crate) text: String,
    pub(crate) kind: JobKind,
}

pub(crate) enum ParseError {
    TooLong(usize),
    Empty,
    MissingText,
}

pub(crate) fn parse_error(lang: Lang, error: ParseError) -> String {
    match error {
        ParseError::TooLong(max) => text(lang, "input-too-long", &[("max", Arg::from(max as i64))]),
        ParseError::Empty => text(lang, "input-empty", &[]),
        ParseError::MissingText => text(lang, "control-without-text", &[]),
    }
}

pub(crate) fn parse_job(text: &str, limits: &Limits) -> Result<Job, ParseError> {
    if text.len() > limits.max_input_bytes {
        return Err(ParseError::TooLong(limits.max_input_bytes));
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(ParseError::Empty);
    }
    let (kind, body) = if let Some(body) = trimmed.strip_prefix("/wonton interrupt ") {
        (JobKind::Interrupt, body)
    } else if let Some(body) = trimmed.strip_prefix("/wonton queue ") {
        (JobKind::Queue, body)
    } else if let Some(body) = trimmed.strip_prefix("/wonton steer ") {
        (JobKind::Steer, body)
    } else {
        (JobKind::Normal, trimmed)
    };
    if body.trim().is_empty() {
        return Err(ParseError::MissingText);
    }
    Ok(Job {
        kind,
        text: body.trim().to_owned(),
    })
}
