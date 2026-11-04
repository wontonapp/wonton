//! 帮助文档的一页：`user/<语言>/<主题>.md`，开头是 frontmatter（ADR-0034）。

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub lang: String,
    pub title: String,
    pub summary: String,
    pub requires_core: Option<String>,
    pub requires_app: Option<String>,
    pub body: String,
}

/// 主题 → 语言 → 页。
pub type Pages = BTreeMap<String, BTreeMap<String, Page>>;

#[derive(Deserialize)]
struct Front {
    title: String,
    summary: String,
    requires_core: Option<String>,
    requires_app: Option<String>,
}

pub fn parse(lang: &str, text: &str) -> Result<Page, String> {
    let rest = text
        .strip_prefix("---\n")
        .ok_or("开头没有 frontmatter".to_owned())?;
    let (front, body) = rest
        .split_once("\n---\n")
        .ok_or("frontmatter 没有结束".to_owned())?;
    let front: Front =
        serde_saphyr::from_str(front).map_err(|e| format!("frontmatter 读不懂：{e}"))?;
    if front.title.trim().is_empty() || front.summary.trim().is_empty() {
        return Err("title 和 summary 不能为空".to_owned());
    }
    Ok(Page {
        lang: lang.to_owned(),
        title: front.title,
        summary: front.summary,
        requires_core: front.requires_core,
        requires_app: front.requires_app,
        body: body.trim_start_matches('\n').to_owned(),
    })
}

/// `user/<语言>/<主题>.md` 拆成语言和主题，别的路径为空。路径来自索引，要拼进磁盘路径，所以只收这些字符。
pub fn split_path(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("user/")?;
    let (lang, file) = rest.split_once('/')?;
    let topic = file.strip_suffix(".md")?;
    let lang_ok = !lang.is_empty() && lang.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    let topic_ok = !topic.is_empty()
        && topic
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    (lang_ok && topic_ok).then_some((lang, topic))
}

/// 目录里的全部页。不是 `user/<语言>/<主题>.md` 的文件不算；一页读不懂时整份都不用，文档不能只对一半。
pub fn load_dir(dir: &Path) -> Result<Pages, String> {
    let mut pages = Pages::new();
    let user = dir.join("user");
    let read =
        |path: &Path| std::fs::read_dir(path).map_err(|e| format!("{}：{e}", path.display()));
    for lang in read(&user)? {
        let lang = lang.map_err(|e| e.to_string())?;
        if !lang.path().is_dir() {
            continue;
        }
        for file in read(&lang.path())? {
            let file = file.map_err(|e| e.to_string())?;
            let path = format!(
                "user/{}/{}",
                lang.file_name().to_string_lossy(),
                file.file_name().to_string_lossy()
            );
            let Some((lang, topic)) = split_path(&path) else {
                continue;
            };
            let text = std::fs::read_to_string(file.path()).map_err(|e| format!("{path}：{e}"))?;
            let page = parse(lang, &text).map_err(|e| format!("{path}：{e}"))?;
            pages
                .entry(topic.to_owned())
                .or_default()
                .insert(lang.to_owned(), page);
        }
    }
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_outside_the_docs_is_refused() {
        assert_eq!(split_path("user/zh/devices.md"), Some(("zh", "devices")));
        assert_eq!(
            split_path("user/zh-Hans/a-b2.md"),
            Some(("zh-Hans", "a-b2"))
        );
        for bad in [
            "user/../secret.md",
            "user/zh/../../secret.md",
            "user/zh/a/b.md",
            "user/zh/.md",
            "user/README.md",
            "skills/x.md",
            "user/zh/devices.txt",
        ] {
            assert_eq!(split_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_page_needs_a_title_and_a_summary() {
        let page = parse(
            "zh",
            "---\ntitle: 加设备\nsummary: 再加一台\nrequires_core: 0.9.0\n---\n\n正文\n",
        )
        .unwrap();
        assert_eq!(page.title, "加设备");
        assert_eq!(page.requires_core.as_deref(), Some("0.9.0"));
        assert_eq!(page.body, "正文\n");
        assert!(parse("zh", "没有开头").is_err());
        assert!(parse("zh", "---\ntitle: 只有标题\n---\n正文").is_err());
    }
}
