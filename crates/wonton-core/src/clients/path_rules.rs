//! 公网入口的路径规则和请求头名规范化（ADR-0021“路径怎么匹配”）。
//!
//! 应用解析路径的方式各不相同（Kestrel 保留 `%2F` 再去点段、不合并斜杠；Express、chi 按字面路由），
//! 所以同一段原始路径按不解码、解码一次、解码两次算三份候选，`deny` 和 `auth_failures` 命中任何一份就成立，
//! `allow` 三份都得成立。转发给插件的始终是原始路径，这里不返回改写后的路径。

use std::fmt;

use percent_encoding::percent_decode_str;

/// 公网路径限制，二选一（ADR-0021）。局域网入口不受影响。
#[derive(Debug, Clone)]
pub enum PathRules {
    /// 只放行这些前缀。
    Allow(Vec<String>),
    /// 这些前缀之外一律拒绝。
    Deny(Vec<String>),
}

/// `auth_failures` 的一条：这些路径上回这些状态码，算一次验证失败（ADR-0021）。
#[derive(Debug, Clone)]
pub struct AuthFailureRule {
    pub path: String,
    pub status: Vec<u16>,
}

/// 原始路径里有应用会各自解成点段的写法，入口直接回 400 `invalid_input`。
#[derive(Debug, PartialEq, Eq)]
pub struct Invalid;

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("路径里有点段")
    }
}

impl std::error::Error for Invalid {}

/// 原始路径、解码一次、解码两次三份。非法的百分号序列按原样保留，不回 400，也不丢掉候选。
fn decodings(raw_path: &str) -> [String; 3] {
    let once = percent_decode_str(raw_path)
        .decode_utf8_lossy()
        .into_owned();
    let twice = percent_decode_str(&once).decode_utf8_lossy().into_owned();
    [raw_path.to_owned(), once, twice]
}

/// 去掉段里 `;` 起的部分（路径参数）。
fn strip_params(segment: &str) -> &str {
    match segment.find(';') {
        Some(end) => &segment[..end],
        None => segment,
    }
}

/// 三份解码里只要有一份切出 `.` 或 `..` 段就非法：这样 `%2e%2e`、`..;` 和要解码后才切得开的
/// `..%2f` 都到不了应用（ADR-0009 的规则收紧）。
pub fn check_dot_segments(raw_path: &str) -> Result<(), Invalid> {
    for decoding in decodings(raw_path) {
        for segment in decoding.split(['/', '\\']) {
            let segment = strip_params(segment);
            if segment == "." || segment == ".." {
                return Err(Invalid);
            }
        }
    }
    Ok(())
}

/// 一份候选：去掉 `;` 的用于 `deny`、`auth_failures`，保留 `;` 的字面形式给 `allow` 再判一次，
/// 按字面处理 `;` 的应用看到的是 `share;x`。
struct Candidate {
    stripped: String,
    literal: String,
}

/// 用同一套分段规则规范化声明和候选；保留 `;` 的版本供 allow 判字面路径。
fn normalized_segments(path: &str, strip: bool) -> impl Iterator<Item = &str> {
    path.split(['/', '\\'])
        .map(move |segment| {
            if strip {
                strip_params(segment)
            } else {
                segment
            }
        })
        .filter(|segment| !segment.is_empty())
}

fn normalize_path(path: &str, strip: bool) -> String {
    let mut normalized = String::with_capacity(path.len().max(1));
    for segment in normalized_segments(path, strip) {
        normalized.push('/');
        normalized.extend(segment.chars().flat_map(char::to_lowercase));
    }
    if normalized.is_empty() {
        normalized.push('/');
    }
    normalized
}

/// 三份解码各自 `\` 换成 `/`、去掉段里的 `;`、合并 `//`、转小写，结果以 `/` 开头。
fn candidates_with_params(raw_path: &str) -> Vec<Candidate> {
    let mut candidates: Vec<Candidate> = Vec::new();
    for decoding in decodings(raw_path) {
        let candidate = Candidate {
            stripped: normalize_path(&decoding, true),
            literal: normalize_path(&decoding, false),
        };
        if !candidates.iter().any(|existing| {
            existing.stripped == candidate.stripped && existing.literal == candidate.literal
        }) {
            candidates.push(candidate);
        }
    }
    candidates
}

/// 三份规范化后的候选，已去重；`allow` 之外只用这一份。
pub fn candidates(raw_path: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    for decoding in decodings(raw_path) {
        let candidate = normalize_path(&decoding, true);
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

/// 原始路径及最多两次解码中，字面或去点段后可能触及 `/_wonton` 的请求都归核心。
/// 不改变普通路径的点段准入策略；大小写、分隔符和路径参数沿用入口匹配规则。
pub fn is_reserved_path(raw_path: &str) -> bool {
    decodings(raw_path).iter().any(|decoding| {
        let mut depth = 0usize;
        for segment in normalized_segments(decoding, true) {
            match segment {
                "." => {}
                ".." => depth = depth.saturating_sub(1),
                _ => {
                    if depth == 0 && segment.eq_ignore_ascii_case("_wonton") {
                        return true;
                    }
                    depth += 1;
                }
            }
        }
        false
    })
}

/// 按整段前缀匹配，不分大小写：`/share` 只匹配 `/share` 和 `/share/...`。清单值都以 `/` 开头（C1 校验），
/// 这里仍把空值当作根，匹配全部。
fn matches_prefix(prefix: &str, candidate: &str) -> bool {
    let mut candidate = normalized_segments(candidate, false);
    normalized_segments(prefix, false).all(|segment| {
        candidate.next().is_some_and(|candidate| {
            segment
                .chars()
                .flat_map(char::to_lowercase)
                .eq(candidate.chars())
        })
    })
}

fn hits(prefixes: &[String], candidate: &str) -> bool {
    prefixes
        .iter()
        .any(|prefix| matches_prefix(prefix, candidate))
}

/// 这个公网请求能不能到插件。`deny` 任何一份候选命中就拒；`allow` 三份候选，连不去 `;` 的字面形式都要命中。
pub fn public_allows(rules: &PathRules, raw_path: &str) -> bool {
    let candidates = candidates_with_params(raw_path);
    match rules {
        PathRules::Deny(prefixes) => !candidates
            .iter()
            .any(|candidate| hits(prefixes, &candidate.stripped)),
        PathRules::Allow(prefixes) => candidates.iter().all(|candidate| {
            hits(prefixes, &candidate.stripped) && hits(prefixes, &candidate.literal)
        }),
    }
}

/// 这个响应算不算一次验证失败：任何一份候选命中声明的路径，且状态码在列表里。
pub fn is_auth_failure(rules: &[AuthFailureRule], raw_path: &str, status: u16) -> bool {
    let candidates = candidates(raw_path);
    rules.iter().any(|rule| {
        rule.status.contains(&status)
            && candidates
                .iter()
                .any(|candidate| matches_prefix(&rule.path, candidate))
    })
}

/// 删除请求头时按这个比较：`_` 换成 `-`、忽略大小写（CGI、WSGI、FastCGI、PHP 和很多 Python 应用
/// 把 `X_Remote_User` 和 `X-Remote-User` 映射成同一个变量）。
pub fn normalize_header_name(name: &str) -> String {
    name.replace('_', "-").to_lowercase()
}

pub fn same_header(a: &str, b: &str) -> bool {
    normalize_header_name(a) == normalize_header_name(b)
}
