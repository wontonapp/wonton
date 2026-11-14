//! Redact diagnostic text before the MCP SDK can log or return a remote credential.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Redaction(Arc<[(String, Option<String>)]>);

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
        patterns.sort_unstable_by(|a, b| b.0.len().cmp(&a.0.len()));
        Some(Self(patterns.into()))
    }

    pub(crate) fn text<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let mut next = self.next(text);
        if next.is_none() {
            return Cow::Borrowed(text);
        }
        let mut remaining = text;
        let mut redacted = String::with_capacity(text.len());
        while let Some((start, (needle, replacement))) = next {
            redacted.push_str(&remaining[..start]);
            redacted.push_str(replacement.as_deref().unwrap_or("[secret]"));
            remaining = &remaining[start + needle.len()..];
            next = self.next(remaining);
        }
        redacted.push_str(remaining);
        Cow::Owned(redacted)
    }

    fn next<'a>(&'a self, text: &str) -> Option<(usize, &'a (String, Option<String>))> {
        self.0
            .iter()
            .filter_map(|pattern| text.find(&pattern.0).map(|start| (start, pattern)))
            .min_by_key(|(start, _)| *start)
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
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
}
