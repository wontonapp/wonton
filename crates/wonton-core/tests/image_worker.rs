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
    worker_for(Path::new(WONTON_CORE), timeout)
}

/// 用 `program` 处理图片，其余同 [`zh_worker`]。
fn worker_for(program: &Path, timeout: Option<Duration>) -> (ImageWorker, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let langs = dir.path().join("core/plugins/store");
    std::fs::create_dir_all(&langs).unwrap();
    std::fs::write(
        langs.join("people-languages.json"),
        r#"{"owner": "zh-Hans"}"#,
    )
    .unwrap();
    let mut worker = ImageWorker::new(program);
    if let Some(t) = timeout {
        worker = worker.with_timeout(t);
    }
    worker.set_languages(
        dir.path().join("core/plugins/store/people-languages.json"),
        "owner".to_owned(),
    );
    (worker, dir)
}

/// 一个先睡一会儿再转给真核心的壳：任务子进程会在这段时间里带着核心那边套接字的 fd 副本活着。
fn slow_core(dir: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let wrapper = dir.join("slow-core");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nif [ \"$1\" = image-host ]; then exec '{WONTON_CORE}' \"$@\"; fi\nsleep 0.5\nexec '{WONTON_CORE}' \"$@\"\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    wrapper
}

fn refused<T: std::fmt::Debug>(result: Result<T, ImageError>) -> String {
    match result {
        Err(ImageError::Refused(reason)) => reason,
        other => panic!("{other:?}"),
    }
}

fn timed_out<T: std::fmt::Debug>(result: Result<T, ImageError>) -> String {
    match result {
        Err(ImageError::TimedOut(reason)) => reason,
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
    worker.start().await.unwrap();
    let helper = helper_pid().await;
    let worker = std::sync::Arc::new(worker);
    let started = Instant::now();
    let task = tokio::spawn({
        let worker = worker.clone();
        async move {
            let svg = thin_dashes();
            worker.icon(IconFormat::Svg, svg.as_bytes()).await
        }
    });
    let images = if cfg!(target_os = "linux") {
        until_children(helper).await
    } else {
        Vec::new()
    };
    let reason = timed_out(task.await.unwrap());
    assert!(reason.contains("处理超过 2 秒"), "{reason}");
    assert!(started.elapsed() < Duration::from_secs(5));
    for pid in images {
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "处理图片的子进程 {pid} 还在"
        );
    }
    // 只杀了子进程，助手还在，能接着用：下一次不用等重起。
    if cfg!(target_os = "linux") {
        assert!(
            Path::new(&format!("/proc/{helper}")).exists(),
            "助手也跟着没了"
        );
    }
    worker.icon(IconFormat::Svg, ICON.as_bytes()).await.unwrap();
}

/// 图片任务在子进程里跑着时，核心在原端口重绑 UDP 不能被子进程手里的 fd 副本挡住（#66）。
///
/// 核心起子进程时，fork 到 exec 之间子进程手里有核心所有 fd 的副本；正好在这时重绑就 `AddrInUse`。
/// 真隧道的套接字是 CLOEXEC，exec 时子进程手里就没了，窗口只有 fork 到 exec 那一瞬。这里把
/// CLOEXEC 清掉，让处理图片的子进程整个任务期间都拿着端口：核心一 fork，撞不撞得上就不靠调度运气。
#[tokio::test]
async fn rebinding_the_tunnel_port_works_while_an_image_is_processed() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let wrapper = slow_core(dir.path());
    let (worker, _langs) = worker_for(&wrapper, Some(Duration::from_secs(5)));
    worker.start().await.unwrap();

    let socket = tokio::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = socket.local_addr().unwrap().port();
    let socket = socket.into_std().unwrap();
    rustix::io::fcntl_setfd(
        std::os::fd::AsFd::as_fd(&socket),
        rustix::io::FdFlags::empty(),
    )
    .unwrap();
    let socket = tokio::net::UdpSocket::from_std(socket).unwrap();
    let task = tokio::spawn({
        let worker = std::sync::Arc::new(worker);
        async move { worker.icon(IconFormat::Svg, ICON.as_bytes()).await }
    });
    // 壳要睡半秒；到这时处理图片的子进程已经起来了，把端口放掉再绑回来。
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(socket);
    tokio::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .expect("端口被图片子进程占住了");
    task.await.unwrap().unwrap();
}

/// 任务 future 被丢掉（`files` 的缩略图超时就是这样取消的）时，处理图片的子进程跟着停，
/// 助手还能接着用，不会攒下一堆白跑的子进程。
#[tokio::test]
async fn a_cancelled_task_kills_its_child() {
    let _serial = SERIAL.lock().await;
    let (worker, _langs) = zh_worker(Some(Duration::from_secs(10)));
    worker.start().await.unwrap();
    let helper = helper_pid().await;
    let worker = std::sync::Arc::new(worker);
    let task = tokio::spawn({
        let worker = worker.clone();
        async move {
            let svg = thin_dashes();
            worker.icon(IconFormat::Svg, svg.as_bytes()).await
        }
    });
    let images = if cfg!(target_os = "linux") {
        until_children(helper).await
    } else {
        tokio::time::sleep(Duration::from_millis(200)).await;
        Vec::new()
    };
    task.abort();
    task.await.unwrap_err();
    if cfg!(target_os = "linux") {
        until_gone(&images).await;
        assert!(
            Path::new(&format!("/proc/{helper}")).exists(),
            "助手也跟着没了"
        );
    }
    worker.icon(IconFormat::Svg, ICON.as_bytes()).await.unwrap();
}

/// 助手被杀掉后，下一个任务会再起一个，结果照常。
#[tokio::test]
async fn a_dead_helper_is_started_again() {
    let _serial = SERIAL.lock().await;
    let (worker, _langs) = zh_worker(None);
    worker.start().await.unwrap();
    let helper = helper_pid().await;
    if cfg!(target_os = "linux") {
        rustix::process::kill_process(
            rustix::process::Pid::from_raw(helper as i32).unwrap(),
            rustix::process::Signal::KILL,
        )
        .unwrap();
        // 等核心发现它没了（回收后 /proc 里就查不到了），下一个任务才算“助手退出之后的”。
        let deadline = Instant::now() + Duration::from_secs(2);
        while Path::new(&format!("/proc/{helper}")).exists() {
            assert!(Instant::now() < deadline, "助手 {helper} 一直没被回收");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    let icon = worker
        .icon(IconFormat::Svg, ICON.as_bytes())
        .await
        .expect("助手退出后的下一个任务会重起助手");
    assert_eq!(image::load_from_memory(&icon.small).unwrap().width(), 192);
    if cfg!(target_os = "linux") {
        assert_ne!(helper_pid().await, helper);
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

/// 助手的进程号：它是测试进程唯一的子进程（[`SERIAL`] 保证别的测试没在起进程）。
async fn helper_pid() -> u32 {
    if !cfg!(target_os = "linux") {
        tokio::time::sleep(Duration::from_millis(200)).await;
        return 0;
    }
    until_children(std::process::id()).await[0]
}

/// 等 `parent` 的子进程出现，返回它们的进程号（Linux 上按 /proc 找）。
async fn until_children(parent: u32) -> Vec<u32> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let found = children_of(parent);
        if !found.is_empty() || Instant::now() > deadline {
            assert_eq!(found.len(), 1, "子进程：{found:?}");
            return found;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// 等这些进程从 /proc 里消失。
async fn until_gone(pids: &[u32]) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while pids
        .iter()
        .any(|pid| Path::new(&format!("/proc/{pid}")).exists())
    {
        assert!(Instant::now() < deadline, "子进程 {pids:?} 还活着");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn children_of(parent: u32) -> Vec<u32> {
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .filter(|pid| parent_of(*pid) == Some(parent))
        .collect()
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

/// 每项是一页内容流的最小 PDF：交叉引用表按真实偏移写，字体对象编号按页数算。
fn pdf(pages: &[&str]) -> Vec<u8> {
    let font = 3 + pages.len() * 2;
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

#[tokio::test]
async fn pdf_text_comes_back_through_a_real_child() {
    let _serial = SERIAL.lock().await;
    let (worker, _langs) = zh_worker(None);
    let pages = [
        "BT /F1 12 Tf 72 712 Td (MARKER-ONE) Tj ET",
        "BT /F1 12 Tf 72 712 Td (MARKER-TWO) Tj ET",
    ];
    let got = worker.pdf_text(&pdf(&pages)).await.unwrap();
    assert!(got.text.contains("MARKER-ONE"), "{}", got.text);
    assert!(got.text.contains("MARKER-TWO"), "{}", got.text);
    assert_eq!((got.pages_read, got.pages_total), (2, 2));
    assert!(!got.truncated);

    // 没有文字层：子进程按拒绝回，原因里带页数。
    let scan = worker.pdf_text(&pdf(&["q 10 0 0 10 0 0 cm Q"])).await;
    let reason = refused(scan);
    assert!(reason.contains("1 页"), "{reason}");
}
