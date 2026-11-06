//! 安装器的代理预检也用核心的分配路径；此命令运行在宿主网络的一次性核心容器中。

use super::docker::Engine;
use super::lan::Detector;
use super::names::Instance;
use super::network_pool::{NetworkPool, VAR};

pub const COMMAND: &str = "prepare-egress-network";

pub async fn run() -> anyhow::Result<()> {
    let pool = std::env::var(VAR)
        .ok()
        .filter(|text| !text.is_empty())
        .map_or_else(
            || Ok(NetworkPool::default()),
            |text| text.parse::<NetworkPool>(),
        )
        .map_err(anyhow::Error::msg)?;
    let engine = Engine::connect()?.with_network_pool(pool, Detector::Here);
    let instance = Instance::parse(
        std::env::var("WONTON_INSTANCE")
            .ok()
            .filter(|text| !text.is_empty())
            .as_deref(),
    )
    .map_err(anyhow::Error::msg)?;
    let name = instance.egress();
    match engine.network_labels(&name).await? {
        None => {
            engine
                .create_relay_network(&name, &instance.component_labels())
                .await?
        }
        Some(labels) if instance.is_component(&labels) => {}
        Some(_) => anyhow::bail!("网络 {name} 已被其他程序占用，不能用于出口容器"),
    }
    Ok(())
}
