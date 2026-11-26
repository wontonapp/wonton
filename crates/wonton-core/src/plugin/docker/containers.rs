//! 容器的操作：创建、启停、删、查、重启，按标签列出属于某个插件的容器、网络和卷。

use std::collections::HashMap;

use bollard::models::{ContainerCreateBody, ContainerInspectResponse};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, KillContainerOptionsBuilder, ListContainersOptionsBuilder,
    ListNetworksOptionsBuilder, ListVolumesOptionsBuilder, RemoveContainerOptionsBuilder,
    StopContainerOptionsBuilder,
};

use super::super::languages::Languages;
use super::{Engine, EngineError, LabeledResources, Result, ignore_missing};

impl Engine {
    pub async fn create_container(&self, name: &str, body: ContainerCreateBody) -> Result<()> {
        if let Some(creations) = &self.creations {
            let command = body.cmd.clone().unwrap_or_default().join(" ");
            creations.lock().unwrap().push(command);
        }
        let options = CreateContainerOptionsBuilder::default().name(name).build();
        let response = self
            .patient()
            .create_container(Some(options), body)
            .await
            .map_err(|e| {
                self.fail_name("plugin-install-docker-context-create-container", name, e)
            })?;
        for warning in response.warnings {
            tracing::warn!(container = name, warning = %warning, "Docker 创建容器时给出警告");
        }
        Ok(())
    }

    pub async fn start_container(&self, name: &str) -> Result<()> {
        self.docker
            .start_container(name, None)
            .await
            .map_err(|e| self.fail_name("plugin-install-docker-context-start-container", name, e))
    }
    /// 给容器发信号（如 `SIGUSR1`）；返回错误时信号可能已经发出，调用方不要盲目重试。
    pub async fn kill_container(&self, name: &str, signal: &str) -> Result<()> {
        let options = KillContainerOptionsBuilder::default()
            .signal(signal)
            .build();
        self.docker
            .kill_container(name, Some(options))
            .await
            .map_err(|e| {
                self.fail(
                    "plugin-install-docker-context-kill-container",
                    &[("name", name.into()), ("signal", signal.into())],
                    e,
                )
            })
    }

    /// 查询容器；不存在时返回 `None`。
    pub async fn inspect_container(&self, name: &str) -> Result<Option<ContainerInspectResponse>> {
        match self.docker.inspect_container(name, None).await {
            Ok(info) => Ok(Some(info)),
            Err(e) => {
                match self.fail_name("plugin-install-docker-context-inspect-container", name, e) {
                    EngineError::NotFound(_) => Ok(None),
                    other => Err(other),
                }
            }
        }
    }

    /// 重启容器：dockerd 会重新解析设备节点上的 by-id，拔插后编号变了也拿到新节点（ADR-0036 的实测）。
    pub async fn restart_container(&self, name: &str, _langs: &Languages) -> Result<()> {
        self.patient()
            .restart_container(name, None)
            .await
            .map_err(|error| {
                self.fail_name(
                    "plugin-install-docker-context-restart-container",
                    name,
                    error,
                )
            })
    }

    /// 停止但保留容器及卷；已停止视为成功。
    pub async fn stop_container(&self, name: &str) -> Result<()> {
        let stop = StopContainerOptionsBuilder::default().t(10).build();
        match self.patient().stop_container(name, Some(stop)).await {
            Ok(()) => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 304, ..
            }) => Ok(()),
            Err(error) => {
                Err(self.fail_name("plugin-install-docker-context-stop-container", name, error))
            }
        }
    }

    /// 停止并删除容器（连同匿名卷）；不存在视为成功。
    pub async fn remove_container(&self, name: &str) -> Result<()> {
        let stop = StopContainerOptionsBuilder::default().t(10).build();
        let docker = self.patient();
        let stopped = docker
            .stop_container(name, Some(stop))
            .await
            .map_err(|e| self.fail_name("plugin-install-docker-context-stop-container", name, e));
        match stopped {
            Ok(()) | Err(EngineError::NotFound(_)) | Err(EngineError::Rejected { .. }) => {}
            Err(other) => return Err(other),
        }
        let options = RemoveContainerOptionsBuilder::default()
            .force(true)
            .v(true)
            .build();
        ignore_missing(
            docker
                .remove_container(name, Some(options))
                .await
                .map_err(|e| {
                    self.fail_name("plugin-install-docker-context-remove-container", name, e)
                }),
        )
    }

    /// 列出带 `key=value` 标签、其余标签又让 `keep` 认可的容器、网络和卷（容器包括已停止的）。Docker 的标签过滤
    /// 只能要求有某个标签，不能要求没有，所以 `keep` 在这里筛。
    pub async fn labeled(
        &self,
        key: &str,
        value: &str,
        keep: impl Fn(&HashMap<String, String>) -> bool,
    ) -> Result<LabeledResources> {
        let filters = HashMap::from([("label".to_owned(), vec![format!("{key}={value}")])]);
        let kept =
            |labels: Option<&HashMap<String, String>>| keep(labels.unwrap_or(&HashMap::new()));
        let containers = self
            .docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default()
                    .all(true)
                    .filters(&filters)
                    .build(),
            ))
            .await
            .map_err(|e| self.fail("plugin-install-docker-context-list-containers", &[], e))?
            .into_iter()
            .filter(|c| kept(c.labels.as_ref()))
            .filter_map(|c| {
                c.names
                    .and_then(|n| n.first().map(|n| n.trim_start_matches('/').to_owned()))
            })
            .collect();
        let networks = self
            .docker
            .list_networks(Some(
                ListNetworksOptionsBuilder::default()
                    .filters(&filters)
                    .build(),
            ))
            .await
            .map_err(|e| self.fail("plugin-install-docker-context-list-networks", &[], e))?
            .into_iter()
            .filter(|n| kept(n.labels.as_ref()))
            .filter_map(|n| n.name)
            .collect();
        let volumes = self
            .docker
            .list_volumes(Some(
                ListVolumesOptionsBuilder::default()
                    .filters(&filters)
                    .build(),
            ))
            .await
            .map_err(|e| self.fail("plugin-install-docker-context-list-volumes", &[], e))?
            .volumes
            .unwrap_or_default()
            .into_iter()
            .filter(|v| kept(Some(&v.labels)))
            .map(|v| v.name)
            .collect();
        Ok(LabeledResources {
            containers,
            networks,
            volumes,
        })
    }
}
