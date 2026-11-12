//! 消息文字里的链接：认出来、补全协议，按核心的判断给类别（ADR-0065 第 3 节）。壳只按类别打开。
//!
//! 认这些东西：http(s) 网址、不带协议的域名、邮箱、`mailto:`、电话（`tel:` 和带 `+` 的 E.164 号码）、
//! 云吞自己的 `wonton://`。认不出的就不是链接，不猜。

use std::net::IpAddr;
use std::ops::Range;

use linkify::{LinkFinder, LinkKind};
use percent_encoding::percent_decode_str;
use url::Url;
use wonton_protocol::{MatrixTarget, PluginsViewModel, RichLink, RichLinkKind};

use super::rich::CJK_PUNCTUATION;

/// 认链接要用到的这台服务器的事实：只有已装插件的网页地址。别的都从地址本身看。
#[derive(Default)]
pub(crate) struct LinkTargets {
    /// 插件的网页地址和插件 ID：来源和路径都对得上才是这个插件的页面。
    plugins: Vec<(String, Url)>,
}

impl LinkTargets {
    pub(crate) fn of(plugins: &PluginsViewModel) -> Self {
        let installed = plugins
            .installed
            .as_ref()
            .map(|installed| installed.plugins.as_slice())
            .unwrap_or_default();
        Self {
            plugins: installed
                .iter()
                .filter_map(|plugin| {
                    let base = Url::parse(plugin.web_url.as_deref()?).ok()?;
                    Some((plugin.id.clone(), base))
                })
                .collect(),
        }
    }

    /// 一段文字是不是链接，是的话是什么链接。`found` 已经去掉了句尾的中文标点。
    pub(crate) fn link(&self, found: &str, kind: &LinkKind) -> Option<RichLink> {
        if found.starts_with('+')
            && let Some(link) = self.tel(found, 8)
        {
            return Some(link);
        }
        if let Ok(url) = Url::parse(found) {
            match url.scheme() {
                "http" | "https" => return self.web(url),
                "mailto" => return self.mail(url.path()),
                "tel" => return self.tel(url.path(), 3),
                "wonton" => {
                    return Some(RichLink {
                        url: url.to_string(),
                        kind: RichLinkKind::Other,
                    });
                }
                // `example.com:8080` 会解析成 scheme 是 `example.com`，按不带协议的地址再试一次。
                _ => {}
            }
        }
        if *kind == LinkKind::Email {
            return self.mail(found);
        }
        self.schemeless(found)
    }

    /// Markdown 里显式写出来的目标。相对路径（`main.rs`、`docs/a.md`）不补协议，显示成普通文字；带协议的照常分类。
    pub(crate) fn explicit(&self, found: &str) -> Option<RichLink> {
        let url = Url::parse(found).ok()?;
        let kind = match url.scheme() {
            "http" | "https" => return self.web(url),
            "mailto" => return self.mail(url.path()),
            "tel" => return self.tel(url.path(), 3),
            "wonton" => RichLinkKind::Other,
            _ => return None,
        };
        Some(RichLink {
            url: url.to_string(),
            kind,
        })
    }

    /// 不带协议的地址：顶级域要在公共后缀表里，并躲开和文件扩展名撞名的那些。
    fn schemeless(&self, text: &str) -> Option<RichLink> {
        let url = Url::parse(&format!("http://{text}")).ok()?;
        let host = url.host_str()?;
        if host.parse::<IpAddr>().is_ok() {
            return self.web(url);
        }
        let tld = host.rsplit_once('.')?.1.to_ascii_lowercase();
        // 公共后缀表对不认识的顶级域也回一个后缀，只有表里的顶级域才带类型。
        let known = SPECIAL_USE_TLDS.contains(&tld.as_str())
            || psl::suffix(tld.as_bytes()).is_some_and(|suffix| {
                suffix.typ().is_some() && suffix.as_bytes() == tld.as_bytes()
            });
        if !known {
            return None;
        }
        // 没有别的线索（路径、端口、`www.`）时，和常见文件扩展名撞名的顶级域按文件名算：Agent 天天说
        // `main.rs`、`config.yaml`。代价是单独一个 `example.app` 要点成链接得加上路径或 `www.`。
        let bare = url.path() == "/"
            && url.port().is_none()
            && !host.to_ascii_lowercase().starts_with("www.");
        if bare && FILE_EXTENSIONS.contains(&tld.as_str()) {
            return None;
        }
        self.web(url)
    }

    fn web(&self, url: Url) -> Option<RichLink> {
        url.host_str()?;
        let kind = match matrix_target(&url) {
            Some(target) => RichLinkKind::Matrix { target },
            None => self.plugin(&url),
        };
        Some(RichLink {
            url: url.to_string(),
            kind,
        })
    }

    /// 没有 scheme 的邮箱。
    fn mail(&self, address: &str) -> Option<RichLink> {
        Url::parse(&format!("mailto:{address}")).ok()?;
        Some(RichLink {
            url: format!("mailto:{address}"),
            kind: RichLinkKind::Email,
        })
    }

    /// 号码。正文里裸写的按 E.164 至少 8 位收；`tel:` 明写出来的（`fewest` 是 3）连内部短号也认。
    fn tel(&self, text: &str, fewest: usize) -> Option<RichLink> {
        let number = canonical_phone(text)?;
        (fewest..=15)
            .contains(&digits(&number))
            .then_some(RichLink {
                url: format!("tel:{number}"),
                kind: RichLinkKind::Phone,
            })
    }

    fn plugin(&self, url: &Url) -> RichLinkKind {
        let origin = url.origin();
        self.plugins
            .iter()
            .find(|(_, base)| base.origin() == origin && url.path().starts_with(base.path()))
            .map_or(RichLinkKind::External, |(plugin_id, _)| {
                RichLinkKind::Plugin {
                    plugin_id: plugin_id.clone(),
                }
            })
    }
}

/// 公共后缀表之外、家里服务器自己用的顶级域：`localhost` 的插件地址和 `internal` 的别名。
const SPECIAL_USE_TLDS: [&str; 4] = ["internal", "local", "localhost", "test"];

/// 和文件扩展名撞名的顶级域：只有地址里没有路径、端口和 `www.` 前缀时才算文件名。消息里的文件名比这些域名常见。
const FILE_EXTENSIONS: [&str; 25] = [
    "app", "css", "csv", "gif", "gz", "html", "jpeg", "jpg", "js", "json", "log", "md", "mov",
    "mp3", "mp4", "pdf", "pl", "png", "py", "rs", "sh", "svg", "toml", "yaml", "yml",
];

/// matrix.to 的用户、房间和事件永久链接。别的 matrix.to 地址（没有片段、指向客户端选择页的）是外部网页。
pub(crate) fn matrix_target(url: &Url) -> Option<MatrixTarget> {
    if !matches!(url.scheme(), "http" | "https") || url.host_str()? != "matrix.to" {
        return None;
    }
    // matrix.to 把 via 等查询参数放在片段里；它们不是 Matrix 标识的一部分。
    let fragment = url.fragment()?.split('?').next()?;
    let fragment = percent_decode_str(fragment).decode_utf8().ok()?;
    let path = fragment.strip_prefix('/')?;
    let (id, event) = match path.split_once('/') {
        Some((id, rest)) => (id, Some(rest)),
        None => (path, None),
    };
    let event_id = match event {
        None => None,
        Some(id) if id.strip_prefix('$').is_some_and(|name| !name.is_empty()) => {
            Some(id.to_owned())
        }
        Some(_) => return None,
    };
    let (sigil, name) = id.split_at_checked(1)?;
    let (local, server) = name.split_once(':')?;
    if local.is_empty() || server.is_empty() {
        return None;
    }
    match sigil {
        "@" => Some(MatrixTarget::User {
            user_id: id.to_owned(),
        }),
        "#" | "!" => Some(MatrixTarget::Room {
            room_id: id.to_owned(),
            event_id,
        }),
        _ => None,
    }
}

/// 文字里的号码：`+` 紧跟国家码的 E.164 号码（至少 8 位），以及 `tel:` 明写出来的号码（短号也认）。
/// 允许空格、`-` 和括号分组。裸号码前面的字符是 ASCII 字母或数字时不算（`C+86…` 里的 `+` 太可能是算式）；
/// 紧跟在汉字后面算。
pub(crate) fn phones(text: &str) -> Vec<Range<usize>> {
    let mut phones = tel_numbers(text);
    let mut at = 0;
    while let Some(found) = text[at..].find('+') {
        let start = at + found;
        at = start + 1;
        if phones.iter().any(|phone| phone.contains(&start)) {
            continue;
        }
        let before = text[..start].chars().next_back();
        if before.is_some_and(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        let Some(end) = number_end(&text[start..]) else {
            continue;
        };
        if !(8..=15).contains(&digits(&text[start..start + end])) {
            continue;
        }
        phones.push(start..start + end);
        at = start + end;
    }
    phones.sort_by_key(|phone| phone.start);
    phones.dedup_by(|later, earlier| later.start < earlier.end);
    phones
}

/// 文字里第一个外部 http(s) 链接，链接预览用它（ADR-0065）：`raw` 是原文里找到的那段（发事件时写进
/// `matched_url`），`url` 是补全协议后的地址（取预览用它）。插件页和 matrix.to 不是外部链接。
pub(crate) struct ExternalLink {
    pub(crate) raw: String,
    pub(crate) url: String,
}

/// 扫描顺序和 `rich` 排版一样：号码先切出来，`linkify` 认网址和邮箱，中文标点隔开的段各自判断。
pub(crate) fn first_external_link(text: &str, links: &LinkTargets) -> Option<ExternalLink> {
    let mut at = 0;
    for phone in phones(text) {
        if let Some(link) = external_in(&text[at..phone.start], links) {
            return Some(link);
        }
        at = phone.end;
    }
    external_in(&text[at..], links)
}

fn external_in(text: &str, links: &LinkTargets) -> Option<ExternalLink> {
    let mut finder = LinkFinder::new();
    finder.kinds(&[LinkKind::Url, LinkKind::Email]);
    finder.url_must_have_scheme(false);
    for part in finder.spans(text) {
        if part.kind().is_none() {
            continue;
        }
        let mut rest = part.as_str();
        while !rest.is_empty() {
            let cut = rest.find(CJK_PUNCTUATION).unwrap_or(rest.len());
            let (piece, tail) = rest.split_at(cut);
            if let Some(link) = piece_link(piece, links) {
                return Some(link);
            }
            let punctuation = tail.chars().take(1).count();
            rest = &tail[punctuation..];
        }
    }
    None
}

/// 一段切开的文字：是外部链接就返回。
fn piece_link(piece: &str, links: &LinkTargets) -> Option<ExternalLink> {
    if piece.is_empty() {
        return None;
    }
    let kind = if piece.contains('@') && !piece.contains("://") {
        LinkKind::Email
    } else {
        LinkKind::Url
    };
    let link = links.link(piece, &kind)?;
    (link.kind == RichLinkKind::External).then(|| ExternalLink {
        raw: piece.to_owned(),
        url: link.url,
    })
}

const TEL: &str = "tel:";

/// `tel:` 后面跟的数字（前面允许空格）。整个片段都在返回的区间里，`rich` 照原样交给 `tel:` 的解析。
fn tel_numbers(text: &str) -> Vec<Range<usize>> {
    let mut numbers = Vec::new();
    for (start, _) in text.to_ascii_lowercase().match_indices(TEL) {
        if text[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        {
            continue;
        }
        let after = start + TEL.len();
        let rest = &text[after..];
        let lead = rest.len() - rest.trim_start_matches(' ').len();
        let Some(end) = number_end(&rest[lead..]) else {
            continue;
        };
        if (3..=15).contains(&digits(&rest[lead..lead + end])) {
            numbers.push(start..after + lead + end);
        }
    }
    numbers
}

/// 从 `+` 开始量到号码结束，去掉末尾的分隔符。返回的片段至少有一位数字。
fn number_end(text: &str) -> Option<usize> {
    let mut end = 0;
    for (at, c) in text.char_indices() {
        let ok = match c {
            '+' => at == 0,
            '0'..='9' => true,
            ' ' | '-' | '(' | ')' | '.' => true,
            _ => false,
        };
        if !ok {
            break;
        }
        if c.is_ascii_digit() {
            end = at + c.len_utf8();
        }
    }
    (end > 0).then_some(end)
}

fn digits(text: &str) -> usize {
    text.bytes().filter(u8::is_ascii_digit).count()
}

/// `tel:` 里的号码：去掉分隔符，只留数字和开头的 `+`。
fn canonical_phone(text: &str) -> Option<String> {
    let number: String = text
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect();
    let plus = number.starts_with('+') || !number.contains('+');
    (plus && (2..=15).contains(&digits(&number))).then_some(number)
}

/// 测试里直接给出插件表，不用造整个 `PluginsViewModel`。
#[cfg(test)]
pub(crate) fn test_targets(pairs: &[(&str, &str)]) -> LinkTargets {
    LinkTargets {
        plugins: pairs
            .iter()
            .map(|(id, url)| ((*id).to_owned(), Url::parse(url).unwrap()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(targets: &LinkTargets, found: &str, kind: LinkKind) -> RichLinkKind {
        targets.link(found, &kind).unwrap().kind
    }

    #[test]
    fn plugin_pages_are_recognized_by_origin_and_path() {
        let targets = test_targets(&[
            ("app.grafana", "http://app-grafana.home.example:18480/"),
            ("app.notes", "https://notes.example/home/"),
        ]);
        assert_eq!(
            kind(
                &targets,
                "http://app-grafana.home.example:18480/explore",
                LinkKind::Url
            ),
            RichLinkKind::Plugin {
                plugin_id: "app.grafana".into()
            }
        );
        assert_eq!(
            kind(&targets, "https://notes.example/home/deep", LinkKind::Url),
            RichLinkKind::Plugin {
                plugin_id: "app.notes".into()
            }
        );
        // 同一个站点别的路径、别的端口都是外部网页。
        assert_eq!(
            kind(&targets, "https://notes.example/other", LinkKind::Url),
            RichLinkKind::External
        );
        assert_eq!(
            kind(
                &targets,
                "http://app-grafana.home.example:9999/",
                LinkKind::Url
            ),
            RichLinkKind::External
        );
    }

    #[test]
    fn matrix_links_carry_a_user_or_room() {
        let targets = LinkTargets::default();
        assert_eq!(
            kind(
                &targets,
                "https://matrix.to/#/@family:home.example",
                LinkKind::Url
            ),
            RichLinkKind::Matrix {
                target: MatrixTarget::User {
                    user_id: "@family:home.example".into()
                }
            }
        );
        assert_eq!(
            kind(
                &targets,
                "https://matrix.to/#/%23room:home.example/$event",
                LinkKind::Url
            ),
            RichLinkKind::Matrix {
                target: MatrixTarget::Room {
                    room_id: "#room:home.example".into(),
                    event_id: Some("$event".into())
                }
            }
        );
        // 没有片段的 matrix.to 是客户端选择页。
        assert_eq!(
            kind(&targets, "https://matrix.to/", LinkKind::Url),
            RichLinkKind::External
        );
        assert_eq!(
            kind(&targets, "https://matrix.to/#/not-a-link", LinkKind::Url),
            RichLinkKind::External
        );
    }
    #[test]
    fn matrix_routing_parameters_do_not_change_the_target() {
        let targets = LinkTargets::default();
        for (url, event_id) in [
            (
                "https://matrix.to/#/%23room:example.com?via=example.com",
                None,
            ),
            (
                "https://matrix.to/#/%23room:example.com/%24event?via=example.com",
                Some("$event".to_owned()),
            ),
        ] {
            assert_eq!(
                kind(&targets, url, LinkKind::Url),
                RichLinkKind::Matrix {
                    target: MatrixTarget::Room {
                        room_id: "#room:example.com".into(),
                        event_id
                    },
                }
            );
        }
    }

    /// 显式写的 Markdown 目标要带协议：相对路径（`main.rs`、`docs/a.md`）不是网址。
    #[test]
    fn explicit_targets_need_a_scheme() {
        let targets = LinkTargets::default();
        assert!(targets.explicit("main.rs").is_none());
        assert!(targets.explicit("docs/a.md").is_none());
        assert_eq!(
            targets.explicit("https://example.com/x").unwrap().url,
            "https://example.com/x"
        );
        assert_eq!(
            targets.explicit("mailto:a@example.com").unwrap().kind,
            RichLinkKind::Email
        );
    }

    /// 家里服务器自己的域名不带协议也是链接，插件地址照样认成插件页。
    #[test]
    fn local_server_names_are_links() {
        let targets = test_targets(&[("app.grafana", "http://app-grafana.home.localhost:18480/")]);
        let link = targets
            .link("app-grafana.home.localhost:18480/explore", &LinkKind::Url)
            .unwrap();
        assert_eq!(
            link.kind,
            RichLinkKind::Plugin {
                plugin_id: "app.grafana".into()
            }
        );
        assert_eq!(
            targets
                .link("wonton.localhost", &LinkKind::Url)
                .unwrap()
                .kind,
            RichLinkKind::External
        );
    }

    #[test]
    fn bare_domains_need_a_real_top_level_domain() {
        let targets = LinkTargets::default();
        assert!(targets.link("main.rs", &LinkKind::Url).is_none());
        assert!(targets.link("config.yaml", &LinkKind::Url).is_none());
        assert!(targets.link("a.b", &LinkKind::Url).is_none());
        assert!(targets.link("example.notatld", &LinkKind::Url).is_none());
        for link in ["x.io", "example.cn", "example.md/x", "example.md:8080"] {
            let link = targets.link(link, &LinkKind::Url).unwrap();
            assert_eq!(link.kind, RichLinkKind::External, "{link:?}");
        }
    }

    #[test]
    fn phones_need_a_country_code_and_whole_digits() {
        assert_eq!(phones("打 +86 138-0013-8000，谢谢"), vec![4..21]);
        assert_eq!(phones("电话+8613800138000，谢谢"), vec![6..20]);
        assert_eq!(phones("C++17 和 1+2"), Vec::<Range<usize>>::new());
        assert_eq!(phones("+1234567"), Vec::<Range<usize>>::new());
        assert_eq!(phones("a+8613800138000"), Vec::<Range<usize>>::new());
        // `tel:` 前缀算进同一个区间，里面的 `+` 不另算一次。
        assert_eq!(phones("打 tel:+86 138 0013 8000"), vec![4..25]);
    }

    #[test]
    fn tel_numbers_are_canonicalized() {
        let targets = LinkTargets::default();
        let link = targets
            .link("tel:+86 (138) 0013-8000", &LinkKind::Url)
            .unwrap();
        assert_eq!(link.url, "tel:+8613800138000");
        assert_eq!(link.kind, RichLinkKind::Phone);
    }
}
