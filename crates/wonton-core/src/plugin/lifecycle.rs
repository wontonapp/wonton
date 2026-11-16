//! 部署、恢复与卸载：把部署计划交给 Docker，失败时按相反顺序回滚。

mod config;
mod context;
mod deploy;
mod grants;
mod network;
mod start;
mod teardown;
mod undo;

mod oidc_client;
mod prepare;
pub(super) mod rollback;
pub(super) mod volumes;

pub use config::LifecycleConfig;
pub(super) use grants::Grants;
pub(super) use oidc_client::{client_spec, client_spec_at};
pub(super) use undo::Undo;
