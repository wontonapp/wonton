//! 查代理为什么连不上（ADR-0023 的“代理连不上时怎么报错”）。经出口容器分不清：代理拒绝连接、丢包和回 407，gost 都回 503（实测）。
//! 所以直接连代理，依次试 TCP、CONNECT、TLS。
//!
//! `wonton-core egress-proxy-check`：在接着出口网络的一次性容器里跑。防火墙常按网段放行，要在实际用的网桥上测。代理地址经环境变量
//! 传入，不放命令行：容器进程的命令行宿主上的普通用户也看得到。结果以 JSON 打印在标准输出上；代理能用（包括它连不上外面）时退出码是
//! 0，安装脚本只看退出码。

use std::error::Error;
use std::process::ExitCode;
use std::time::Duration;

use base64::Engine as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use wonton_protocol::{EgressFailure, EgressProblem};

use super::address::{ProxyAddress, Scheme};

pub const COMMAND: &str = "egress-proxy-check";
pub const URL_VAR: &str = "WONTON_EGRESS_CHECK_PROXY";

/// 探测访问这个网站：核心本来就要连它（ADR-0017）。
pub const TARGET: Target = Target {
    host: "github.com",
    url: "https://github.com/",
};

/// TCP 和 CONNECT 各等这么久。插件经 gost 等的是 gost 默认的 15 秒，不是这个。
const STEP_TIMEOUT: Duration = Duration::from_secs(5);
/// 经代理走完 TLS、拿到响应头。代理的出口比到代理慢得多。
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HEAD: usize = 8 << 10;

/// 探测的目标。
#[derive(Debug, Clone, Copy)]
pub struct Target {
    /// CONNECT 到这个主机的 443。
    pub host: &'static str,
    pub url: &'static str,
}

fn problem(reason: EgressFailure, message: String) -> Option<EgressProblem> {
    Some(EgressProblem { reason, message })
}

fn unreachable(message: String) -> Option<EgressProblem> {
    problem(EgressFailure::ProxyUnreachable, message)
}

fn rejected(message: String) -> Option<EgressProblem> {
    problem(EgressFailure::ProxyRejected, message)
}

fn no_response(address: &str) -> Option<EgressProblem> {
    let seconds = STEP_TIMEOUT.as_secs();
    unreachable(format!(
        "代理 {address} {seconds} 秒没响应：宿主防火墙要放行 Docker 网段到这个端口"
    ))
}

/// 错误和它的原因连成一行：reqwest 把有用的话放在 `source` 里。
fn chain(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(next) = source {
        text = format!("{text}：{next}");
        source = next.source();
    }
    text
}

/// 经代理访问 `url`，拿到响应头就算通。不读 `HTTPS_PROXY`：指定了代理就只用它。
pub async fn fetch(proxy: &str, url: &str, limit: Duration) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy).map_err(|e| chain(&e))?)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(limit)
        .build()
        .map_err(|e| chain(&e))?;
    client.get(url).send().await.map_err(|e| chain(&e))?;
    Ok(())
}

/// 问题；能用时为空。结果按这个顺序判断：连不上 → 代理拒绝 → 代理连不上外面。
pub async fn check(proxy: &ProxyAddress, target: Target) -> Option<EgressProblem> {
    let address = proxy.authority();
    let mut stream = match timeout(STEP_TIMEOUT, TcpStream::connect(&address)).await {
        Err(_) => return no_response(&address),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            return unreachable(format!(
                "代理 {address} 拒绝连接：端口没人听，或者只监听回环地址"
            ));
        }
        Ok(Err(e)) => return unreachable(format!("连不上代理 {address}：{e}")),
        Ok(Ok(stream)) => stream,
    };
    // https 的代理要先握手，socks5 不说 HTTP：这两种看 reqwest 的错误。
    if proxy.scheme == Scheme::Http
        && let Some(problem) = connect_status(&mut stream, proxy, target).await
    {
        return problem;
    }
    drop(stream);
    match fetch(proxy.text(), target.url, FETCH_TIMEOUT).await {
        Ok(()) => None,
        // mihomo 对 CONNECT 先回 200，连不上目标时才断开（实测），所以要做完 TLS 握手。
        Err(e) if proxy.scheme == Scheme::Http || !refused_by_proxy(&e) => problem(
            EgressFailure::UpstreamFailed,
            format!("代理本身是通的，它连不上外面：{e}"),
        ),
        Err(e) => rejected(format!("代理这一步没通过：{e}")),
    }
}

/// reqwest 没有错误类别，只能看字：这两种代理才走到这里。对着明文端口写 `https://` 时报的是 `failed to create underlying
/// connection`（reqwest 0.13.5 实测）。
fn refused_by_proxy(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    [
        "proxy authentication",
        "unsuccessful tunnel",
        "underlying connection",
        "socks",
    ]
    .iter()
    .any(|word| error.contains(word))
}

/// 发 CONNECT，看状态码。外层的 `Some` 是有结论了：内层是问题。
async fn connect_status(
    stream: &mut TcpStream,
    proxy: &ProxyAddress,
    target: Target,
) -> Option<Option<EgressProblem>> {
    let address = proxy.authority();
    let auth = (proxy.username.is_some() || proxy.password.is_some()).then(|| {
        let user = proxy.username.as_deref().unwrap_or_default();
        let password = proxy.password.as_deref().unwrap_or_default();
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
        format!("Proxy-Authorization: Basic {encoded}\r\n")
    });
    let host = target.host;
    let auth = auth.unwrap_or_default();
    let request = format!("CONNECT {host}:443 HTTP/1.1\r\nHost: {host}:443\r\n{auth}\r\n");
    if let Err(e) = stream.write_all(request.as_bytes()).await {
        return Some(unreachable(format!("连不上代理 {address}：{e}")));
    }
    let head = match timeout(STEP_TIMEOUT, read_head(stream)).await {
        Err(_) => return Some(no_response(&address)),
        Ok(Err(e)) => return Some(unreachable(format!("代理 {address} 断开了连接：{e}"))),
        Ok(Ok(head)) => head,
    };
    let status = head
        .split_ascii_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok());
    match status {
        None => Some(rejected(format!("代理 {address} 回的不是 HTTP"))),
        Some(code) if (200..300).contains(&code) => None,
        Some(407) => Some(rejected(format!(
            "代理 {address} 回 407：要用户名和密码，或者写错了"
        ))),
        Some(code) => Some(rejected(format!("代理 {address} 回 {code}"))),
    }
}

async fn read_head(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut head = Vec::new();
    let mut buffer = [0; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < MAX_HEAD {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        head.extend_from_slice(&buffer[..read]);
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

/// 子命令的入口。
pub fn command() -> ExitCode {
    let text = std::env::var(URL_VAR).unwrap_or_default();
    let proxy = match ProxyAddress::parse(&text) {
        Ok(proxy) => proxy,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let found = runtime.block_on(check(&proxy, TARGET));
    println!("{}", serde_json::to_string(&found).unwrap_or_default());
    match found {
        Some(EgressProblem { reason, .. }) if reason != EgressFailure::UpstreamFailed => {
            ExitCode::from(1)
        }
        _ => ExitCode::SUCCESS,
    }
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use super::*;

    /// 只回答一次的假代理：读完请求头，回 `reply`。把请求头交回来。
    async fn proxy_replying(
        reply: &'static str,
    ) -> (ProxyAddress, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let head = read_head(&mut stream).await.unwrap();
            stream.write_all(reply.as_bytes()).await.unwrap();
            head
        });
        let address = format!("http://me:s3%3Acret@127.0.0.1:{port}");
        (ProxyAddress::parse(&address).unwrap(), task)
    }

    fn reason(found: Option<EgressProblem>) -> EgressFailure {
        found.expect("应有问题").reason
    }

    #[tokio::test]
    async fn a_closed_port_is_unreachable() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let proxy = ProxyAddress::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        let found = check(&proxy, TARGET).await.unwrap();
        assert_eq!(found.reason, EgressFailure::ProxyUnreachable);
        assert!(found.message.contains("只监听回环"), "{}", found.message);
    }

    #[tokio::test]
    async fn the_proxy_saying_407_is_rejected_and_says_so() {
        let (proxy, seen) = proxy_replying("HTTP/1.1 407 Proxy Auth\r\n\r\n").await;
        let found = check(&proxy, TARGET).await.unwrap();
        assert_eq!(found.reason, EgressFailure::ProxyRejected);
        assert!(found.message.contains("407"), "{}", found.message);
        let head = seen.await.unwrap();
        assert!(head.starts_with("CONNECT github.com:443 "), "{head}");
        // me:s3:cret 的 base64。
        assert!(
            head.contains("Proxy-Authorization: Basic bWU6czM6Y3JldA=="),
            "{head}"
        );
    }

    #[tokio::test]
    async fn other_refusals_carry_the_status_code() {
        let (proxy, _seen) = proxy_replying("HTTP/1.1 403 Forbidden\r\n\r\n").await;
        let found = check(&proxy, TARGET).await.unwrap();
        assert_eq!(found.reason, EgressFailure::ProxyRejected);
        assert!(found.message.contains("403"), "{}", found.message);
    }

    #[tokio::test]
    async fn saying_200_and_then_not_tunnelling_is_the_proxys_way_out_failing() {
        // 第一次回 200，之后 reqwest 再连时没人听：对应 mihomo 先回 200、之后断开。
        let (proxy, _seen) = proxy_replying("HTTP/1.1 200 OK\r\n\r\n").await;
        assert_eq!(
            reason(check(&proxy, TARGET).await),
            EgressFailure::UpstreamFailed
        );
    }

    #[test]
    fn only_the_proxys_own_words_count_as_a_refusal() {
        assert!(refused_by_proxy("proxy authentication required"));
        assert!(refused_by_proxy("socks connect error"));
        assert!(refused_by_proxy(
            "tunnel error: failed to create underlying connection：tls handshake eof"
        ));
        assert!(!refused_by_proxy("invalid peer certificate"));
    }
}
