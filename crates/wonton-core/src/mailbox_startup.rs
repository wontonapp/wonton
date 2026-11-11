use std::sync::Arc;

use anyhow::Context;
use wonton_core::config::Config;
use wonton_core::gateway::Gateway;
use wonton_core::http::HttpClient;
use wonton_core::mailbox::{self, Component, Worker};
use wonton_core::members::Members;
use wonton_core::plugin::docker::Engine;

pub(super) fn open_mailpit(
    cfg: &Config,
    engine: &Engine,
    members: Arc<Members>,
) -> anyhow::Result<Option<Arc<Component>>> {
    if cfg.mailpit.is_none() {
        return Ok(None);
    }
    let container = cfg
        .core_container
        .clone()
        .context("配置了 Mailpit，但缺少 WONTON_CORE_CONTAINER，无法管理收件组件")?;
    Component::open(
        engine.clone(),
        container,
        &cfg.core_state_dir(),
        &cfg.username,
        members,
        cfg.puid,
        cfg.pgid,
    )
    .map(Some)
}

pub(super) fn start_mailpit(
    cfg: &Config,
    component: Option<&Arc<Component>>,
    http: HttpClient,
    gateway: Arc<Gateway>,
) -> anyhow::Result<Option<Worker>> {
    let Some(authority) = cfg.mailpit.as_deref() else {
        return Ok(None);
    };
    let component = component.context("Mailpit 未初始化")?;
    component.spawn_members(
        gateway.notices.clone(),
        super::owner_languages(&gateway.plugins),
    );
    mailbox::start(
        authority,
        component.password(),
        http,
        gateway,
        &cfg.core_state_dir(),
    )
    .map(Some)
}
