//! 插件的图片在子进程里处理：起真的 `wonton-core image …`，看结果、报错和时限（插件规范的“图标和截图”）。

use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, Instant};

use image::{ImageFormat, Rgba, RgbaImage};
use wonton_core::plugin::check::check;
use wonton_core::plugin::image_worker::{ImageError, ImageWorker};
use wonton_core::plugin::images::IconFormat;

const WONTON_CORE: &str = env!("CARGO_BIN_EXE_wonton-core");

/// 这个文件里的测试一个一个跑，找子进程时只找得到自己起的。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" fill="#22aa66"/></svg>"##;

/// 很细的虚线：每条路径要画几十万段，32 KB 的图标要渲染好几分钟。预检挡不住，靠子进程的时限。
fn thin_dashes() -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><defs><path id="p" d="M0 8 L16 8" stroke="#000" stroke-dasharray="0.00002"/></defs>{}</svg>"##,
        r##"<use href="#p"/>"##.repeat(2000)
    )
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    RgbaImage::from_pixel(width, height, Rgba([20, 120, 220, 255]))
        .write_to(&mut out, ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

/// 说话按主人语言（zh-Hans）：断言里的中文说明来自子进程和核心两边。
fn zh_worker(timeout: Option<Duration>) -> (ImageWorker, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let langs = dir.path().join("core/plugins/store");
    std::fs::create_dir_all(&langs).unwrap();
    std::fs::write(
        langs.join("people-languages.json"),
        r#"{"owner": "zh-Hans"}"#,
    )
    .unwrap();
    let mut worker = ImageWorker::new(WONTON_CORE);
    if let Some(t) = timeout {
        worker = worker.with_timeout(t);
    }
    worker.set_languages(
        dir.path().join("core/plugins/store/people-languages.json"),
        "owner".to_owned(),
    );
    (worker, dir)
}

fn refused<T: std::fmt::Debug>(result: Result<T, ImageError>) -> String {
    match result {
        Err(ImageError::Refused(reason)) => reason,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn icons_and_screenshots_come_back_reencoded() {
    let _serial = SERIAL.lock().await;
    let (worker, _langs) = zh_worker(None);
    let icon = worker.icon(IconFormat::Svg, ICON.as_bytes()).await.unwrap();
    for (png, edge) in [(&icon.small, 192), (&icon.large, 512)] {
        let image = image::load_from_memory(png).unwrap();
        assert_eq!((image.width(), image.height()), (edge, edge));
    }
    assert!(!icon.transparent_corner);

    let shot = worker.screenshot(&png(1600, 1000)).await.unwrap();
    assert_eq!((shot.width, shot.height), (1600, 1000));
    assert_eq!(image::load_from_memory(&shot.jpeg).unwrap().width(), 1280);
}

#[tokio::test]
async fn images_that_break_the_rules_are_refused_with_the_reason() {
    let _serial = SERIAL.lock().await;
    let (worker, _langs) = zh_worker(None);
    let wide = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 24"/>"#;
    let reason = refused(worker.icon(IconFormat::Svg, wide.as_bytes()).await);
    assert!(reason.contains("正方形"), "{reason}");
    // 子进程读到上限就不读了，核心那边写不进去也不卡住。
    let reason = refused(worker.screenshot(&vec![0; 3 << 20]).await);
    assert!(reason.contains("超过 1024 KiB"), "{reason}");
}

#[tokio::test]
async fn slow_images_are_stopped_at_the_deadline_and_the_process_is_gone() {
    let _serial = SERIAL.lock().await;
    let (worker, _langs) = zh_worker(Some(Duration::from_secs(2)));
    let started = Instant::now();
    let task = tokio::spawn(async move {
        let svg = thin_dashes();
        worker.icon(IconFormat::Svg, svg.as_bytes()).await
    });
    let workers = if cfg!(target_os = "linux") {
        running_children().await
    } else {
        Vec::new()
    };
    let reason = refused(task.await.unwrap());
    assert!(reason.contains("处理超过 2 秒"), "{reason}");
    assert!(started.elapsed() < Duration::from_secs(5));
    for pid in workers {
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "子进程 {pid} 还在"
        );
    }
}

#[tokio::test]
async fn plugin_check_processes_images_in_the_same_limited_process() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let compose = "services:\n  web:\n    image: nginx:1.27\nx-wonton:\n  id: app.test\n  version: 1.0.0\n  \
                   kind: app\n  spec: \"0.2\"\n  title: {zh: 测试, en: Test}\n  description: {zh: 测试, en: Test}\n  icon: icon.svg\n  \
                   permissions: []\n";
    std::fs::write(dir.path().join("docker-compose.yml"), compose).unwrap();
    std::fs::write(dir.path().join("icon.svg"), thin_dashes()).unwrap();
    let (worker, _langs_dir) = zh_worker(Some(Duration::from_secs(2)));
    let report = check(dir.path(), false, &worker).await.unwrap();
    let [problem] = &report.problems[..] else {
        panic!("{report:?}")
    };
    assert!(
        problem.starts_with("icon icon.svg: 处理超过 2 秒"),
        "{problem}"
    );
}

/// 等这个测试进程起的子进程出现，返回它们的进程号（Linux 上按 /proc 找）。
async fn running_children() -> Vec<u32> {
    let me = std::process::id();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let found: Vec<u32> = std::fs::read_dir("/proc")
            .unwrap()
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
            .filter(|pid| parent_of(*pid) == Some(me))
            .collect();
        if !found.is_empty() || Instant::now() > deadline {
            assert_eq!(found.len(), 1, "子进程：{found:?}");
            return found;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// `/proc/<pid>/stat` 的第 4 个字段。第 2 个字段是括号里的命令名，可能有空格，从最后一个 `)` 往后数。
fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}
