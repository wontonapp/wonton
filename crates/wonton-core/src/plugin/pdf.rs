//! PDF 的文字层（ADR-0104）：`wonton__view_file` 的 PDF 分支在图片助手子进程里调这里，panic 和死循环关在那个进程里。
//!
//! `pdf-extract` 按页抽，遇到第一页解析不了就停：一页都没有算整份解析不了（加密、结构坏），抽出页来但一个字都没有
//! 算没有文字层（扫描件、整页图）。

use super::languages::Languages;
use crate::files::MAX_TEXT_PREVIEW_BYTES;
use crate::l10n;

/// 抽出来的文字和页数。
#[derive(Debug)]
pub struct Extracted {
    pub text: String,
    /// 写进 `text` 的最后一页是哪一页（从 1 数起，空白页也算读过）。
    pub pages_read: u32,
    /// 抽出来一共有多少页。
    pub pages_total: u32,
    /// 文字到了上限被截断：后面的页没读，或者最后一页只留了前半。单页超限时 `pages_read` 等于 `pages_total`。
    pub truncated: bool,
}

/// 按页抽文字，凑满 [`MAX_TEXT_PREVIEW_BYTES`] 字节就停，截断点落在字符边界上。空白页不进文字，但算读过。
pub fn extract(data: &[u8], langs: &Languages) -> Result<Extracted, String> {
    let pages = pdf_extract::extract_text_from_mem_by_pages(data)
        .map_err(|_| l10n::text(langs, "files-pdf-unreadable", &[]))?;
    let pages_total = u32::try_from(pages.len()).unwrap_or(u32::MAX);
    if pages.is_empty() {
        return Err(l10n::text(langs, "files-pdf-unreadable", &[]));
    }
    if pages.iter().all(|page| page.trim().is_empty()) {
        return Err(l10n::text(
            langs,
            "files-pdf-no-text",
            &[("pages", pages_total.into())],
        ));
    }
    let mut text = String::new();
    let mut pages_read = 0;
    for (index, page) in pages.iter().enumerate() {
        if text.len() >= MAX_TEXT_PREVIEW_BYTES {
            break;
        }
        pages_read = index as u32 + 1;
        let page = page.trim_end();
        if page.trim().is_empty() {
            continue;
        }
        if !text.is_empty() {
            // 页之间空一行，两页的字不会粘成一句。
            text.push_str("\n\n");
        }
        text.push_str(page);
    }
    let truncated = pages_read < pages_total || text.len() > MAX_TEXT_PREVIEW_BYTES;
    if text.len() > MAX_TEXT_PREVIEW_BYTES {
        text.truncate(char_boundary(&text, MAX_TEXT_PREVIEW_BYTES));
    }
    Ok(Extracted {
        text,
        pages_read,
        pages_total,
        truncated,
    })
}

/// 不超过 `limit` 的最近一个字符边界。
fn char_boundary(text: &str, limit: usize) -> usize {
    let mut end = limit.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[cfg(test)]
pub(crate) mod test_support {
    /// 一页的文字内容流。
    pub(crate) fn page(text: &str) -> String {
        format!("BT /F1 12 Tf 72 712 Td ({text}) Tj ET")
    }

    /// 每项是一页内容流的最小 PDF，交叉引用表按真实偏移写。
    pub(crate) fn pdf(pages: &[&str]) -> Vec<u8> {
        let mut objects: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                (0..pages.len())
                    .map(|i| format!("{} 0 R", 3 + i * 2))
                    .collect::<Vec<_>>()
                    .join(" "),
                pages.len()
            )
            .into_bytes(),
        ];
        let font = 3 + pages.len() * 2;
        for (index, content) in pages.iter().enumerate() {
            objects.push(
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R /Resources << /Font << /F1 {font} 0 R >> >> >>",
                    4 + index * 2
                )
                .into_bytes(),
            );
            objects.push(
                format!(
                    "<< /Length {} >>\nstream\n{content}\nendstream",
                    content.len()
                )
                .into_bytes(),
            );
        }
        objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (number, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", number + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in &offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::test_support::{page as text, pdf};

    fn extract_pages(pages: &[&str]) -> Extracted {
        extract(&pdf(pages), &Languages::default()).expect("应该能抽")
    }

    #[test]
    fn reads_each_page_and_counts_them() {
        let got = extract_pages(&[&text("MARKER-ONE"), &text("MARKER-TWO")]);
        assert!(got.text.contains("MARKER-ONE"), "{}", got.text);
        assert!(got.text.contains("MARKER-TWO"), "{}", got.text);
        assert_eq!((got.pages_read, got.pages_total), (2, 2));
    }

    #[test]
    fn stops_at_the_limit_and_says_which_page_it_reached() {
        let body = "x".repeat(MAX_TEXT_PREVIEW_BYTES);
        let got = extract_pages(&[&text(&body), &text("MARKER-TWO")]);
        assert!(got.text.len() <= MAX_TEXT_PREVIEW_BYTES);
        assert!(!got.text.contains("MARKER-TWO"));
        assert!(got.pages_read < got.pages_total, "{got:?}");
        assert!(got.truncated);
        assert_eq!(got.pages_total, 2);
    }

    #[test]
    fn one_huge_page_is_truncated_too() {
        let body = "x".repeat(MAX_TEXT_PREVIEW_BYTES + 100);
        let got = extract_pages(&[&text(&body)]);
        assert_eq!((got.pages_read, got.pages_total), (1, 1));
        assert!(got.truncated, "单页超限也是截断");
        assert!(got.text.len() <= MAX_TEXT_PREVIEW_BYTES);
    }

    #[test]
    fn a_pdf_without_text_has_its_own_error() {
        let no_text = pdf(&["q 100 0 0 100 100 100 cm Q", ""]);
        let error = extract(&no_text, &Languages::default()).expect_err("没有文字层");
        assert!(error.contains("no text layer"), "{error}");
        assert!(error.contains("2 pages"), "{error}");
    }

    #[test]
    fn broken_bytes_do_not_parse() {
        let error = extract(b"%PDF-1.4\nnothing", &Languages::default()).expect_err("坏的");
        assert!(error.contains("Can't parse this PDF"), "{error}");
    }

    #[test]
    fn the_cut_falls_on_a_character_boundary() {
        // 「字」占三个字节，从第 10 个字节起放不下整个字符。
        let body = format!("{}字", "x".repeat(9));
        assert_eq!(char_boundary(&body, 10), 9);
        assert_eq!(char_boundary(&body, 12), 12);
        assert_eq!(char_boundary(&body, 99), body.len());
    }
}
