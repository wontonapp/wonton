use std::os::unix::fs::MetadataExt;
use std::time::Duration;

use bollard::models::{ContainerConfig, ContainerCreateBody, HostConfig, PortBinding};
use bollard::query_parameters::{
    CommitContainerOptionsBuilder, PushImageOptionsBuilder, RemoveImageOptionsBuilder,
};
use futures_util::StreamExt;

use crate::plugin::PluginManager;
use crate::plugin::compose::PullPolicy;
use crate::plugin::docker::Engine;
use crate::plugin::manifest::PluginId;
use crate::test_docker::Names;

/// busybox 和 registry：本地从 ECR Public 拉；CI 经 `WONTON_TEST_IMAGES` 指到镜像仓库副本（wonton-test-images）。
fn base() -> String {
    wonton_test_images::image("public.ecr.aws/docker/library/busybox:1.37")
}

fn registry() -> String {
    wonton_test_images::image("public.ecr.aws/docker/library/registry:2")
}
const REGISTRY_PORT: &str = "5000/tcp";

pub(super) struct Fixture {
    pub engine: Engine,
    pub dir: tempfile::TempDir,
    pub manager: PluginManager,
    /// 仓库在宿主回环上的地址，[`Self::start`] 之后才有。
    pub mirror: String,
    pub repo: String,
    pub id: PluginId,
    registry: String,
    seed: String,
    /// 守卫活到测试结束：一次性容器按登记回收，插件的容器、网络、卷带实例标签，守卫按标签统一清。
    _names: Names,
    pub images: Vec<String>,
}

impl Fixture {
    pub fn new() -> Self {
        let engine = Engine::connect().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut names = Names::new();
        let manager = super::test_support::manager(engine.clone(), &dir, names.name());
        Self {
            engine,
            dir,
            manager,
            mirror: String::new(),
            repo: format!("{}/fixture", names.name()),
            id: PluginId::parse(&format!("app.{}", names.name())).unwrap(),
            registry: names.oneshot_container("registry"),
            seed: names.oneshot_container("fixture-seed"),
            _names: names,
            images: Vec::new(),
        }
    }

    /// 端口由 Docker 发布时分配，不先在测试里绑一个再放掉：放掉到仓库绑上之间，并行的测试或别的进程可能拿走同一个
    /// 端口，仓库起不来，就绪检查却被占着端口的那个应答（CI 上实测过：另一个测试的仓库应答，它一拆，推镜像就被拒）。
    pub async fn start(&mut self) {
        for image in [&registry(), &base()] {
            self.engine
                .ensure_image(image, PullPolicy::Missing, None)
                .await
                .unwrap();
        }
        let storage = self.dir.path().join("registry");
        std::fs::create_dir(&storage).unwrap();
        let owner = std::fs::metadata(&storage).unwrap();
        let loopback = PortBinding {
            host_ip: Some("127.0.0.1".into()),
            host_port: None,
        };
        self.engine
            .create_container(
                &self.registry,
                ContainerCreateBody {
                    image: Some(registry()),
                    user: Some(format!("{}:{}", owner.uid(), owner.gid())),
                    env: Some(vec!["REGISTRY_STORAGE_DELETE_ENABLED=true".into()]),
                    exposed_ports: Some(vec![REGISTRY_PORT.into()]),
                    host_config: Some(HostConfig {
                        port_bindings: Some(
                            [(REGISTRY_PORT.to_owned(), Some(vec![loopback]))].into(),
                        ),
                        binds: Some(vec![format!("{}:/var/lib/registry", storage.display())]),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        self.engine.start_container(&self.registry).await.unwrap();
        let port = self
            .engine
            .inspect_container(&self.registry)
            .await
            .unwrap()
            .and_then(|info| info.network_settings?.ports?.remove(REGISTRY_PORT)?)
            .and_then(|bindings| {
                bindings
                    .into_iter()
                    .find_map(|b| b.host_port.filter(|p| !p.is_empty()))
            })
            .expect("Docker 给仓库发布了端口");
        self.mirror = format!("127.0.0.1:{port}");
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let Ok(reply) = client
                    .get(format!("http://{}/v2/", self.mirror))
                    .send()
                    .await
                    && reply.status().is_success()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("registry:2 就绪");
    }

    pub async fn push(&mut self, variant: &str) -> String {
        self.engine
            .create_container(
                &self.seed,
                ContainerCreateBody {
                    image: Some(base()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let name = format!("{}/{}", self.mirror, self.repo);
        let options = CommitContainerOptionsBuilder::default()
            .container(&self.seed)
            .repo(&name)
            .tag(variant)
            .pause(false)
            .build();
        let config = ContainerConfig {
            labels: Some(
                [(
                    "wonton.registry-fixture".to_owned(),
                    format!("{}-{variant}", self.repo),
                )]
                .into(),
            ),
            ..Default::default()
        };
        let image = self
            .engine
            .docker
            .commit_container(options, config)
            .await
            .unwrap()
            .id;
        self.images.push(image);
        self.engine.remove_container(&self.seed).await.unwrap();
        let mut push = self.engine.docker.push_image(
            &name,
            Some(PushImageOptionsBuilder::default().tag(variant).build()),
            None,
        );
        while let Some(progress) = push.next().await {
            progress.unwrap();
        }
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client.get(format!("http://{}/v2/{}/manifests/{variant}", self.mirror, self.repo))
            .header("accept", "application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.manifest.v1+json")
            .send().await.unwrap();
        assert!(response.status().is_success(), "{}", response.status());
        response.headers()["docker-content-digest"]
            .to_str()
            .unwrap()
            .to_owned()
    }

    /// containerd 可能跨仓库解析摘要；删掉本测试生成的整个镜像 ID，不能只删 tag。
    pub async fn evict_fixture_images(&self) {
        for id in &self.images {
            match self
                .engine
                .docker
                .remove_image(
                    id,
                    Some(RemoveImageOptionsBuilder::default().force(true).build()),
                    None,
                )
                .await
            {
                Ok(_)
                | Err(bollard::errors::Error::DockerResponseServerError {
                    status_code: 404, ..
                }) => {}
                Err(error) => panic!("删除测试镜像 {id}: {error}"),
            }
        }
    }

    pub async fn remove_manifest(&self, digest: &str) {
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .delete(format!(
                "http://{}/v2/{}/manifests/{digest}",
                self.mirror, self.repo
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 202);
    }

    pub fn compose(&self, digest: &str) -> String {
        format!(
            "services:\n  app:\n    image: {}:fixture@{digest}\n    command: [sh, -c, 'trap \"exit 0\" TERM; sleep 300 & wait']\nx-wonton:\n  spec: '0.5'\n  id: {}\n  version: 1.0.0\n  kind: app\n  title: {{en: Registry}}\n  description: {{en: Registry}}\n  permissions: []\n",
            self.repo, self.id
        )
    }

    pub fn container(&self) -> String {
        self.manager.cfg.instance.container(&self.id.slug(), "app")
    }
}

impl Drop for Fixture {
    /// 镜像不在守卫的标签清理里（不是容器、网络、卷），按 ID 用 CLI 删；容器先删掉，
    /// rmi 不被在用的镜像挡住。
    fn drop(&mut self) {
        for id in std::mem::take(&mut self.images) {
            let _ = std::process::Command::new("docker")
                .args(["rmi", "-f", &id])
                .output();
        }
    }
}
