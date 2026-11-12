//! 按摘要钉住的引用改成经加速站的引用；标签不能提供加速站必须遵守的内容约束。

use super::mirrors::Registry;

pub(crate) fn digest(image: &str) -> Option<&str> {
    let (_, digest) = image.split_once('@')?;
    let hex = digest.strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.bytes().all(|c| c.is_ascii_hexdigit())).then_some(digest)
}

/// 这张镜像在加速站上的引用，按名单顺序。只写标签、或者仓库没有名单时为空。
pub(in crate::plugin) fn accelerated(
    image: &str,
    stations: &[String],
    registry: Registry,
) -> Vec<String> {
    if stations.is_empty() {
        return Vec::new();
    }
    let Some(digest) = digest(image) else {
        return Vec::new();
    };
    let name = image.split_once('@').expect("有摘要的引用带 @").0;
    let name = match registry {
        Registry::DockerHub => ["docker.io/", "index.docker.io/", "registry-1.docker.io/"]
            .iter()
            .find_map(|prefix| name.strip_prefix(prefix))
            .unwrap_or(name),
        Registry::Ghcr => name.strip_prefix("ghcr.io/").unwrap_or(name),
        Registry::Quay => name.strip_prefix("quay.io/").unwrap_or(name),
    };
    let repository = name
        .rsplit_once(':')
        .map_or(name, |(repository, _)| repository);
    let repository = if registry == Registry::DockerHub && !repository.contains('/') {
        format!("library/{repository}")
    } else {
        repository.to_owned()
    };
    stations
        .iter()
        .map(|station| format!("{station}/{repository}@{digest}"))
        .collect()
}

/// 只比摘要：同级引用换了写法、或者来自不同的站，内容仍然可能是同一份。
pub(crate) fn same_image(actual: &str, expected: &str) -> bool {
    actual == expected
        || digest(expected).is_some_and(|digest| self::digest(actual) == Some(digest))
}

/// Engine API 只传仓库名会拉全部标签，不是 compose 的 latest。
pub(in crate::plugin) fn pull_reference(image: &str) -> String {
    let name = image.rsplit('/').next().unwrap_or(image);
    if image.contains('@') || name.contains(':') {
        image.to_owned()
    } else {
        format!("{image}:latest")
    }
}

/// 加速引用里的站，用来在错误里说清试了哪个。
pub(in crate::plugin) fn station(reference: &str) -> &str {
    reference
        .split_once('/')
        .map_or(reference, |(station, _)| station)
}

#[cfg(test)]
mod tests {
    use super::super::mirrors::registry;
    use super::*;

    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn digest_references_keep_the_official_digest_on_every_registry() {
        for (name, registry, repository) in [
            ("alpine", Registry::DockerHub, "library/alpine"),
            (
                "docker.io/library/alpine",
                Registry::DockerHub,
                "library/alpine",
            ),
            ("gogost/gost:3", Registry::DockerHub, "gogost/gost"),
            (
                "ghcr.io/immich-app/immich-server:v3",
                Registry::Ghcr,
                "immich-app/immich-server",
            ),
            (
                "quay.io/prometheus/busybox:latest",
                Registry::Quay,
                "prometheus/busybox",
            ),
        ] {
            let original = format!("{name}@{DIGEST}");
            let stations = vec!["station.example:5443".to_owned()];
            assert_eq!(
                accelerated(&original, &stations, registry),
                [format!("station.example:5443/{repository}@{DIGEST}")]
            );
        }
    }

    #[test]
    fn tags_without_a_digest_and_empty_lists_are_not_redirected() {
        let stations = vec!["station.example".to_owned()];
        for image in ["alpine:3", "ghcr.io/org/app:1", "quay.io/org/app"] {
            let registry = registry(image).unwrap();
            assert!(
                accelerated(image, &stations, registry).is_empty(),
                "{image}"
            );
        }
        assert!(accelerated(&format!("ghcr.io/org/app@{DIGEST}"), &[], Registry::Ghcr).is_empty());
    }

    #[test]
    fn stations_are_kept_in_order() {
        let stations = vec!["first.example".to_owned(), "second.example".to_owned()];
        assert_eq!(
            accelerated(
                &format!("ghcr.io/org/app@{DIGEST}"),
                &stations,
                Registry::Ghcr
            ),
            [
                format!("first.example/org/app@{DIGEST}"),
                format!("second.example/org/app@{DIGEST}"),
            ]
        );
        assert_eq!(
            station(&format!("first.example/org/app@{DIGEST}")),
            "first.example"
        );
    }

    #[test]
    fn same_image_accepts_the_same_digest_only() {
        assert!(!same_image(&format!("alpine@{DIGEST}"), "alpine:3"));
        assert!(!same_image(
            &format!("alpine@{}", DIGEST.replace('0', "1")),
            &format!("alpine@{DIGEST}")
        ));
    }
}
