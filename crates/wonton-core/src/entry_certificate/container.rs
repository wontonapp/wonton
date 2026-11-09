use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use bollard::models::{ContainerCreateBody, HostConfig, Mount, MountType};
use bollard::query_parameters::WaitContainerOptionsBuilder;
use futures_util::StreamExt;

use crate::plugin::compose::PullPolicy;

use super::{CertificateManager, LEGO_IMAGE};

const COMPLETION_TIMEOUT: Duration = Duration::from_secs(600);

impl CertificateManager {
    pub(super) async fn run_lego(&self, command: Vec<String>) -> Result<()> {
        let image = self
            .engine
            .ensure_image(LEGO_IMAGE, PullPolicy::Missing, None)
            .await?;
        let mut random = [0u8; 12];
        getrandom::fill(&mut random).context("generate ACME container name")?;
        let nonce: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let name = format!("wonton-acme-{nonce}");
        let mut body = self.container_body(command)?;
        body.image = Some(image);
        self.engine.create_container(&name, body).await?;
        // Cleanup is attempted after every start/wait outcome. A cleanup failure is
        // never hidden, since the container holds ACME account/DNS capabilities.
        let outcome = async {
            self.engine.start_container(&name).await?;
            self.wait_lego(&name).await
        }
        .await;
        let removed = self.engine.remove_container(&name).await;
        match (outcome, removed) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(error).context("remove ACME helper container"),
            (Err(error), Err(cleanup)) => {
                Err(error.context(format!("ACME helper cleanup also failed: {cleanup}")))
            }
        }
    }

    async fn wait_lego(&self, name: &str) -> Result<()> {
        let docker = self.engine.docker.clone().with_timeout(COMPLETION_TIMEOUT);
        let options = WaitContainerOptionsBuilder::default()
            .condition("not-running")
            .build();
        let mut stream = docker.wait_container(name, Some(options));
        let result = tokio::time::timeout(COMPLETION_TIMEOUT, stream.next()).await;
        let error = match result {
            Ok(Some(Ok(response))) if response.status_code == 0 => return Ok(()),
            Ok(Some(Ok(response))) => format!("lego exited with status {}", response.status_code),
            Ok(Some(Err(error))) => format!("ACME helper wait failed: {error}"),
            Ok(None) => "ACME helper wait ended without an exit status".to_owned(),
            Err(_) => "ACME helper exceeded its 600-second deadline".to_owned(),
        };
        let logs = self.engine.logs_tail(name, 80).await;
        bail!("{error}\n{logs}")
    }

    pub(super) fn container_body(&self, command: Vec<String>) -> Result<ContainerCreateBody> {
        let host_entry = self.config.host_install_dir.join("entry");
        let acme = std::fs::metadata(self.entry().join("acme"))?;
        let mounts = vec![
            bind_mount(host_entry.join("acme"), "/acme", false)?,
            bind_mount(
                host_entry.join("requests/site.csr"),
                "/request/site.csr",
                true,
            )?,
        ];
        let mut environment = vec!["HOME=/tmp".to_owned(), "LEGO_LOG_FORMAT=text".to_owned()];
        if self.config.ca_file.is_some() {
            // LEGO_CA_CERTIFICATES covers ACME; SSL_CERT_FILE also covers the
            // acme-dns provider's separate Go HTTP client. Neither skips TLS.
            environment.extend([
                "LEGO_CA_CERTIFICATES=/acme/ca.pem".to_owned(),
                "SSL_CERT_FILE=/acme/ca.pem".to_owned(),
            ]);
        }
        // No cert directory, leaf key, Docker socket or parent directory mount.
        Ok(ContainerCreateBody {
            image: Some(LEGO_IMAGE.to_owned()),
            cmd: Some(command),
            user: Some(format!("{}:{}", acme.uid(), acme.gid())),
            working_dir: Some("/acme".to_owned()),
            env: Some(environment),
            labels: Some(HashMap::from([(
                "app.wonton.component".to_owned(),
                "acme".to_owned(),
            )])),
            host_config: Some(HostConfig {
                mounts: Some(mounts),
                network_mode: Some(self.config.network.clone()),
                // 显式配了解析器时用它：ACME 目录、acme-dns 的更新接口和 CAA 都在这些名字上。
                // 配置的解析器在前，Docker 的 127.0.0.11 兜底内部别名：前者对区外名字回 REFUSED，
                // 后者认不出才回 NXDOMAIN。
                dns: Some(
                    self.config
                        .dns_resolvers
                        .iter()
                        .filter_map(|server| {
                            server
                                .parse::<std::net::SocketAddr>()
                                .ok()
                                .map(|address| address.ip().to_string())
                        })
                        .chain(std::iter::once("127.0.0.11".to_owned()))
                        .collect(),
                ),
                readonly_rootfs: Some(true),
                cap_drop: Some(vec!["ALL".to_owned()]),
                security_opt: Some(vec!["no-new-privileges:true".to_owned()]),
                tmpfs: Some(HashMap::from([(
                    "/tmp".to_owned(),
                    "rw,noexec,nosuid,nodev,size=16m,mode=1777".to_owned(),
                )])),
                pids_limit: Some(128),
                ..Default::default()
            }),
            ..Default::default()
        })
    }
}

fn bind_mount(source: std::path::PathBuf, target: &str, read_only: bool) -> Result<Mount> {
    let source = source
        .to_str()
        .ok_or_else(|| anyhow!("certificate Docker mount path is not UTF-8"))?;
    Ok(Mount {
        typ: Some(MountType::BIND),
        source: Some(source.to_owned()),
        target: Some(target.to_owned()),
        read_only: Some(read_only),
        ..Default::default()
    })
}
