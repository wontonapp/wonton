//! 索引里的加速名单：按仓库主机各一个有序列表（ADR-0074）。列表只影响按摘要钉住的引用。

use serde::Deserialize;

use super::address;
use crate::plugin::languages::Languages;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mirrors {
    #[serde(rename = "docker.io", default)]
    pub(in crate::plugin) docker_hub: Vec<String>,
    #[serde(rename = "ghcr.io", default)]
    pub(in crate::plugin) ghcr: Vec<String>,
    #[serde(rename = "quay.io", default)]
    pub(in crate::plugin) quay: Vec<String>,
}

/// 索引里的 `registry_mirrors` 读不懂时整份名单作废，原因交给设置页显示；别的索引数据照常可用。
pub(in crate::plugin) fn parse(
    value: serde_json::Value,
    langs: &Languages,
) -> Result<Mirrors, String> {
    let mirrors: Mirrors = serde_json::from_value(value).map_err(|e| e.to_string())?;
    for address in mirrors
        .docker_hub
        .iter()
        .chain(&mirrors.ghcr)
        .chain(&mirrors.quay)
    {
        address::validate(address, langs)?;
    }
    Ok(mirrors)
}

/// 名字里的仓库主机。裸名按 Docker Hub 算，和 Docker 的解析一致。
pub(in crate::plugin) fn registry(image: &str) -> Option<Registry> {
    let name = image.split('@').next().unwrap_or(image);
    match name.split_once('/') {
        Some((host, _)) if host.contains(['.', ':']) || host == "localhost" => match host {
            "docker.io" | "index.docker.io" | "registry-1.docker.io" => Some(Registry::DockerHub),
            "ghcr.io" => Some(Registry::Ghcr),
            "quay.io" => Some(Registry::Quay),
            _ => None,
        },
        _ => Some(Registry::DockerHub),
    }
}

impl Mirrors {
    pub(in crate::plugin) fn stations(&self, registry: Registry) -> &[String] {
        match registry {
            Registry::DockerHub => &self.docker_hub,
            Registry::Ghcr => &self.ghcr,
            Registry::Quay => &self.quay,
        }
    }

    /// 同一个地址可能既在索引里又由主人填过：只留第一次出现的位置。
    pub(in crate::plugin) fn dedupe(&mut self) {
        for stations in [&mut self.docker_hub, &mut self.ghcr, &mut self.quay] {
            let mut seen = Vec::new();
            stations.retain(|station| {
                let first = !seen.contains(station);
                seen.push(station.clone());
                first
            });
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::plugin) enum Registry {
    DockerHub,
    Ghcr,
    Quay,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn langs() -> Languages {
        Languages::default()
    }

    #[test]
    fn mirrors_are_read_per_registry_and_checked() {
        let mirrors = parse(
            serde_json::json!({
                "docker.io": ["docker.example"],
                "ghcr.io": ["ghcr.example:5000"],
                "quay.io": [],
            }),
            &langs(),
        )
        .unwrap();
        assert_eq!(mirrors.docker_hub, ["docker.example"]);
        assert_eq!(mirrors.ghcr, ["ghcr.example:5000"]);
        assert!(mirrors.quay.is_empty());
        // 没写的仓库当没有名单。
        assert_eq!(
            parse(serde_json::json!({"ghcr.io": ["ghcr.example"]}), &langs())
                .unwrap()
                .docker_hub,
            Vec::<String>::new()
        );
        // 沿用地址的写法：协议、路径都不收。
        for invalid in [
            serde_json::json!(["mirror.example"]),
            serde_json::json!({"ghcr.io": "ghcr.example"}),
            serde_json::json!({"ghcr.io": ["https://ghcr.example"]}),
            serde_json::json!({"gitlab.com": ["gitlab.example"]}),
        ] {
            assert!(parse(invalid.clone(), &langs()).is_err(), "{invalid}");
        }
    }

    #[test]
    fn registries_are_recognized_from_the_reference() {
        for (image, registry) in [
            ("alpine", Registry::DockerHub),
            ("library/alpine", Registry::DockerHub),
            ("docker.io/org/app", Registry::DockerHub),
            ("registry-1.docker.io/org/app", Registry::DockerHub),
            ("ghcr.io/org/app", Registry::Ghcr),
            ("quay.io/org/app", Registry::Quay),
        ] {
            assert_eq!(registry_of(image), Some(registry), "{image}");
        }
        for image in [
            "localhost:5000/org/app",
            "127.0.0.1:5000/app",
            "gitlab.com/org/app",
        ] {
            assert_eq!(registry_of(image), None, "{image}");
        }
    }

    fn registry_of(image: &str) -> Option<Registry> {
        registry(image)
    }

    #[test]
    fn dedupe_keeps_the_first_position() {
        let mut mirrors = Mirrors {
            docker_hub: vec!["a.example".into(), "b.example".into(), "a.example".into()],
            ghcr: vec![],
            quay: vec![],
        };
        mirrors.dedupe();
        assert_eq!(mirrors.docker_hub, ["a.example", "b.example"]);
    }
}
