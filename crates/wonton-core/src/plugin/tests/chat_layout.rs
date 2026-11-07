//! 插件声明的聊天显示方式（ADR-0060）：随插件列表给出，没有 Bot 的插件不给。

use std::sync::Arc;

use wonton_protocol::{ChatForm, PluginChat};

use super::super::docker::Engine;
use super::super::fake_docker::FakeDocker;
use super::super::{Permission, PluginId, PluginManager};
use super::manager::manager_in_container;
use super::{langs, package};
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::fake::{Fake, start};

fn id() -> PluginId {
    PluginId::parse("app.test").unwrap()
}

fn bot(extra: &str) -> String {
    package(
        "  bot:\n    image: echo",
        &format!(
            "  bot:\n    display_name: {{zh: 回声, en: Echo}}{extra}\n  permissions: [matrix.bot]"
        ),
    )
}

struct Bed {
    _docker: FakeDocker,
    engine: Engine,
    _tuwunel: Fake,
    matrix: Arc<Matrix>,
    dir: tempfile::TempDir,
}

impl Bed {
    async fn start() -> Self {
        let (docker, engine) = FakeDocker::start().await;
        let (tuwunel, config) = start().await;
        let dir = tempfile::tempdir().unwrap();
        let matrix = Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
            .await
            .unwrap();
        Self {
            _docker: docker,
            engine,
            _tuwunel: tuwunel,
            matrix: Arc::new(matrix),
            dir,
        }
    }

    fn manager(&self) -> PluginManager {
        manager_in_container(self.engine.clone(), &self.dir).with_matrix(Some(self.matrix.clone()))
    }
}

fn granted(compose: &str) -> Vec<Permission> {
    super::super::package::PluginPackage::parse(compose, &langs())
        .unwrap()
        .manifest
        .permissions
        .to_vec()
}

async fn installed_chat(compose: &str) -> Option<PluginChat> {
    let bed = Bed::start().await;
    let m = bed.manager();
    let grant = granted(compose);
    m.install(compose, &grant, langs()).await.unwrap();
    m.installed_plugin(&id(), &langs()).unwrap().chat
}

#[tokio::test]
async fn a_declared_form_reaches_the_plugin_list() {
    let chat = installed_chat(&bot("\n    chat_layout: document")).await;
    assert_eq!(
        chat,
        Some(PluginChat {
            form: ChatForm::Document
        })
    );
}

#[tokio::test]
async fn an_undeclared_form_is_bubbles() {
    let chat = installed_chat(&bot("")).await;
    assert_eq!(
        chat,
        Some(PluginChat {
            form: ChatForm::Bubbles
        })
    );
}

#[tokio::test]
async fn a_plugin_without_bot_has_no_chat() {
    let compose = package("  web:\n    image: echo", "  permissions: []");
    assert_eq!(installed_chat(&compose).await, None);
}
