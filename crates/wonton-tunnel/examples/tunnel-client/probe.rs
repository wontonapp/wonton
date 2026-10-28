//! 测量直连与中继的比例和延迟。
//!
//! 每轮用一个新的 endpoint 从零建连，模拟 App 冷启动。每轮依次：
//! 1. ping 一次，包含建连和服务端确认授权，记为建连耗时；
//! 2. 观察期内每 200 毫秒 ping 一次，记录往返时间和当时选中的路径，以及第一次出现直连的时刻；
//! 3. 经本地代理发若干个 HTTP GET（每个一条新 TCP 连接、一条新隧道流），记录耗时和路径。

use std::{
    net::{Ipv4Addr, SocketAddr},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use iroh::{Endpoint, EndpointAddr, SecretKey};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use wonton_tunnel::{LocalProxy, PathInfo, PathKind, ProxyAccess, TunnelClient};

/// 观察期内两次 ping 的间隔。
const PING_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, clap::Args)]
pub struct ProbeArgs {
    /// 轮数。每轮用新的 endpoint 冷启动建连。
    #[arg(long, default_value_t = 5)]
    trials: usize,
    /// 每轮建连后的观察时长（秒）。
    #[arg(long, default_value_t = 10)]
    observe_secs: u64,
    /// 每轮经本地代理发出的 HTTP GET 数。
    #[arg(long, default_value_t = 20)]
    requests: usize,
    /// HTTP 请求路径，服务端上游要对它回 2xx。
    #[arg(long, default_value = "/")]
    path: String,
}

/// 一次耗时样本和当时选中的路径。
#[derive(Debug, Clone, Copy)]
struct Sample {
    kind: Option<PathKind>,
    elapsed: Duration,
}

#[derive(Debug)]
struct Trial {
    connect: Duration,
    first_path: Option<PathKind>,
    direct_after: Option<Duration>,
    final_path: Option<PathInfo>,
    pings: Vec<Sample>,
    http: Vec<Sample>,
}

pub async fn run(
    key: SecretKey,
    server: EndpointAddr,
    relay_only: bool,
    args: &ProbeArgs,
) -> Result<()> {
    let mut trials = Vec::new();
    let mut failures = 0;
    for round in 1..=args.trials {
        let endpoint = crate::bind_endpoint(key.clone(), relay_only).await?;
        let result = run_trial(&endpoint, server.clone(), args).await;
        endpoint.close().await;
        match result {
            Ok(trial) => {
                print_trial(round, &trial);
                trials.push(trial);
            }
            Err(err) => {
                failures += 1;
                println!("第 {round} 轮失败：{err:#}");
            }
        }
    }
    print_summary(&trials, failures);
    Ok(())
}

async fn run_trial(endpoint: &Endpoint, server: EndpointAddr, args: &ProbeArgs) -> Result<Trial> {
    let client = TunnelClient::new(endpoint.clone(), server);
    let connect = client.ping().await.context("建连")?;
    let first_path = client.path().map(|path| path.kind);
    let mut direct_after = (first_path == Some(PathKind::Direct)).then_some(connect);
    let mut pings = Vec::new();
    let observe_until = Instant::now() + Duration::from_secs(args.observe_secs);
    while Instant::now() < observe_until {
        let rtt = client.ping().await.context("ping")?;
        let kind = client.path().map(|path| path.kind);
        if direct_after.is_none() && kind == Some(PathKind::Direct) {
            direct_after = Some(connect + elapsed_since_connect(&pings, rtt));
        }
        pings.push(Sample { kind, elapsed: rtt });
        tokio::time::sleep(PING_INTERVAL).await;
    }
    let http = run_http(&client, args).await?;
    let final_path = client.path();
    client.close();
    Ok(Trial {
        connect,
        first_path,
        direct_after,
        final_path,
        pings,
        http,
    })
}

/// 建连之后到当前这次 ping 结束经过的时间（每次 ping 之间隔 [`PING_INTERVAL`]）。
fn elapsed_since_connect(previous: &[Sample], current: Duration) -> Duration {
    let pinged: Duration = previous.iter().map(|sample| sample.elapsed).sum();
    let slept = PING_INTERVAL * u32::try_from(previous.len()).unwrap_or(u32::MAX);
    pinged + slept + current
}

async fn run_http(client: &TunnelClient, args: &ProbeArgs) -> Result<Vec<Sample>> {
    let proxy = LocalProxy::bind(
        client.clone(),
        (Ipv4Addr::LOCALHOST, 0).into(),
        ProxyAccess::Open,
    )
    .await?;
    let mut samples = Vec::with_capacity(args.requests);
    for _ in 0..args.requests {
        let started = Instant::now();
        let status = http_get(proxy.local_addr(), &args.path).await?;
        if !(200..300).contains(&status) {
            bail!("HTTP 状态 {status}");
        }
        samples.push(Sample {
            kind: client.path().map(|path| path.kind),
            elapsed: started.elapsed(),
        });
    }
    proxy.shutdown().await;
    Ok(samples)
}

/// 发一个 GET，读完整个响应，返回状态码。
async fn http_get(proxy: SocketAddr, path: &str) -> Result<u16> {
    let mut tcp = TcpStream::connect(proxy).await?;
    tcp.set_nodelay(true)?;
    let request = format!("GET {path} HTTP/1.1\r\nhost: wonton\r\nconnection: close\r\n\r\n");
    tcp.write_all(request.as_bytes()).await?;
    let mut response = Vec::new();
    tcp.read_to_end(&mut response).await?;
    let status_line = response.split(|&b| b == b'\n').next().unwrap_or_default();
    String::from_utf8_lossy(status_line)
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .context("响应不是 HTTP")
}

fn print_trial(round: usize, trial: &Trial) {
    let direct_after = trial
        .direct_after
        .map_or("观察期内没有直连".to_owned(), |after| {
            format!("{} ms 后直连", ms(after))
        });
    let final_path = trial.final_path.as_ref().map_or("无".to_owned(), |path| {
        format!("{} {}（RTT {} ms）", path.kind, path.remote, ms(path.rtt))
    });
    println!(
        "第 {round} 轮：建连 {} ms，初始路径 {}，{direct_after}，最终路径 {final_path}；ping {}；HTTP {}",
        ms(trial.connect),
        kind_name(trial.first_path),
        summarize(trial.pings.iter().map(|sample| sample.elapsed)),
        summarize(trial.http.iter().map(|sample| sample.elapsed)),
    );
}

fn print_summary(trials: &[Trial], failures: usize) {
    println!("\n== 汇总：成功 {} 轮，失败 {failures} 轮 ==", trials.len());
    let final_direct = trials
        .iter()
        .filter(|trial| matches!(&trial.final_path, Some(path) if path.kind == PathKind::Direct))
        .count();
    println!("最终走直连：{final_direct}/{} 轮", trials.len());
    println!(
        "建连（含授权确认）：{}",
        summarize(trials.iter().map(|trial| trial.connect))
    );
    println!(
        "建连到出现直连：{}",
        summarize(trials.iter().filter_map(|trial| trial.direct_after))
    );
    let pings: Vec<Sample> = trials
        .iter()
        .flat_map(|trial| trial.pings.clone())
        .collect();
    let http: Vec<Sample> = trials.iter().flat_map(|trial| trial.http.clone()).collect();
    for (name, samples) in [("ping 往返", &pings), ("HTTP GET", &http)] {
        for kind in [PathKind::Direct, PathKind::Relay] {
            let matching = samples.iter().filter(|sample| sample.kind == Some(kind));
            println!(
                "{name}（{kind}）：{}",
                summarize(matching.map(|sample| sample.elapsed))
            );
        }
    }
    let direct = pings
        .iter()
        .filter(|sample| sample.kind == Some(PathKind::Direct))
        .count();
    println!("ping 样本中直连占比：{direct}/{}", pings.len());
}

/// “n 个，中位 x ms，P90 y ms，最大 z ms”。
fn summarize(values: impl Iterator<Item = Duration>) -> String {
    let mut values: Vec<Duration> = values.collect();
    if values.is_empty() {
        return "无样本".to_owned();
    }
    values.sort();
    let at = |p: f64| {
        let last = values.len() - 1;
        let index = (last as f64 * p).round() as usize;
        values[index.min(last)]
    };
    format!(
        "{} 个，中位 {} ms，P90 {} ms，最大 {} ms",
        values.len(),
        ms(at(0.5)),
        ms(at(0.9)),
        ms(values[values.len() - 1]),
    )
}

fn kind_name(kind: Option<PathKind>) -> String {
    kind.map_or("未知".to_owned(), |kind| kind.to_string())
}

fn ms(duration: Duration) -> String {
    format!("{:.1}", duration.as_secs_f64() * 1000.0)
}
