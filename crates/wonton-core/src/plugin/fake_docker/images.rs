//! 假 Docker 的镜像：拉取、查询、平台列表和相关的故障注入。

use std::time::Duration;

use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Value, json};

use super::{FakeDocker, FakePlatforms, Resources, json_reply};

impl FakeDocker {
    /// 之后镜像都不在本地、拉取时 Docker 回 `message`（500）；`None` 恢复成镜像都在本地。
    pub fn fail_pulls(&self, message: Option<&str>) {
        self.lock().pull_error = message.map(str::to_owned);
    }

    /// 之后拉镜像的请求拖 `delay` 才回应，让客户端先超时。
    pub fn slow_pulls(&self, delay: Duration) {
        self.lock().pull_delay = Some(delay);
    }

    /// 同 [`Self::fail_pulls`]，但名字里有 `spared` 的镜像还在本地。
    pub fn fail_pulls_except(&self, spared: &str, message: &str) {
        let mut docker = self.lock();
        docker.pull_error = Some(message.to_owned());
        docker.pull_spared = Some(spared.to_owned());
    }

    /// 忘掉一个镜像：查它和用它创建容器都回 404，像 containerd 存储下镜像记录被清掉后那样（#666）。
    pub fn forget_image(&self, image: &str) {
        self.lock().missing_images.insert(image.to_owned());
    }

    /// 让 `GET /info` 报告这些运行时（如 `runc`、`nvidia`）。
    /// 给镜像设上它自带的 `HEALTHCHECK`（Docker 的 JSON 写法）。
    pub fn set_image_healthcheck(&self, image: &str, check: Value) {
        self.lock()
            .image_healthchecks
            .insert(image.to_owned(), check);
    }

    /// 让 `GET /distribution/{镜像}/json` 报告这些架构（不写 `os`）。
    pub fn set_platforms(&self, image: &str, architectures: &[&str], size: Option<u64>) {
        self.lock().platforms.insert(
            image.to_owned(),
            FakePlatforms {
                architectures: architectures.iter().map(|a| (*a).to_owned()).collect(),
                size,
            },
        );
    }

    /// 让查镜像平台列表回这个错误。
    pub fn set_platform_error(&self, error: &str) {
        self.lock().platform_error = Some(error.to_owned());
    }
}

/// `GET /distribution/{镜像}/json`：平台列表和单清单的大小；没设的镜像报一个 linux/amd64。
pub(super) fn distribution(docker: &Resources, image: &str) -> Option<Response> {
    if let Some(error) = &docker.platform_error {
        return Some(json_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"message": error}),
        ));
    }
    let platforms = docker.platforms.get(image).cloned().unwrap_or_default();
    let architectures: Vec<Value> = if platforms.architectures.is_empty() {
        vec![json!({"architecture": "amd64", "os": "linux"})]
    } else {
        platforms
            .architectures
            .iter()
            .map(|arch| json!({"architecture": arch, "os": "linux"}))
            .collect()
    };
    let media_type = if architectures.len() > 1 {
        "application/vnd.docker.distribution.manifest.list.v2+json"
    } else {
        "application/vnd.docker.distribution.manifest.v2+json"
    };
    Some(json_reply(
        StatusCode::OK,
        json!({
            "Descriptor": {
                "mediaType": media_type,
                "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                "size": platforms.size,
            },
            "Platforms": architectures,
        }),
    ))
}
