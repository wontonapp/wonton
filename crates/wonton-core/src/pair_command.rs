//! `wonton-core pair [--first] [IP:端口…]`：在服务器上向运行中的核心要配对码，终端里显示二维码。在核心的容器里执行
//! （`docker exec`；安装好的主机上是 `sudo wonton pair`）。能执行它就是能用服务器的终端，配对的信任边界就在这里
//! （ADR-0009）。参数是可选的直连地址，不给时由核心按宣告地址和局域网检测算。
//!
//! - 不带 `--first`：发一个主人的码，已经有设备时也能用，设备都丢了时用它找回。
//! - `--first`：显示首次配对码（和局域网状态页上的是同一个），换码时显示新的，第一台设备配上后退出。

use std::fmt::Write;
use std::io::IsTerminal;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, bail};
use hyper::{Method, StatusCode};
use serde::Deserialize;
use wonton_core::config::Config;
use wonton_core::http::HttpClient;
use wonton_core::pairing::terminal_qr;

/// 等第一台设备时多久问一次核心：首次配对码换了新的、或者被扫走了，终端跟着变。
const POLL: Duration = Duration::from_secs(3);

#[derive(Deserialize)]
struct Issued {
    code: String,
    expires_in: u64,
}

pub async fn run(args: Vec<String>) -> anyhow::Result<()> {
    let first = args.iter().any(|arg| arg == "--first");
    let direct_addrs = args
        .iter()
        .filter(|arg| *arg != "--first")
        .map(|addr| {
            addr.parse::<SocketAddr>()
                .with_context(|| format!("直连地址 {addr} 应为 IP:端口"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let core = LocalCore::open()?;
    let body = serde_json::json!({ "direct_addrs": direct_addrs }).to_string();
    if first {
        return wait_for_first_device(&core, &body).await;
    }
    let (status, bytes) = core.post("/_wonton/api/pairing-codes", &body).await?;
    show(&issued(status, &bytes)?, "只能用一次")
}

/// 一开始就已经有设备时说明怎么加设备；否则显示首次配对码，直到第一台设备配上。
async fn wait_for_first_device(core: &LocalCore, body: &str) -> anyhow::Result<()> {
    let mut shown: Option<String> = None;
    loop {
        let (status, bytes) = core.post("/_wonton/api/pairing-codes/first", body).await?;
        if status == StatusCode::CONFLICT {
            if shown.is_some() {
                println!("配对成功，这台设备已经连上服务器。");
            } else {
                println!(
                    "服务器上已经有配对的设备。要加设备，在已配对的 App 里点“添加设备”；设备都丢了，在服务器上执行 sudo wonton pair。"
                );
            }
            return Ok(());
        }
        let code = issued(status, &bytes)?;
        if shown.as_deref() != Some(code.code.as_str()) {
            show(&code, "第一台设备配上后作废，过期前这里会换一个新码")?;
            shown = Some(code.code);
        }
        tokio::time::sleep(POLL).await;
    }
}

fn issued(status: StatusCode, bytes: &[u8]) -> anyhow::Result<Issued> {
    if !status.is_success() {
        bail!(
            "核心没有发码：HTTP {status} {}",
            String::from_utf8_lossy(bytes)
        );
    }
    serde_json::from_slice(bytes).context("核心的回应不是发码的 JSON")
}

fn show(issued: &Issued, rule: &str) -> anyhow::Result<()> {
    let stdout = std::io::stdout();
    let columns = if stdout.is_terminal() {
        Some(
            rustix::termios::tcgetwinsize(&stdout)
                .context("读取终端宽度失败")?
                .ws_col,
        )
    } else {
        None
    };
    print!("{}", format_pairing(issued, rule, columns));
    Ok(())
}

fn format_pairing(issued: &Issued, rule: &str, columns: Option<u16>) -> String {
    let (mut output, action) = match terminal_qr(&issued.code, columns) {
        Ok(qr) => (qr, "用云吞 App 扫上面的二维码，或者在 App 里粘贴这段配对码"),
        Err(required) => (
            format!(
                "终端窗口太窄（需要至少 {required} 列，现在 {} 列），请把窗口拉宽后重新运行。",
                columns.expect("只有限制列数时二维码才会放不下")
            ),
            "在云吞 App 里粘贴这段配对码",
        ),
    };
    let minutes = issued.expires_in.div_ceil(60);
    write!(
        output,
        "\n{action}（{minutes} 分钟内有效，{rule}）：\n{}\n",
        issued.code
    )
    .expect("写入字符串不会失败");
    output
}

/// 同一个容器里运行中的核心，用主人令牌调它的管理 API。
struct LocalCore {
    url: String,
    host: String,
    token: String,
    http: HttpClient,
}

impl LocalCore {
    fn open() -> anyhow::Result<Self> {
        let cfg = Config::from_env()?;
        let token_path = cfg.core_state_dir().join("owner-token");
        let token = std::fs::read_to_string(&token_path)
            .with_context(|| format!("读取令牌 {}（核心还没启动过？）", token_path.display()))?;
        Ok(Self {
            url: format!("http://127.0.0.1:{}", cfg.listen.port()),
            host: cfg.public.core_host().to_owned(),
            token: token.trim().to_owned(),
            http: HttpClient::new(),
        })
    }

    async fn post(&self, path: &str, body: &str) -> anyhow::Result<(StatusCode, Vec<u8>)> {
        let request = hyper::Request::builder()
            .method(Method::POST)
            .uri(format!("{}{path}", self.url))
            .header(hyper::header::HOST, &self.host)
            .header(
                hyper::header::AUTHORIZATION,
                format!("Bearer {}", self.token),
            )
            .header(hyper::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body.to_owned()))?;
        let response = tokio::time::timeout(Duration::from_secs(10), self.http.send(request))
            .await
            .context("核心没有及时回应")?
            .context("连不上运行中的核心")?;
        let status = response.status();
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await?
            .to_bytes();
        Ok((status, bytes.to_vec()))
    }
}

#[cfg(test)]
#[path = "pair_command_tests.rs"]
mod pair_command_tests;
