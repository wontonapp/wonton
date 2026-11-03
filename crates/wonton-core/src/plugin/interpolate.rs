//! Compose 变量插值：`$VAR`、`${VAR}`、`${VAR:-默认}`、`${VAR-默认}`、`${VAR:?错误}`、
//! `${VAR?错误}`、`${VAR:+替换}`、`${VAR+替换}`，`$$` 表示字面量 `$`。
//!
//! 与 docker compose 不同，未定义且没有默认值的变量直接报错，而不是替换成空串：
//! 插件引用了核心不提供的变量，说明清单和权限不匹配，必须在安装前暴露出来。

use std::collections::BTreeMap;

use serde_json::Value;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InterpolateError {
    #[error("变量 `{0}` 未定义（插件是否声明了对应的权限或网页入口？）")]
    Missing(String),
    #[error("变量 `{name}` 是必填项：{message}")]
    Required { name: String, message: String },
    #[error("插值格式不合法：`{0}`")]
    Syntax(String),
}

impl InterpolateError {
    /// 因为这个变量没有注入而失败时，变量的名字。
    pub fn variable(&self) -> Option<&str> {
        match self {
            Self::Missing(name) | Self::Required { name, .. } => Some(name),
            Self::Syntax(_) => None,
        }
    }
}

/// 插值用的变量表。
pub type Vars = BTreeMap<String, String>;

/// 对 JSON 值里的所有字符串做插值（只处理值，不处理键，与 compose 一致）。
pub fn interpolate_value(value: &mut Value, vars: &Vars) -> Result<(), InterpolateError> {
    match value {
        Value::String(text) => {
            *text = interpolate(text, vars)?;
            Ok(())
        }
        Value::Array(items) => items
            .iter_mut()
            .try_for_each(|item| interpolate_value(item, vars)),
        Value::Object(map) => map
            .values_mut()
            .try_for_each(|item| interpolate_value(item, vars)),
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
    }
}

pub fn interpolate(input: &str, vars: &Vars) -> Result<String, InterpolateError> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(pos) = rest.find('$') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        if let Some(tail) = after.strip_prefix('$') {
            out.push('$');
            rest = tail;
        } else if let Some(body) = after.strip_prefix('{') {
            let end = matching_brace(body).ok_or_else(|| InterpolateError::Syntax(input.into()))?;
            out.push_str(&expand_braced(&body[..end], vars, input)?);
            rest = &body[end + 1..];
        } else {
            let len = name_len(after);
            if len == 0 {
                return Err(InterpolateError::Syntax(input.to_owned()));
            }
            let name = &after[..len];
            let value = vars
                .get(name)
                .ok_or_else(|| InterpolateError::Missing(name.to_owned()))?;
            out.push_str(value);
            rest = &after[len..];
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// 返回与开头 `${` 配对的 `}` 在 `body` 中的位置，支持嵌套的 `${...}`。
fn matching_brace(body: &str) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'$' if bytes.get(i + 1) == Some(&b'$') => i += 1,
            b'$' if bytes.get(i + 1) == Some(&b'{') => {
                depth += 1;
                i += 1;
            }
            b'}' if depth == 0 => return Some(i),
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

fn name_len(text: &str) -> usize {
    text.bytes()
        .enumerate()
        .take_while(|(i, b)| {
            b.is_ascii_alphabetic() || *b == b'_' || (*i > 0 && b.is_ascii_digit())
        })
        .count()
}

fn expand_braced(inner: &str, vars: &Vars, input: &str) -> Result<String, InterpolateError> {
    let len = name_len(inner);
    if len == 0 {
        return Err(InterpolateError::Syntax(input.to_owned()));
    }
    let (name, op) = inner.split_at(len);
    let value = vars.get(name);
    let set_nonempty = value.is_some_and(|v| !v.is_empty());
    let resolve = |word: &str| interpolate(word, vars);
    let required = |message: &str| InterpolateError::Required {
        name: name.to_owned(),
        message: message.to_owned(),
    };
    if op.is_empty() {
        return value
            .cloned()
            .ok_or_else(|| InterpolateError::Missing(name.to_owned()));
    }
    let present = |cond: bool| value.filter(|_| cond).cloned();
    if let Some(word) = op.strip_prefix(":-") {
        present(set_nonempty).map_or_else(|| resolve(word), Ok)
    } else if let Some(word) = op.strip_prefix('-') {
        present(value.is_some()).map_or_else(|| resolve(word), Ok)
    } else if let Some(message) = op.strip_prefix(":?") {
        present(set_nonempty).ok_or_else(|| required(message))
    } else if let Some(message) = op.strip_prefix('?') {
        present(value.is_some()).ok_or_else(|| required(message))
    } else if let Some(word) = op.strip_prefix(":+") {
        if set_nonempty {
            resolve(word)
        } else {
            Ok(String::new())
        }
    } else if let Some(word) = op.strip_prefix('+') {
        if value.is_some() {
            resolve(word)
        } else {
            Ok(String::new())
        }
    } else {
        Err(InterpolateError::Syntax(input.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vars {
        [
            ("A", "1"),
            ("EMPTY", ""),
            ("WONTON_APP_DATA", "/srv/apps/x"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
    }

    #[test]
    fn expands_compose_forms() {
        let v = vars();
        let cases = [
            (
                "${WONTON_APP_DATA}/upload:/data",
                "/srv/apps/x/upload:/data",
            ),
            ("$A-$A", "1-1"),
            ("cost $$5", "cost $5"),
            ("${MISSING:-d}", "d"),
            ("${EMPTY:-d}", "d"),
            ("${EMPTY-d}", ""),
            ("${MISSING-${A}}", "1"),
            ("${A:+set}", "set"),
            ("${EMPTY:+set}", ""),
            ("${EMPTY+set}", "set"),
            ("${MISSING:-${MISSING2:-deep}}", "deep"),
        ];
        for (input, expected) in cases {
            assert_eq!(interpolate(input, &v).unwrap(), expected, "{input}");
        }
    }

    #[test]
    fn missing_and_required_variables_fail() {
        let v = vars();
        assert_eq!(
            interpolate("${NOPE}", &v),
            Err(InterpolateError::Missing("NOPE".into()))
        );
        assert_eq!(
            interpolate("$NOPE", &v),
            Err(InterpolateError::Missing("NOPE".into()))
        );
        assert!(matches!(
            interpolate("${EMPTY:?必须设置}", &v),
            Err(InterpolateError::Required { .. })
        ));
        for bad in ["${", "${A", "$-x", "${:-x}", "${A!x}"] {
            assert!(
                matches!(interpolate(bad, &v), Err(InterpolateError::Syntax(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn walks_nested_json_values_only() {
        let mut doc = serde_json::json!({"${A}": ["$A", {"k": "${A}"}, 3, true, null]});
        interpolate_value(&mut doc, &vars()).unwrap();
        assert_eq!(
            doc,
            serde_json::json!({"${A}": ["1", {"k": "1"}, 3, true, null]})
        );
    }
}
