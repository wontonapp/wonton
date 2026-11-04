//! `wonton-core compose-images`：这个版本的 compose 文件要用的其他镜像，一行一个。升级器是旧版本，看不懂新版本的 compose
//! 文件，所以让新镜像自己说（跨版本的接口，ADR-0025）。不用 `docker-compose pull`：它加载项目时就检查 `env_file`，
//! 新版本多引用一个由 `init` 生成的文件，这一版就永远升不了。

use std::collections::{BTreeMap, BTreeSet};
use std::process::ExitCode;

use serde::Deserialize;

/// 给升级器调用的子命令。结果打印在标准输出上，不初始化日志。
pub const COMMAND: &str = "compose-images";

#[derive(Deserialize)]
struct Compose {
    services: BTreeMap<String, Service>,
}

#[derive(Deserialize)]
struct Service {
    image: Option<String>,
}

/// 除了核心自己以外的镜像，去重。核心的镜像来自 `.env`，由升级流程单独处理。
pub fn images(compose: &str) -> anyhow::Result<Vec<String>> {
    let compose: Compose = serde_saphyr::from_str(compose)?;
    let images: BTreeSet<String> = compose
        .services
        .into_iter()
        .filter(|(name, _)| name != super::CORE_SERVICE)
        .filter_map(|(_, service)| service.image)
        .collect();
    Ok(images.into_iter().collect())
}

pub fn command() -> ExitCode {
    match images(crate::install::COMPOSE) {
        Ok(images) => {
            for image in images {
                println!("{image}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("解析 compose 文件失败：{e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 发布的 compose 文件：其他服务的镜像都不带变量，升级器才能直接拉。
    #[test]
    fn the_installed_compose_lists_fixed_images_without_the_core() {
        let images = images(crate::install::COMPOSE).unwrap();
        assert!(images.iter().any(|i| i.contains("tuwunel")), "{images:?}");
        assert!(images.iter().any(|i| i.contains("hydra")), "{images:?}");
        assert!(images.iter().all(|i| !i.contains('$')), "{images:?}");
        let hydra = images.iter().filter(|i| i.contains("hydra")).count();
        assert_eq!(hydra, 1, "hydra 和 hydra-migrate 用同一个镜像：{images:?}");
    }
}
