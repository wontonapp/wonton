//! 调容器资源上限、看哪个容器内存不够被杀（ADR-0037）用到的 Docker 接口。

use std::collections::HashMap;

use bollard::errors::Error as BollardError;
use bollard::models::{ContainerUpdateBody, EventMessage};
use bollard::query_parameters::EventsOptionsBuilder;
use futures_util::Stream;

use super::docker::{Engine, EngineError};

impl Engine {
    /// `POST /containers/{name}/update`：不重启容器，当场生效。
    pub async fn update_container(
        &self,
        name: &str,
        config: ContainerUpdateBody,
    ) -> Result<(), EngineError> {
        self.docker
            .update_container(name, config)
            .await
            .map_err(|e| EngineError::from_bollard(&format!("更新容器 {name}"), e))
    }

    /// 容器内存不够被杀的事件。
    pub fn oom_events(&self) -> impl Stream<Item = Result<EventMessage, BollardError>> + '_ {
        let filters = HashMap::from([
            ("type".to_owned(), vec!["container".to_owned()]),
            ("event".to_owned(), vec!["oom".to_owned()]),
        ]);
        self.docker.events(Some(
            EventsOptionsBuilder::default().filters(&filters).build(),
        ))
    }
}
