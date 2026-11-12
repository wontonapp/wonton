use std::os::unix::fs::MetadataExt;
use std::time::Duration;

use bollard::models::{ContainerConfig, ContainerCreateBody, HostConfig};
use bollard::query_parameters::{
    CommitContainerOptionsBuilder, PushImageOptionsBuilder, RemoveImageOptionsBuilder,
};
use futures_util::StreamExt;

use crate::plugin::PluginManager;
use crate::plugin::compose::PullPolicy;
use crate::plugin::docker::Engine;
use crate::plugin::manifest::PluginId;

const BASE: &str = "public.ecr.aws/docker/library/busybox:1.37";
const REGISTRY: &str = "public.ecr.aws/docker/library/registry:2";

pub(super) struct Fixture {
    pub engine: Engine,
    pub dir: tempfile::TempDir,
    pub manager: PluginManager,
    pub mirror: String,
    pub repo: String,
    pub id: PluginId,
    registry: String,
    seed: String,
    pub images: Vec<String>,
}

impl Fixture {
    pub fn new() -> Self {
        let engine = Engine::connect().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let manager = super::test_support::manager(engine.clone(), &dir);
        let unique = getrandom::u32().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mirror = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        Self {
            engine,
            dir,
            manager,
            mirror,
            repo: format!("wonton-emcm/fixture{unique:x}"),
            id: PluginId::parse(&format!("app.registry{unique:x}")).unwrap(),
            registry: format!("wonton-emcm-registry-{unique:x}"),
            seed: format!("wonton-emcm-seed-{unique:x}"),
            images: Vec::new(),
        }
    }

    pub async fn start(&self) {
        for image in [REGISTRY, BASE] {
            self.engine
                .ensure_image(image, PullPolicy::Missing, None)
                .await
                .unwrap();
        }
        let storage = self.dir.path().join("registry");
        std::fs::create_dir(&storage).unwrap();
        let owner = std::fs::metadata(&storage).unwrap();
        self.engine
            .create_container(
                &self.registry,
                ContainerCreateBody {
                    image: Some(REGISTRY.to_owned()),
                    user: Some(format!("{}:{}", owner.uid(), owner.gid())),
                    env: Some(vec![
                        format!("REGISTRY_HTTP_ADDR={}", self.mirror),
                        "REGISTRY_STORAGE_DELETE_ENABLED=true".into(),
                    ]),
                    host_config: Some(HostConfig {
                        network_mode: Some("host".into()),
                        binds: Some(vec![format!(
                            "{}:/var/lib/registry",
                            self.dir.path().join("registry").display()
                        )]),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        self.engine.start_container(&self.registry).await.unwrap();
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
                    image: Some(BASE.to_owned()),
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

    pub async fn clean(&self) {
        self.engine
            .remove_container(&self.container())
            .await
            .unwrap();
        self.engine
            .remove_network(&self.manager.cfg.instance.network(&self.id.slug()))
            .await
            .unwrap();
        self.engine.remove_container(&self.seed).await.unwrap();
        self.engine.remove_container(&self.registry).await.unwrap();
        self.evict_fixture_images().await;
    }
}
