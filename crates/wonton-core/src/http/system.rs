//! 系统服务只连接核心 Compose 网络里的地址；宿主开发模式只连接回环地址。

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::future::BoxFuture;
use hyper::Uri;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tower::Service;

use super::{HttpClient, HttpError};
use crate::plugin::docker::{Engine, EngineError};
use crate::plugin::sources::Subnet;

const PROJECT: &str = "com.docker.compose.project";
const NETWORK: &str = "com.docker.compose.network";
type Resolve = Arc<dyn Fn(String) -> BoxFuture<'static, io::Result<Vec<SocketAddr>>> + Send + Sync>;

#[derive(Clone)]
pub(super) struct SystemConnector {
    subnets: Arc<[Subnet]>,
    resolve: Resolve,
}

impl HttpClient {
    /// 启动时只读取核心自己的 Compose `core` 网络；查询失败不能继续启动。
    pub async fn for_system_services(
        engine: &Engine,
        core_container: Option<&str>,
    ) -> Result<Self, EngineError> {
        let Some(container) = core_container else {
            return Ok(Self::system_loopback());
        };
        Ok(Self::system_client(core_subnets(engine, container).await?))
    }

    pub(crate) fn system_loopback() -> Self {
        Self::system_client(vec![
            Subnet::parse("127.0.0.0/8").unwrap(),
            Subnet::parse("::1/128").unwrap(),
        ])
    }

    fn system_client(subnets: Vec<Subnet>) -> Self {
        Self::with_system_connector(SystemConnector {
            subnets: subnets.into(),
            resolve: Arc::new(|name| {
                Box::pin(async move {
                    tokio::net::lookup_host((name.as_str(), 0))
                        .await
                        .map(Iterator::collect)
                })
            }),
        })
    }
}

async fn core_subnets(engine: &Engine, container: &str) -> Result<Vec<Subnet>, EngineError> {
    let context = format!("查询核心容器 {container} 的 Compose core 网络");
    let invalid = |message: &str| EngineError::Rejected {
        context: context.clone(),
        message: message.to_owned(),
    };
    let info = engine
        .inspect_container(container)
        .await?
        .ok_or_else(|| EngineError::NotFound(context.clone()))?;
    let project = info
        .config
        .and_then(|config| config.labels)
        .and_then(|mut labels| labels.remove(PROJECT))
        .filter(|project| !project.is_empty())
        .ok_or_else(|| invalid("核心容器没有 Compose 项目标签"))?;
    let networks = info
        .network_settings
        .and_then(|settings| settings.networks)
        .ok_or_else(|| invalid("核心容器没有网络信息"))?;
    let mut selected = None;
    for (name, _) in networks {
        let labels = engine
            .network_labels(&name)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("网络 {name}")))?;
        if labels.get(PROJECT) == Some(&project)
            && labels.get(NETWORK).is_some_and(|network| network == "core")
            && selected.replace(name).is_some()
        {
            return Err(invalid("核心容器接在多个 Compose core 网络上"));
        }
    }
    let network =
        selected.ok_or_else(|| invalid("核心容器没有接入自己项目的 Compose core 网络"))?;
    engine.network_subnets(&network).await
}

impl SystemConnector {
    async fn connect(self, uri: Uri) -> Result<TokioIo<TcpStream>, BoxError> {
        if uri.scheme_str() != Some("http") {
            return Err(HttpError::BadUrl(uri.to_string()).into());
        }
        let host = uri
            .host()
            .ok_or_else(|| HttpError::BadUrl(uri.to_string()))?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let port = uri.port_u16().unwrap_or(80);
        let mut addresses = if let Ok(ip) = host.parse::<IpAddr>() {
            vec![SocketAddr::new(ip, port)]
        } else {
            (self.resolve)(host.to_owned()).await?
        };
        for address in &mut addresses {
            address.set_port(port);
        }
        if addresses.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("系统服务 {host} 没有解析到地址"),
            )
            .into());
        }
        // 先核对整份答案，再把同一批地址交给 TCP；混合内外地址也拒绝，不做第二次名字解析。
        if addresses
            .iter()
            .any(|address| !self.subnets.iter().any(|net| net.contains(address.ip())))
        {
            return Err(HttpError::SystemResolution {
                name: host.to_owned(),
                addresses,
            }
            .into());
        }
        let stream = tokio::time::timeout(
            Duration::from_secs(5),
            TcpStream::connect(addresses.as_slice()),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "连接系统服务超时"))??;
        stream.set_nodelay(true)?;
        Ok(TokioIo::new(stream))
    }
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

impl Service<Uri> for SystemConnector {
    type Response = TokioIo<TcpStream>;
    type Error = BoxError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        Box::pin(self.clone().connect(uri))
    }
}

#[cfg(test)]
mod tests;
