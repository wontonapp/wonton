//! Docker Hub 的摘要引用；标签不能提供加速地址必须遵守的内容约束。

pub(crate) fn docker_hub(image: &str) -> bool {
    let name = image.split('@').next().unwrap_or(image);
    match name.split_once('/') {
        Some((host, _)) => {
            matches!(
                host,
                "docker.io" | "index.docker.io" | "registry-1.docker.io"
            ) || (!host.contains(['.', ':']) && host != "localhost")
        }
        None => true,
    }
}

pub(crate) fn digest(image: &str) -> Option<&str> {
    let (_, digest) = image.split_once('@')?;
    let hex = digest.strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.bytes().all(|c| c.is_ascii_hexdigit())).then_some(digest)
}

pub(crate) fn mirrored(image: &str, mirror: &str) -> Option<String> {
    if !docker_hub(image) {
        return None;
    }
    let digest = digest(image)?;
    let (mut name, _) = image.split_once('@')?;
    for prefix in ["docker.io/", "index.docker.io/", "registry-1.docker.io/"] {
        if let Some(repository) = name.strip_prefix(prefix) {
            name = repository;
            break;
        }
    }
    let name = name
        .rsplit_once(':')
        .map_or(name, |(repository, _)| repository);
    let library = if name.contains('/') { "" } else { "library/" };
    Some(format!("{mirror}/{library}{name}@{digest}"))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn registry_hub_digest_references_keep_the_official_digest() {
        for (name, repository) in [
            ("alpine", "library/alpine"),
            ("library/alpine", "library/alpine"),
            ("docker.io/alpine:3", "library/alpine"),
            ("docker.io/library/alpine", "library/alpine"),
            ("index.docker.io/gogost/gost:3", "gogost/gost"),
            ("registry-1.docker.io/gogost/gost", "gogost/gost"),
            ("gogost/gost:3", "gogost/gost"),
        ] {
            let original = format!("{name}@{DIGEST}");
            let actual = mirrored(&original, "mirror.example:5443").unwrap();
            assert_eq!(actual, format!("mirror.example:5443/{repository}@{DIGEST}"));
            assert!(same_image(&actual, &original));
        }
    }

    #[test]
    fn registry_tags_and_other_registries_are_not_redirected() {
        for image in [
            "alpine:3".to_owned(),
            "docker.io/alpine:latest".to_owned(),
            format!("ghcr.io/org/app@{DIGEST}"),
            format!("quay.io/org/app@{DIGEST}"),
            format!("localhost:5000/app@{DIGEST}"),
        ] {
            assert_eq!(mirrored(&image, "mirror.example"), None);
        }
        assert!(!same_image(&format!("alpine@{DIGEST}"), "alpine:3"));
    }
}
