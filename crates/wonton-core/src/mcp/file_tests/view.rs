//! view_file 工具和预览：图片、PDF、视频、文本截断和预览预算。

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::json;
use wonton_protocol::{FileFailure, FileId};

use super::helpers::{
    Bench, bench, ids, images, jpeg, plugin_resource, share_link, starts_with_kind,
};
use crate::files::MAX_TEXT_PREVIEW_BYTES;
use crate::files::test_support::Fixture;
use crate::mcp::file_results::human_size;
use crate::mcp::file_tools::definitions;
use crate::mcp::tests::{failed, setup, text};
use crate::plugin::docker::Engine as Docker;
use crate::plugin::images::MAX_THUMBNAIL_INPUT;
use crate::plugin::pdf::test_support::{page as pdf_page, pdf as pdf_bytes};

#[tokio::test]
async fn the_first_four_images_get_previews_no_larger_than_1024() {
    // 预览期限是给产品用的 10 秒（files/mod.rs 的 PREVIEW_BUDGET）。CI 是 debug 构建，慢机器上一个切片的
    // 测试挤在一起解码 1600 像素的图偶尔超过它（2 核机器上实测），这条测的是「前四张里能看的都有预览、
    // 不超过 1024」，跟单张预算的生死赛跑无关，所以先放宽预算，断言就和机器快慢脱钩。
    let mut f = Fixture::new();
    std::sync::Arc::get_mut(&mut f.files)
        .unwrap()
        .preview_budget = Duration::from_secs(60);
    let s = setup(Docker::unreachable_for_test(), Duration::from_secs(60)).await;
    s.bus.attach_files(f.files.clone());
    let b = Bench { s, f };
    let mut links = Vec::new();
    for i in 0..6 {
        let path = format!("photos/{i}.jpg");
        b.f.put(&path, &jpeg(1600));
        links.push(share_link(&path));
    }
    b.f.put("photos/broken.jpg", b"not an image");
    links.insert(0, share_link("photos/broken.jpg"));
    let result = b.find(links).await;
    assert_eq!(ids(&result).len(), 7, "解不了的文字照常");
    let previews = images(&result);
    assert_eq!(previews, [(1024, 1024); 3], "第一张解不了，也占一个名额");
}

#[tokio::test]
async fn view_file_returns_a_preview_and_says_why_it_cannot() {
    let b = bench().await;
    let photo = b.found("photos/a.jpg", &jpeg(1600)).await;
    let result = b.view(&photo).await;
    assert!(!failed(&result), "{}", text(&result));
    assert_eq!(images(&result), [(1024, 1024)]);

    // 二进制（含 NUL）按内容判出来，PDF 这类读不了内容。
    let pdf = b.found("docs/a.pdf", b"%PDF-1.7\n%binary\0\0 more\0").await;
    starts_with_kind(&b.view(&pdf).await, FileFailure::NotViewable);

    // 文本按内容认出来，MIME 和扩展名都不参与判断。
    let note = b
        .found("docs/notes.dat", "暗号：WONTON-M1V3-7391\n".as_bytes())
        .await;
    let result = b.view(&note).await;
    assert!(!failed(&result), "{}", text(&result));
    assert!(
        text(&result).contains("WONTON-M1V3-7391"),
        "{}",
        text(&result)
    );
    assert!(text(&result).contains("full contents"), "{}", text(&result));

    let unknown: FileId = "a".repeat(26).parse().unwrap();
    starts_with_kind(&b.view(&unknown).await, FileFailure::FileUnknown);
    let garbage =
        b.s.call("wonton__view_file", json!({"file_id": "../../etc/passwd"}))
            .await;
    starts_with_kind(&garbage, FileFailure::FileUnknown);
    let missing = b.s.call("wonton__share_file", json!({})).await;
    starts_with_kind(&missing, FileFailure::InvalidInput);
}

/// 夹具里 16×16、0.5 秒的视频改成 32×16、两分钟：用户视频不受头像的正方和时长限制。
fn rectangle_video() -> Vec<u8> {
    fn at(bytes: &[u8], kind: &[u8]) -> usize {
        bytes
            .windows(kind.len())
            .position(|part| part == kind)
            .unwrap()
    }
    let mut bytes = include_bytes!("../../../tests/fixtures/avatars/h264.mp4").to_vec();
    let mvhd = at(&bytes, b"mvhd");
    bytes[mvhd + 16..mvhd + 20].copy_from_slice(&1000u32.to_be_bytes());
    bytes[mvhd + 20..mvhd + 24].copy_from_slice(&120_000u32.to_be_bytes());
    let avc = at(&bytes, b"avc1\0\0\0\0");
    bytes[avc + 28..avc + 30].copy_from_slice(&32u16.to_be_bytes());
    bytes[avc + 30..avc + 32].copy_from_slice(&16u16.to_be_bytes());
    bytes
}

#[tokio::test]
async fn view_file_reads_pdf_text_and_counts_the_pages() {
    let b = bench().await;
    let pages: Vec<String> = (1..=40)
        .map(|n| pdf_page(&format!("M1V3-PAGE-{n}")))
        .collect();
    let streams: Vec<&str> = pages.iter().map(String::as_str).collect();
    let id = b.found("docs/many.pdf", &pdf_bytes(&streams)).await;
    let result = b.view(&id).await;
    assert!(!failed(&result), "{}", text(&result));
    let answer = text(&result);
    assert!(answer.starts_with("Text contents of"), "{answer}");
    assert!(answer.contains("M1V3-PAGE-1"), "{answer}");
    assert!(answer.contains("M1V3-PAGE-40"), "{answer}");
    assert!(answer.contains("40 pages"), "{answer}");
    assert!(!answer.contains("%PDF-"), "{answer}");
    assert!(images(&result).is_empty());
}

#[tokio::test]
async fn view_file_says_which_page_it_stopped_at() {
    let b = bench().await;
    let head = "h".repeat(MAX_TEXT_PREVIEW_BYTES);
    let pages = [pdf_page(&head), pdf_page("M1V3-TAIL")];
    let streams: Vec<&str> = pages.iter().map(String::as_str).collect();
    let id = b.found("docs/huge-page.pdf", &pdf_bytes(&streams)).await;
    let result = b.view(&id).await;
    assert!(!failed(&result), "{}", text(&result));
    let answer = text(&result);
    assert!(!answer.contains("M1V3-TAIL"), "{answer}");
    assert!(answer.contains("truncated"), "{answer}");
    assert!(answer.contains("page 1 of 2"), "{answer}");
}

#[tokio::test]
async fn view_file_says_a_pdf_has_no_text_layer() {
    let b = bench().await;
    let id = b
        .found(
            "docs/scan.pdf",
            &pdf_bytes(&["q 100 0 0 100 100 100 cm Q", "q 10 0 0 10 0 0 cm Q"]),
        )
        .await;
    let result = b.view(&id).await;
    starts_with_kind(&result, FileFailure::NotViewable);
    assert!(text(&result).contains("2 pages"), "{}", text(&result));
}

#[tokio::test]
async fn view_file_refuses_pdfs_over_the_input_limit() {
    let b = bench().await;
    let mut big = pdf_bytes(&[&pdf_page("M1V3-HEAD")]);
    big.resize(MAX_THUMBNAIL_INPUT + 1, 0);
    let id = b.found("docs/big.pdf", &big).await;
    starts_with_kind(&b.view(&id).await, FileFailure::ResourceTooLarge);
}

#[tokio::test]
async fn view_file_returns_video_metadata_and_no_frames() {
    let b = bench().await;
    let id = b
        .found(
            "clips/a.mp4",
            include_bytes!("../../../tests/fixtures/avatars/h264.mp4"),
        )
        .await;
    let result = b.view(&id).await;
    assert!(!failed(&result), "{}", text(&result));
    assert!(images(&result).is_empty());
    let answer = text(&result);
    assert!(answer.contains("0.5 seconds"), "{answer}");
    assert!(answer.contains("16×16"), "{answer}");
    assert!(answer.contains("frames"), "{answer}");

    let long = b.found("clips/long.mov", &rectangle_video()).await;
    let result = b.view(&long).await;
    assert!(!failed(&result), "{}", text(&result));
    let answer = text(&result);
    assert!(answer.contains("2:00"), "{answer}");
    assert!(answer.contains("32×16"), "{answer}");
}

#[tokio::test]
async fn view_file_says_which_containers_it_cannot_read() {
    let b = bench().await;
    let webm = b.found("clips/a.webm", b"\x1aE\xdf\xa3\0\0\0").await;
    let audio = b.found("clips/a.mp3", b"ID3\0\0\0").await;
    let webm_result = b.view(&webm).await;
    let audio_result = b.view(&audio).await;
    starts_with_kind(&webm_result, FileFailure::NotViewable);
    starts_with_kind(&audio_result, FileFailure::NotViewable);
    assert_ne!(text(&webm_result), text(&audio_result));
}

#[tokio::test]
async fn the_view_tool_no_longer_points_at_share_file() {
    let view = definitions()
        .into_iter()
        .find(|tool| tool.name == "wonton__view_file")
        .expect("应该定义 view_file");
    let description = view.description.unwrap_or_default();
    for word in ["PDF", "MP4", "duration"] {
        assert!(description.contains(word), "{description}");
    }
    assert!(!description.contains("share_file"), "{description}");

    let b = bench().await;
    let photo = b.found("photos/a.jpg", &jpeg(32)).await;
    let result = b.view(&photo).await;
    assert!(!failed(&result), "{}", text(&result));
    assert!(!text(&result).contains("share_file"), "{}", text(&result));
}

#[tokio::test]
async fn view_file_truncates_long_text_and_reports_the_size() {
    let b = bench().await;
    let mut body = b"M1V3-HEAD-MARKER\n".to_vec();
    body.resize(MAX_TEXT_PREVIEW_BYTES + 1000, b'x');
    body.extend_from_slice(b"\nM1V3-TAIL-MARKER\n");
    let id = b.found("docs/long.log", &body).await;
    let result = b.view(&id).await;
    assert!(!failed(&result), "{}", text(&result));
    let answer = text(&result);
    assert!(answer.contains("M1V3-HEAD-MARKER"), "{answer}");
    assert!(!answer.contains("M1V3-TAIL-MARKER"), "{answer}");
    assert!(answer.contains("truncated"), "{answer}");
    assert!(
        answer.contains(&human_size(MAX_TEXT_PREVIEW_BYTES as u64 + 1000 + 22)),
        "写明文件总大小：{answer}"
    );
}

#[tokio::test]
async fn slow_previews_do_not_hold_up_the_result() {
    let mut f = Fixture::new();
    std::sync::Arc::get_mut(&mut f.files)
        .unwrap()
        .preview_budget = Duration::from_millis(200);
    let s = setup(Docker::unreachable_for_test(), Duration::from_secs(60)).await;
    s.bus.attach_files(f.files.clone());
    let b = Bench { s, f };
    b.plugin_runs();
    // 假插件的读一直挂着，放行之前一定还在读：结果回不回来不靠机器快慢。
    b.f.host.reads_hang.store(true, Ordering::SeqCst);
    let mut found = std::pin::pin!(b.find(vec![plugin_resource("notes://a", "a.jpg", None)]));
    tokio::select! {
        _ = b.f.host.reads_stuck.notified() => {}
        result = &mut found => panic!("读还没卡住，结果就回来了：{result:?}"),
    }
    let result = tokio::time::timeout(Duration::from_secs(20), &mut found)
        .await
        .expect("预览到时不做了，结果照常给");
    assert_eq!(ids(&result).len(), 1);
    assert!(images(&result).is_empty());
    assert_eq!(
        b.f.host.reads_running.load(Ordering::SeqCst),
        1,
        "插件收不到取消，还在读，这次读取不能提前还名额"
    );
    // 放行，读真的结束：后台这次读不会一直挂着。
    b.f.host.reads_release.notify_one();
    tokio::time::timeout(Duration::from_secs(20), b.f.host.reads_over.notified())
        .await
        .expect("放行以后读要结束");
    assert_eq!(b.f.host.reads_running.load(Ordering::SeqCst), 0);
}
