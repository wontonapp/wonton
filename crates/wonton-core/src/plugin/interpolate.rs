//! Compose 变量插值：`$VAR`、`${VAR}`、`${VAR:-默认}`、`${VAR-默认}`、`${VAR:?错误}`、
//! `${VAR?错误}`、`${VAR:+替换}`、`${VAR+替换}`，`$$` 表示字面量 `$`。
//!
//! 与 docker compose 不同，未定义且没有默认值的变量直接报错，而不是替换成空串：
//! 插件引用了核心不提供的变量，说明清单和权限不匹配，必须在安装前暴露出来。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::Value;

/// 片段在出错处前后各取多少个字符。
const SNIPPET_CONTEXT: usize = 20;

/// 错误带字段路径：递归走 JSON 值时由映射的键和序列的下标拼出来（如 `entrypoint[2]`）；
/// 直接对一条字符串插值（配置的 content）时路径为空。
#[derive(Debug, PartialEq, Eq)]
pub enum InterpolateError {
    Missing {
        name: String,
        path: String,
    },
    Required {
        name: String,
        message: String,
        path: String,
    },
    Syntax {
        err: SyntaxError,
        path: String,
    },
}

impl InterpolateError {
    /// 因为这个变量没有注入而失败时，变量的名字。
    pub fn variable(&self) -> Option<&str> {
        match self {
            Self::Missing { name, .. } | Self::Required { name, .. } => Some(name),
            Self::Syntax { .. } => None,
        }
    }

    /// 报错的字符串在哪个字段里，错误就定位到哪：路径只在最深处补这一次，往上冒泡不再改。
    fn with_path(self, path: &str) -> Self {
        match self {
            Self::Missing { name, .. } => Self::Missing {
                name,
                path: path.to_owned(),
            },
            Self::Required { name, message, .. } => Self::Required {
                name,
                message,
                path: path.to_owned(),
            },
            Self::Syntax { err, .. } => Self::Syntax {
                err,
                path: path.to_owned(),
            },
        }
    }
}

impl fmt::Display for InterpolateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (path, message) = match self {
            Self::Missing { name, path } => (
                path,
                format!("变量 `{name}` 未定义（插件是否声明了对应的权限或网页入口？）"),
            ),
            Self::Required {
                name,
                message,
                path,
            } => (path, format!("变量 `{name}` 是必填项：{message}")),
            Self::Syntax { err, path } => (path, err.to_string()),
        };
        if path.is_empty() {
            f.write_str(&message)
        } else {
            write!(f, "{path}：{message}")
        }
    }
}

impl std::error::Error for InterpolateError {}

/// 插值格式错误的位置：错在这条字符串里第几个字符、前后长什么样、是不是忘了 `$$` 转义。
#[derive(Debug, PartialEq, Eq)]
pub struct SyntaxError {
    /// 出错位置在这条字符串里的偏移，从 0 数（第一个字符是 0）；给人看的“第 N 个字符”是它加 1。
    pub offset: usize,
    /// 出错处前后各约 [`SNIPPET_CONTEXT`] 个字符，换行显示成 `\n`，截掉的部分用 `…` 标出。
    pub snippet: String,
    /// `$` 后面跟了 shell 才有的写法（`$@`、`$(…)`、`$1`…），提示用 `$$` 转义。
    pub shell_dollar: bool,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "第 {} 个字符附近插值格式不合法：`{}`",
            self.offset + 1,
            self.snippet
        )?;
        if self.shell_dollar {
            f.write_str("；要原样写 $ 时写成 $$")?;
        }
        Ok(())
    }
}

/// 插值用的变量表。
pub type Vars = BTreeMap<String, String>;

/// 实际求值的变量；转义和没有求值的分支不计入。
pub type UsedVars = BTreeSet<String>;

fn record(used: &mut UsedVars, name: &str) {
    if !used.contains(name) {
        used.insert(name.to_owned());
    }
}

/// 对 JSON 值里的所有字符串做插值（只处理值，不处理键，与 compose 一致）。
pub fn interpolate_value(
    value: &mut Value,
    vars: &Vars,
    used: &mut UsedVars,
) -> Result<(), InterpolateError> {
    walk(value, vars, used, &mut String::new())
}

/// `path` 是从插值起点到当前值的路径：映射的键用 `.` 连，序列的下标写成 `[n]`。
fn walk(
    value: &mut Value,
    vars: &Vars,
    used: &mut UsedVars,
    path: &mut String,
) -> Result<(), InterpolateError> {
    match value {
        Value::String(text) => {
            *text = interpolate(text, vars, used).map_err(|e| e.with_path(path))?;
            Ok(())
        }
        Value::Array(items) => items.iter_mut().enumerate().try_for_each(|(i, item)| {
            let mark = path.len();
            path.push('[');
            path.push_str(&i.to_string());
            path.push(']');
            let result = walk(item, vars, used, path);
            path.truncate(mark);
            result
        }),
        Value::Object(map) => map.iter_mut().try_for_each(|(key, item)| {
            let mark = path.len();
            if !path.is_empty() {
                path.push('.');
            }
            path.push_str(key);
            let result = walk(item, vars, used, path);
            path.truncate(mark);
            result
        }),
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
    }
}

enum Expansion<'a> {
    Escaped,
    Plain(&'a str),
    Braced { name: &'a str, op: &'a str },
}

/// `at` 指向 `$`；扫描引用和实际求值共用转义、变量名和括号的解析。
fn expansion(input: &str, at: usize) -> Result<(Expansion<'_>, &str), InterpolateError> {
    let after = &input[at + 1..];
    if let Some(tail) = after.strip_prefix('$') {
        return Ok((Expansion::Escaped, tail));
    }
    if let Some(body) = after.strip_prefix('{') {
        let end = matching_brace(body).ok_or_else(|| syntax_error(input, at, false))?;
        let inner = &body[..end];
        let len = name_len(inner);
        if len == 0 {
            return Err(syntax_error(input, at, false));
        }
        let (name, op) = inner.split_at(len);
        if !op.is_empty()
            && ![":-", "-", ":?", "?", ":+", "+"]
                .iter()
                .any(|prefix| op.starts_with(prefix))
        {
            return Err(syntax_error(input, at, false));
        }
        return Ok((Expansion::Braced { name, op }, &body[end + 1..]));
    }
    let len = name_len(after);
    if len == 0 {
        return Err(syntax_error(input, at, shell_special(after)));
    }
    Ok((Expansion::Plain(&after[..len]), &after[len..]))
}

/// 只收集没有默认值的引用；分支里的必需变量也算，转义和错误消息里的文字不算。
pub fn references(input: &str, found: &mut impl FnMut(&str)) -> Result<(), InterpolateError> {
    let mut rest = input;
    while let Some(pos) = rest.find('$') {
        let at = input.len() - rest.len() + pos;
        let (token, tail) = expansion(input, at)?;
        match token {
            Expansion::Escaped => {}
            Expansion::Plain(name) => found(name),
            Expansion::Braced { name, op } => {
                if op.is_empty() || op.starts_with(":?") || op.starts_with('?') {
                    found(name);
                }
                // 必填项的错误信息不做插值，与实际求值一致。
                if let Some(word) = [":-", "-", ":+", "+"]
                    .iter()
                    .find_map(|prefix| op.strip_prefix(prefix))
                {
                    references(word, found)?;
                }
            }
        }
        rest = tail;
    }
    Ok(())
}

pub fn interpolate(
    input: &str,
    vars: &Vars,
    used: &mut UsedVars,
) -> Result<String, InterpolateError> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(pos) = rest.find('$') {
        let at = input.len() - rest.len() + pos;
        out.push_str(&rest[..pos]);
        let (token, tail) = expansion(input, at)?;
        match token {
            Expansion::Escaped => out.push('$'),
            Expansion::Plain(name) => {
                record(used, name);
                let value = vars.get(name).ok_or_else(|| InterpolateError::Missing {
                    name: name.to_owned(),
                    path: String::new(),
                })?;
                out.push_str(value);
            }
            Expansion::Braced { name, op } => {
                out.push_str(&expand_braced(name, op, vars, used)?);
            }
        }
        rest = tail;
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

/// `$` 后面跟的是 shell 才有的写法（`$(…)`、`$@`、`$*`、`$#`、`$?`、`$!`、`$1`、`$-`）：
/// compose 没有这些，多半是想原样写 `$` 而忘了 `$$`。
fn shell_special(after: &str) -> bool {
    matches!(
        after.chars().next(),
        Some('(' | '@' | '*' | '#' | '?' | '!' | '-' | '0'..='9')
    )
}

/// 在字节偏移 `at` 处的 `$` 报格式错误：偏移换成字符数，片段只取前后各约 [`SNIPPET_CONTEXT`]
/// 个字符，整条长字符串不进错误。
fn syntax_error(input: &str, at: usize, shell_dollar: bool) -> InterpolateError {
    let offset = input[..at].chars().count();
    InterpolateError::Syntax {
        err: SyntaxError {
            offset,
            snippet: snippet(input, offset),
            shell_dollar,
        },
        path: String::new(),
    }
}

/// 出错处前后各约 [`SNIPPET_CONTEXT`] 个字符的片段：换行显示成 `\n`，截掉的部分用 `…` 标出。
fn snippet(input: &str, at: usize) -> String {
    let start = at.saturating_sub(SNIPPET_CONTEXT);
    let end = at + SNIPPET_CONTEXT;
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    for (i, c) in input.chars().enumerate().skip(start) {
        if i > end {
            break;
        }
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    if end + 1 < input.chars().count() {
        out.push('…');
    }
    out
}

fn expand_braced(
    name: &str,
    op: &str,
    vars: &Vars,
    used: &mut UsedVars,
) -> Result<String, InterpolateError> {
    record(used, name);
    let value = vars.get(name);
    let set_nonempty = value.is_some_and(|v| !v.is_empty());
    let mut resolve = |word: &str| interpolate(word, vars, used);
    let required = |message: &str| InterpolateError::Required {
        name: name.to_owned(),
        message: message.to_owned(),
        path: String::new(),
    };
    if op.is_empty() {
        return value.cloned().ok_or_else(|| InterpolateError::Missing {
            name: name.to_owned(),
            path: String::new(),
        });
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
        unreachable!("expansion validates the operator")
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
            assert_eq!(
                interpolate(input, &v, &mut UsedVars::new()).unwrap(),
                expected,
                "{input}"
            );
        }
    }

    #[test]
    fn missing_and_required_variables_fail() {
        let v = vars();
        let missing = |name: &str| InterpolateError::Missing {
            name: name.to_owned(),
            path: String::new(),
        };
        assert_eq!(
            interpolate("${NOPE}", &v, &mut UsedVars::new()),
            Err(missing("NOPE"))
        );
        assert_eq!(
            interpolate("$NOPE", &v, &mut UsedVars::new()),
            Err(missing("NOPE"))
        );
        assert!(matches!(
            interpolate("${EMPTY:?必须设置}", &v, &mut UsedVars::new()),
            Err(InterpolateError::Required { .. })
        ));
        for bad in ["${", "${A", "$-x", "${:-x}", "${A!x}"] {
            assert!(
                matches!(
                    interpolate(bad, &v, &mut UsedVars::new()),
                    Err(InterpolateError::Syntax { .. })
                ),
                "{bad}"
            );
        }
    }

    /// 走一遍插值，取回 `Syntax` 错误的字段路径和位置信息。
    fn syntax_of(value: &mut Value) -> (String, SyntaxError) {
        let err = interpolate_value(value, &vars(), &mut UsedVars::new()).unwrap_err();
        let InterpolateError::Syntax { err, path } = err else {
            panic!("{err:?}")
        };
        (path, err)
    }

    /// 错误带字段路径和出错位置：路径由映射的键和序列的下标拼出来，偏移按字符数，
    /// 长字符串只留出错处前后的片段。
    #[test]
    fn syntax_errors_carry_path_and_offset() {
        // 嵌套映射：路径用点连。
        let (path, err) = syntax_of(&mut serde_json::json!({
            "app": {"environment": {"CMD": "$@"}}
        }));
        assert_eq!(path, "app.environment.CMD");
        assert_eq!(err.offset, 0);
        assert!(err.shell_dollar, "{}", err.snippet);

        // 序列：下标写成 [n]。
        let (path, err) = syntax_of(&mut serde_json::json!({
            "entrypoint": ["sh", "-c", "echo $@"]
        }));
        assert_eq!(path, "entrypoint[2]");
        assert_eq!(err.offset, 5);
        assert_eq!(err.snippet, "echo $@");
        // 给人看的“第 N 个字符”从 1 数：`$` 是第 6 个。
        assert!(err.to_string().contains("第 6 个字符"), "{}", err);

        // 多行字符串：换行按一个字符算，片段里显示成 \n。
        let (path, err) = syntax_of(&mut serde_json::json!({
            "entrypoint": ["sh", "-c", "echo hi\nexec $@"]
        }));
        assert_eq!(path, "entrypoint[2]");
        assert_eq!(err.offset, 13);
        assert_eq!(err.snippet, "echo hi\\nexec $@");

        // 中文按字符数，不按字节。
        let (path, err) = syntax_of(&mut serde_json::json!({"command": "你好，世界 $@"}));
        assert_eq!(path, "command");
        assert_eq!(err.offset, 6);
        assert_eq!(err.snippet, "你好，世界 $@");

        // 整条长字符串不再出现在错误里：片段截在出错处前后各 20 个字符，截掉的部分用 … 标出。
        let long = format!("{}$@{}", "x".repeat(100), "y".repeat(100));
        let (path, err) = syntax_of(&mut serde_json::json!({"entrypoint": [long]}));
        assert_eq!(path, "entrypoint[0]");
        assert_eq!(err.offset, 100);
        assert_eq!(
            err.snippet,
            format!("…{}$@{}…", "x".repeat(20), "y".repeat(19))
        );
    }

    /// `$` 后面跟 shell 的写法时提示用 `$$` 转义；其他格式错误不提示。
    #[test]
    fn shell_syntax_hints_at_escaping_the_dollar() {
        for input in ["$(cmd)", "$@", "$*", "$#", "$?", "$!", "$-", "$1"] {
            let Err(InterpolateError::Syntax { err, .. }) =
                interpolate(input, &vars(), &mut UsedVars::new())
            else {
                panic!("{input}")
            };
            assert!(err.shell_dollar, "{input}");
        }
        for input in ["${", "${A", "${:-x}", "${A!x}"] {
            let Err(InterpolateError::Syntax { err, .. }) =
                interpolate(input, &vars(), &mut UsedVars::new())
            else {
                panic!("{input}")
            };
            assert!(!err.shell_dollar, "{input}");
        }
    }

    /// 未定义的变量也带路径，定位到引用它的字段。
    #[test]
    fn missing_variables_carry_the_field_path() {
        let mut doc = serde_json::json!({"environment": {"TOKEN": "$NOPE"}});
        assert_eq!(
            interpolate_value(&mut doc, &vars(), &mut UsedVars::new()),
            Err(InterpolateError::Missing {
                name: "NOPE".into(),
                path: "environment.TOKEN".into(),
            })
        );
    }

    #[test]
    fn walks_nested_json_values_only() {
        let mut doc = serde_json::json!({"${A}": ["$A", {"k": "${A}"}, 3, true, null]});
        interpolate_value(&mut doc, &vars(), &mut UsedVars::new()).unwrap();
        assert_eq!(
            doc,
            serde_json::json!({"${A}": ["1", {"k": "1"}, 3, true, null]})
        );
    }

    #[test]
    fn required_references_include_branches_but_not_optional_names_or_literal_text() {
        let mut found = UsedVars::new();
        references(
            "$A ${B:-${C}} ${D+${E}} $${F} ${G:?${H}} ${I?${J}}",
            &mut |name| record(&mut found, name),
        )
        .unwrap();
        let expected: UsedVars = ["A", "C", "E", "G", "I"].map(String::from).into();
        assert_eq!(found, expected);
    }

    #[test]
    fn reference_scanning_rejects_the_same_invalid_tokens_as_interpolation() {
        for input in ["${", "${A", "$-x", "${:-x}", "${A!x}"] {
            let scan = references(input, &mut |_| {}).unwrap_err();
            let evaluate = interpolate(input, &vars(), &mut UsedVars::new()).unwrap_err();
            assert_eq!(scan, evaluate, "{input}");
        }
    }
}
