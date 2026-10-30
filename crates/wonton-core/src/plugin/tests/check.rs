//! `wonton-core plugin check` 和 `pack`：插件作者发布前看到的检查结果，开发时上传的插件包。

use std::fs;
use std::io::Cursor;
use std::path::Path;

use image::{ImageFormat, Rgba, RgbaImage};

use super::super::check::{Report, check};
use super::super::image_worker::ImageWorker;
use super::super::package::PluginPackage;
use super::super::package_files::read_archive;
use super::package;
use super::views::ICON;

const UNPINNED: &str = "  web:\n    image: nginx:1.27";

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    RgbaImage::from_pixel(width, height, Rgba([20, 120, 220, 255]))
        .write_to(&mut out, ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

fn write(dir: &Path, path: &str, data: &[u8]) {
    let path = dir.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

fn compose(services: &str, extra: &str) -> String {
    package(
        services,
        &format!(
            "  spec: \"0.2\"\n  icon: icon.svg\n  screenshots: [shots/1.png]\n  permissions: []\n{extra}"
        ),
    )
}

/// 单元测试里在本进程处理图片；起子进程的检查在 `tests/image_worker.rs`。
async fn run(dir: &Path, official: bool) -> Result<Report, String> {
    check(dir, official, &ImageWorker::in_process()).await
}

/// 插件仓库：插件包（compose 和图标），加上不属于插件包的 README 和截图。
fn repo(compose: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "docker-compose.yml", compose.as_bytes());
    write(dir.path(), "icon.svg", ICON.as_bytes());
    write(dir.path(), "shots/1.png", &png(1600, 1000));
    write(dir.path(), "README.md", b"# Test\n");
    dir
}

#[tokio::test]
async fn lists_the_package_and_warns_about_what_the_store_would_flag() {
    let dir = repo(&compose(UNPINNED, ""));
    let report = run(dir.path(), false).await.unwrap();
    assert_eq!(
        (report.id.as_str(), report.version.as_str()),
        ("app.test", "1.0.0")
    );
    let files: Vec<&str> = report.files.iter().map(|(path, _)| path.as_str()).collect();
    assert_eq!(files, ["docker-compose.yml", "icon.svg"]);
    assert_eq!(
        report.digest,
        PluginPackage::load_dir(dir.path()).unwrap().digest()
    );
    assert!(report.problems.is_empty(), "{report:?}");
    let [unpinned, unhealthy] = &report.warnings[..] else {
        panic!("{report:?}")
    };
    assert!(unpinned.contains("nginx:1.27 没有按摘要固定"), "{unpinned}");
    assert!(unhealthy.contains("没有健康检查"), "{unhealthy}");
}

#[tokio::test]
async fn the_official_index_turns_warnings_into_problems_and_wants_store_info() {
    let report = run(repo(&compose(UNPINNED, "")).path(), true)
        .await
        .unwrap();
    for expected in [
        "没有按摘要固定",
        "没有健康检查",
        "官方索引要求写 category",
        "官方索引要求写 license",
        "官方索引要求写 author",
    ] {
        assert!(
            report.problems.iter().any(|p| p.contains(expected)),
            "{expected}：{report:?}"
        );
    }
    let pinned = format!(
        "  web:\n    image: nginx:1.27@sha256:{}\n    healthcheck:\n      test: [CMD, nginx, -t]",
        "a".repeat(64)
    );
    let complete = "  category: tools\n  license: MIT\n  author: {name: Example}";
    let report = run(repo(&compose(&pinned, complete)).path(), true)
        .await
        .unwrap();
    assert!(
        report.problems.is_empty() && report.warnings.is_empty(),
        "{report:?}"
    );
}

#[tokio::test]
async fn icons_and_screenshots_are_processed_like_the_store_does() {
    let shots = compose(UNPINNED, "").replace(
        "[shots/1.png]",
        "[shots/1.png, shots/gif.png, shots/tall.png]",
    );
    let dir = repo(&shots);
    write(dir.path(), "shots/gif.png", b"GIF89a");
    write(dir.path(), "shots/tall.png", &png(600, 900));
    let wide = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 24"/>"#;
    write(dir.path(), "icon.svg", wide.as_bytes());
    let report = run(dir.path(), false).await.unwrap();
    let [icon, gif] = &report.problems[..] else {
        panic!("{report:?}")
    };
    assert!(
        icon.starts_with("图标 icon.svg：") && icon.contains("正方形"),
        "{icon}"
    );
    assert!(
        gif.starts_with("截图 shots/gif.png：") && gif.contains("PNG 或 JPEG"),
        "{gif}"
    );
    for expected in ["shots/tall.png 是 600×900", "横屏和竖屏"] {
        assert!(
            report.warnings.iter().any(|w| w.contains(expected)),
            "{expected}：{report:?}"
        );
    }

    let round = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2 2"><circle cx="1" cy="1" r="1" fill="#000"/></svg>"##;
    write(dir.path(), "icon.svg", round.as_bytes());
    let report = run(dir.path(), false).await.unwrap();
    assert!(
        report.warnings.iter().any(|w| w.contains("角是透明的")),
        "{report:?}"
    );
}

#[tokio::test]
async fn a_package_that_does_not_parse_is_reported_like_at_install() {
    let dir = tempfile::tempdir().unwrap();
    let err = run(dir.path(), false).await.unwrap_err();
    assert!(err.contains("docker-compose.yml：不存在"), "{err}");
    write(
        dir.path(),
        "docker-compose.yml",
        compose(UNPINNED, "").as_bytes(),
    );
    let err = run(dir.path(), false).await.unwrap_err();
    assert!(err.contains("icon.svg"), "{err}");
}

#[test]
fn packing_keeps_only_the_package_and_installs_the_same_digest() {
    let dir = repo(&compose(UNPINNED, ""));
    let package = PluginPackage::load_dir(dir.path()).unwrap();
    let archive = package.archive();
    let files = read_archive(&archive).unwrap();
    let paths: Vec<&str> = files.keys().map(String::as_str).collect();
    assert_eq!(paths, ["docker-compose.yml", "icon.svg"]);
    let uploaded = PluginPackage::from_archive(&archive).unwrap();
    assert_eq!(uploaded.digest(), package.digest());
}
