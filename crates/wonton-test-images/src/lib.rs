//! 把测试要拉的上游镜像引用改写到镜像仓库副本。
//!
//! GitHub 托管 runner 的出口 IP 轮换且量大，Docker Hub 匿名拉取和 ECR Public 都会被限流（实测
//! `registry:2` 返回 toomanyrequests）。CI 每周把测试用到的镜像复制到
//! `ghcr.io/<owner>/test-images/<名字>`（`scripts/mirror-test-images.sh`，digest 原样保留），
//! 测试作业设 `WONTON_TEST_IMAGES=ghcr.io/<owner>/test-images`，镜像引用就指到副本。
//! 环境变量没设时原样返回，本地默认不变。
//!
//! 改写规则：仓库路径取最后一段做名字，标签和 digest 原样保留。
//! `public.ecr.aws/docker/library/busybox:1.37` → `ghcr.io/<owner>/test-images/busybox:1.37`，
//! `gogost/gost:3@sha256:…` → `ghcr.io/<owner>/test-images/gost:3@sha256:…`。

/// 改写一个镜像引用；`WONTON_TEST_IMAGES` 没设时原样返回。
pub fn image(default: &str) -> String {
    let Some(prefix) = std::env::var_os("WONTON_TEST_IMAGES") else {
        return default.to_owned();
    };
    let prefix = prefix.to_string_lossy();
    if prefix.is_empty() {
        return default.to_owned();
    }
    let (name_and_tag, digest) = match default.split_once('@') {
        Some((reference, digest)) => (reference, digest),
        None => (default, ""),
    };
    let (repo, tag) = match name_and_tag.rsplit_once(':') {
        Some((repo, tag)) => (repo, tag),
        None => (name_and_tag, ""),
    };
    let Some(name) = repo.rsplit('/').next() else {
        return default.to_owned();
    };
    let mut mapped = format!("{prefix}/{name}");
    if !tag.is_empty() {
        mapped.push(':');
        mapped.push_str(tag);
    }
    if !digest.is_empty() {
        mapped.push('@');
        mapped.push_str(digest);
    }
    mapped
}

#[cfg(test)]
mod tests {
    use super::image;

    // 一个测试里顺序摆布环境变量：它们是进程级的，分几个测试并行跑会互相踩。
    // set_var/remove_var 在 2024 edition 里是 unsafe：这个测试是单线程的进程内改动，
    // 没有别的线程同时读环境变量，不会读到改了一半的值。
    #[test]
    fn maps_refs_only_with_the_env_set() {
        const TUWUNEL: &str = "ghcr.io/matrix-construct/tuwunel:v1.9.4";

        unsafe { std::env::remove_var("WONTON_TEST_IMAGES") };
        assert_eq!(image(TUWUNEL), TUWUNEL);

        unsafe { std::env::set_var("WONTON_TEST_IMAGES", "") };
        assert_eq!(image(TUWUNEL), TUWUNEL);

        unsafe { std::env::set_var("WONTON_TEST_IMAGES", "ghcr.io/example/test-images") };
        assert_eq!(
            image("public.ecr.aws/docker/library/busybox:1.37"),
            "ghcr.io/example/test-images/busybox:1.37"
        );
        assert_eq!(
            image("docker.io/node:22@sha256:18a6"),
            "ghcr.io/example/test-images/node:22@sha256:18a6"
        );
        assert_eq!(
            image("public.ecr.aws/docker/library/busybox@sha256:a7db"),
            "ghcr.io/example/test-images/busybox@sha256:a7db"
        );
        assert_eq!(
            image("gogost/gost:3@sha256:f266"),
            "ghcr.io/example/test-images/gost:3@sha256:f266"
        );
        unsafe { std::env::remove_var("WONTON_TEST_IMAGES") };
    }
}
