mod admin;
mod api;
mod auth;
mod config;
mod dns;
mod hooks;
mod metrics;
mod service;
mod store;

use anyhow::Result;
use service::App;
use std::{path::PathBuf, sync::Arc, time::Duration};

async fn background(app: Arc<App>) -> Result<()> {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;
        // Each tick is independent accounting/reconciliation, not a hidden request retry.
        if let Err(error) = app.poll_metrics().await {
            tracing::error!(%error, "relay accounting unavailable; admission will fail closed");
        }
        if let Err(error) = app.reconcile().await {
            tracing::error!(%error, "DNS reconciliation failed");
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() && args != ["serve"] {
        let state = PathBuf::from(config::required("WONTON_HOME_STATE")?);
        return admin::client(&state, &args).await;
    }
    let config = config::Config::from_env()?;
    let store = store::Store::open(config.state.clone())?;
    let app = App::new(config, store)?;
    app.dns.ensure_zone().await?;
    app.reconcile().await?;
    app.poll_metrics().await?;
    let public = tokio::net::TcpListener::bind(&app.config.listen).await?;
    let hooks = tokio::net::TcpListener::bind(&app.config.hook_listen).await?;
    tokio::select! {
        result = axum::serve(public, app.router()) => result?,
        result = axum::serve(hooks, app.hooks()) => result?,
        result = admin::serve(app.clone()) => result?,
        result = background(app.clone()) => result?,
        _ = tokio::signal::ctrl_c() => {},
    }
    Ok(())
}

#[cfg(test)]
mod tests;
