//! 工具结果里的二进制块：超过单块上限或 base64 不合法的块整块换成说明，别的块原样过（插件规范的“工具结果里的文件”）。
//! 不经文件登记（没有 resource_link），只要总线和假插件的结果就够了。

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rmcp::model::{CallToolResult, ContentBlock, ResourceContents};
use serde_json::json;

use super::tests::{Setup, setup};
use crate::plugin::docker::Engine as Docker;

const FIND: &str = "app-example-notes__find";

struct Bench {
    s: Setup,
}

async fn bench() -> Bench {
    Bench {
        s: setup(Docker::unreachable_for_test(), Duration::from_secs(60)).await,
    }
}

impl Bench {
    async fn find(&self, blocks: Vec<ContentBlock>) -> CallToolResult {
        *self.s.notes.found.lock().unwrap() = CallToolResult::success(blocks);
        self.s.call(FIND, json!({})).await
    }
}

/// 结果里每个文字块的内容，按顺序。
fn texts(result: &CallToolResult) -> Vec<&str> {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text())
        .map(|text| text.text.as_str())
        .collect()
}

/// 上限按解码后的字节算：正好在上限上的块原样过，小的更不用说。
#[tokio::test]
async fn binary_blocks_within_the_limit_pass_through() {
    let b = bench().await;
    let small = STANDARD.encode(vec![0u8; 2048]);
    let boundary = STANDARD.encode(vec![0u8; 1 << 20]);
    let result = b
        .find(vec![
            ContentBlock::image(small.clone(), "image/jpeg"),
            ContentBlock::image(boundary.clone(), "image/jpeg"),
        ])
        .await;
    let kept: Vec<&str> = result
        .content
        .iter()
        .filter_map(|block| block.as_image())
        .map(|image| image.data.as_str())
        .collect();
    assert_eq!(
        kept,
        vec![small.as_str(), boundary.as_str()],
        "上限以内的原样过：{result:?}"
    );
}

/// 超过单块上限的 image、audio、blob 各整块换成说明（是哪种块、解码后多大、上限多少、大文件要交引用），
/// 同一结果里别的块原样留着。
#[tokio::test]
async fn oversized_binary_blocks_become_an_explanation_and_other_blocks_survive() {
    let b = bench().await;
    let big = STANDARD.encode(vec![0u8; 2 << 20]);
    let result = b
        .find(vec![
            ContentBlock::text("找到 2 张"),
            ContentBlock::image(big.clone(), "image/jpeg"),
            ContentBlock::audio(big.clone(), "audio/wav"),
            ContentBlock::resource(ResourceContents::BlobResourceContents {
                uri: "notes://big.bin".to_owned(),
                mime_type: Some("application/octet-stream".to_owned()),
                blob: big.clone(),
                meta: None,
            }),
            ContentBlock::text("完"),
        ])
        .await;
    let texts = texts(&result);
    assert_eq!(result.content.len(), 5, "{texts:?}");
    assert_eq!(texts.first(), Some(&"找到 2 张"), "{texts:?}");
    assert_eq!(texts.last(), Some(&"完"), "{texts:?}");
    let explanations: Vec<&str> = texts
        .iter()
        .copied()
        .filter(|t| t.contains("over the single-block limit"))
        .collect();
    assert_eq!(explanations.len(), 3, "{texts:?}");
    let all = explanations.join("\n");
    for wanted in [
        "image block",
        "audio block",
        "blob in a resource block",
        "decodes to 2.0 MiB",
        "limit of 1.0 MiB",
        "resource_link",
    ] {
        assert!(all.contains(wanted), "{wanted}：{all}");
    }
    assert!(
        result.content.iter().all(|block| block.as_image().is_none()
            && block.as_audio().is_none()
            && block.as_resource().is_none()),
        "{texts:?}"
    );
    let wire = serde_json::to_string(&result).unwrap();
    assert!(!wire.contains(&big), "超限的 base64 不能交给 Agent");
}

/// base64 不合法的块也整块换成说明，不把废数据交给 Agent。
#[tokio::test]
async fn blocks_that_are_not_valid_base64_become_an_explanation() {
    let b = bench().await;
    let result = b
        .find(vec![
            ContentBlock::image("abc".to_owned(), "image/jpeg"),
            ContentBlock::image("abc!".to_owned(), "image/jpeg"),
        ])
        .await;
    let texts = texts(&result);
    assert_eq!(texts.len(), 2, "{texts:?}");
    for explanation in &texts {
        assert!(explanation.contains("not valid base64"), "{explanation}");
    }
    let wire = serde_json::to_string(&result).unwrap();
    assert!(!wire.contains("\"data\""), "没有 base64 数据：{wire}");
}
