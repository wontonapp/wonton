//! 热加载不新起连接也不断掉正在下载的。

use super::*;

/// 经连接器下载 `/slow`：`-Sv` 的 stderr 留给外面，返回还在下载的子进程。
async fn download(case: &Case, roots: &Path, output: &Path) -> std::process::Child {
    let connector_ip = &case.connector_ip;
    let through = |extra: &[&str]| {
        let resolve = format!("probe.{ZONE}:8443:{connector_ip}");
        let url = format!("https://probe.{ZONE}:8443/slow");
        let mut args = vec![
            "-sS",
            "--noproxy",
            "*",
            "--connect-timeout",
            "5",
            "--max-time",
            "20",
            "--cacert",
            roots.to_str().unwrap(),
            "--resolve",
            resolve.as_str(),
            url.as_str(),
        ];
        args.extend_from_slice(extra);
        let mut command = Command::new("curl");
        command.args(&args);
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap()
    };
    // 连接器的 DNS 解析是懒的（init-addr libc,none）：先打通一次，别把解析慢记成重载丢包。
    for _ in 0..60 {
        if through(&["-o", "/dev/null", "--max-time", "3"])
            .wait()
            .unwrap()
            .success()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let child = through(&[
        "-Sv",
        "-o",
        output.to_str().unwrap(),
        "-w",
        "%{size_download}",
    ]);

    // 等第一个字节真的落盘再重载：没被 accept 的连接在监听套接字换手时不会跟着走，
    // 固定 sleep 会让“下载中”有时其实还没开始。
    for _ in 0..100 {
        if std::fs::metadata(output).map(|m| m.len()).unwrap_or(0) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    child
}

pub(super) async fn check(case: &Case) {
    tokio::time::sleep(Duration::from_millis(800)).await;
    let roots = case.dir.path().join("roots.pem");
    let roots_text = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap()
        .get(format!("https://{}:15000/roots/0", case.pebble_ip))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    std::fs::write(&roots, roots_text).unwrap();
    let output = case.dir.path().join("download");
    let mut child = download(case, &roots, &output).await;
    let (_, status) = docker(&["inspect", "-f", "{{.State.Status}}", &case.entry]);
    if status.trim() != "running" {
        let (_, logs) = docker(&["logs", "--tail", "30", &case.entry]);
        panic!("入口在热加载前已退出：\n{logs}");
    }
    crate::entry_containers::reload_haproxy(&format!("{}:8404", case.entry_ip))
        .await
        .unwrap();
    let result = child.wait().unwrap();
    if !result.success() {
        let (_, elogs) = docker(&["logs", "--timestamps", "--tail", "40", &case.entry]);
        let (_, clogs) = docker(&["logs", "--timestamps", "--tail", "20", &case.connector]);
        let (_, info) = docker(&[
            "inspect",
            "-f",
            "{{.State.Status}} {{.RestartCount}}",
            &case.entry,
        ]);
        panic!(
            "热加载后下载失败: {info}
entry:
{elogs}
connector:
{clogs}"
        );
    }
    let body = std::fs::read_to_string(&output).unwrap();
    assert_eq!(body.lines().count(), 10, "连接被截断：{body:?}");
}
