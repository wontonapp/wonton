//! Redact diagnostic text before the MCP SDK can log or return a remote credential.

use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::sync::Arc;

use serde_json::Value;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Redaction(Arc<[(String, Option<String>)]>);

const SECRET_MARKER: &str = "[secret]";

struct Region<'a> {
    start: usize,
    end: usize,
    url: Option<UrlRegion<'a>>,
}

struct UrlRegion<'a> {
    start: usize,
    end: usize,
    template: &'a str,
}

impl Redaction {
    pub(crate) fn new<'a>(
        url: &str,
        template: &str,
        values: impl IntoIterator<Item = &'a str>,
    ) -> Option<Self> {
        let mut patterns = BTreeMap::new();
        for value in values.into_iter().filter(|value| !value.is_empty()) {
            patterns.insert(value.to_owned(), None);
            patterns.insert(
                percent_encoding::utf8_percent_encode(
                    value,
                    super::proxy_auth::URL_SECRET_COMPONENT,
                )
                .to_string(),
                None,
            );
            let json = serde_json::to_string(value).expect("strings serialize");
            patterns.insert(json[1..json.len() - 1].to_owned(), None);
        }
        if url != template {
            patterns.insert(url.to_owned(), Some(template.to_owned()));
        }
        if patterns.is_empty() {
            return None;
        }
        let mut patterns: Vec<_> = patterns.into_iter().collect();
        patterns.sort_unstable_by_key(|(needle, _)| std::cmp::Reverse(needle.len()));
        Some(Self(patterns.into()))
    }

    pub(crate) fn text<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let mut pending = BinaryHeap::new();
        for (index, (needle, _)) in self.0.iter().enumerate() {
            if let Some(start) = text.find(needle) {
                pending.push(Reverse((start, index)));
            }
        }
        if pending.is_empty() {
            return Cow::Borrowed(text);
        }
        let mut output = String::with_capacity(text.len());
        let mut copied = 0;
        let mut region: Option<Region<'_>> = None;
        while let Some(Reverse((start, index))) = pending.pop() {
            let (needle, template) = &self.0[index];
            let end = start + needle.len();
            if let Some(complete) = region.take_if(|current| start >= current.end) {
                complete.write(text, &mut copied, &mut output);
            }
            let current = region.get_or_insert(Region {
                start,
                end,
                url: None,
            });
            current.end = current.end.max(end);
            if let Some(template) = template {
                let url = current.url.get_or_insert_with(|| UrlRegion {
                    start,
                    end,
                    template,
                });
                url.end = url.end.max(end);
            }
            // Advance by one character, not the match width: even one secret can overlap itself.
            let search = start
                + text[start..]
                    .chars()
                    .next()
                    .expect("nonempty match")
                    .len_utf8();
            if let Some(next) = text[search..].find(needle) {
                pending.push(Reverse((search + next, index)));
            }
        }
        if let Some(complete) = region {
            complete.write(text, &mut copied, &mut output);
        }
        output.push_str(&text[copied..]);
        Cow::Owned(output)
    }

    pub(crate) fn error(&self, error: &mut rmcp::model::ErrorData) {
        if let Cow::Owned(redacted) = self.text(&error.message) {
            error.message = redacted.into();
        }
        if let Some(data) = &mut error.data {
            self.json(data);
        }
    }

    fn json(&self, value: &mut Value) {
        match value {
            Value::String(text) => {
                if let Cow::Owned(redacted) = self.text(text) {
                    *text = redacted;
                }
            }
            Value::Array(values) => {
                for value in values {
                    self.json(value);
                }
            }
            Value::Object(fields) => {
                let keys: Vec<_> = fields
                    .keys()
                    .filter_map(|key| match self.text(key) {
                        Cow::Owned(redacted) => Some((key.clone(), redacted)),
                        Cow::Borrowed(_) => None,
                    })
                    .collect();
                for (key, redacted) in keys {
                    if let Some(value) = fields.remove(&key) {
                        fields.insert(redacted, value);
                    }
                }
                for value in fields.values_mut() {
                    self.json(value);
                }
            }
            Value::Number(number) => {
                let literal = number.to_string();
                self.scalar(value, &literal);
            }
            Value::Bool(boolean) => {
                let literal = if *boolean { "true" } else { "false" };
                self.scalar(value, literal);
            }
            Value::Null => self.scalar(value, "null"),
        }
    }

    fn scalar(&self, value: &mut Value, literal: &str) {
        if let Cow::Owned(redacted) = self.text(literal) {
            *value = Value::String(redacted);
        }
    }
}

impl Region<'_> {
    fn write(self, text: &str, copied: &mut usize, output: &mut String) {
        output.push_str(&text[*copied..self.start]);
        match self.url {
            Some(url) => {
                if self.start < url.start {
                    output.push_str(SECRET_MARKER);
                }
                output.push_str(url.template);
                if self.end > url.end {
                    output.push_str(SECRET_MARKER);
                }
            }
            None => output.push_str(SECRET_MARKER),
        }
        *copied = self.end;
    }
}
