//! Account-scoped sharing links, independent of the paired HomeLink.
mod discovery;
pub(crate) mod hub;
mod source;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_lifecycle;
#[cfg(test)]
mod tests_tls;

use crate::{chat::ChatLink, plugins::PluginsLink};
use source::Source;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};
use tokio::{
    runtime::Handle,
    sync::{mpsc, watch},
};
use wonton_matrix::MatrixSession;
use wonton_protocol::{PluginsViewModel, ShareFailure, ShareProblem, SharedAppsState};

pub(crate) struct SharedAppsLink {
    commands: mpsc::UnboundedSender<Command>,
    hub: Arc<wonton_tunnel::sharing::ProxyHub>,
    chat: watch::Sender<wonton_protocol::ChatViewModel>,
}

/// Discover a source over certificate-validated HTTPS without redirects or proxies.
/// WONTON_SHARE_CA_FILE may explicitly add a valid PEM CA for a private deployment.
pub async fn discover_share_source(
    server: &str,
) -> Result<Option<iroh::EndpointAddr>, ShareProblem> {
    let client = discovery::http_client().await?;
    Ok(discovery::discover(&client, server)
        .await?
        .map(|source| source.target))
}

enum Command {
    Refresh(Option<String>),
    Probe(String, String),
}

impl SharedAppsLink {
    pub(crate) fn new(
        runtime: Handle,
        chat: &Arc<ChatLink>,
        plugins: &Arc<PluginsLink>,
        hub: Arc<wonton_tunnel::sharing::ProxyHub>,
    ) -> Arc<Self> {
        let (commands, receiver) = mpsc::unbounded_channel();
        let link = Arc::new(Self {
            commands,
            hub,
            chat: chat.sender(),
        });
        runtime.spawn(follow(
            Arc::downgrade(&link),
            chat.sharing_sessions(),
            chat.subscribe(),
            plugins.sender(),
            receiver,
        ));
        link
    }

    pub(crate) fn refresh(&self, owner: Option<String>) {
        let _ = self.commands.send(Command::Refresh(owner));
    }

    pub(crate) fn probe(&self, owner: String, plugin: String) {
        let _ = self.commands.send(Command::Probe(owner, plugin));
    }
}

async fn follow(
    link: Weak<SharedAppsLink>,
    mut sessions: watch::Receiver<Option<Arc<MatrixSession>>>,
    mut rooms: watch::Receiver<wonton_protocol::ChatViewModel>,
    view: watch::Sender<PluginsViewModel>,
    mut commands: mpsc::UnboundedReceiver<Command>,
) {
    let mut sources = HashMap::new();
    loop {
        let session = sessions.borrow_and_update().clone();
        let work = async {
            if let Some(session) = &session {
                publish(&view, &sources);
                refresh(&link, session, &view, &mut sources, None).await;
            }
            loop {
                tokio::select! {
                    changed = rooms.changed() => {
                        if changed.is_err() { return; }
                        if let Some(session) = &session {
                            reconcile(&link, session, &view, &mut sources).await;
                        }
                    }
                    command = commands.recv() => match command {
                        Some(Command::Refresh(owner)) => if let Some(session) = &session {
                            refresh(&link, session, &view, &mut sources, owner.as_deref()).await;
                        },
                        Some(Command::Probe(owner, plugin)) => {
                            if let Some(source) = sources.get_mut(&owner) { source.probe(&plugin).await; }
                            publish(&view, &sources);
                        }
                        None => return,
                    }
                }
            }
        };
        tokio::select! {
            changed = sessions.changed() => if changed.is_err() { return; },
            _ = work => return,
        }
        // Cancellation drops in-flight tokens/proxies before the next account is published.
        sources.clear();
        publish(&view, &sources);
    }
}

async fn reconcile(
    link: &Weak<SharedAppsLink>,
    session: &MatrixSession,
    view: &watch::Sender<PluginsViewModel>,
    sources: &mut HashMap<String, Source>,
) {
    let friends = match session.friend_sources().await {
        Ok(friends) => friends,
        Err(_) => {
            for source in sources.values_mut() {
                source.fail(problem(ShareFailure::Unreachable, None));
            }
            publish(view, sources);
            return;
        }
    };
    sources.retain(|owner, _| {
        friends
            .iter()
            .any(|friend| friend.user_id.as_str() == owner)
    });
    publish(view, sources);
    for friend in friends {
        if !sources.contains_key(friend.user_id.as_str()) {
            refresh_one(link, session, view, sources, friend).await;
        }
    }
}

async fn refresh(
    link: &Weak<SharedAppsLink>,
    session: &MatrixSession,
    view: &watch::Sender<PluginsViewModel>,
    sources: &mut HashMap<String, Source>,
    owner: Option<&str>,
) {
    let friends = match session.friend_sources().await {
        Ok(friends) => friends,
        Err(_) => {
            for source in sources.values_mut() {
                source.fail(problem(ShareFailure::Unreachable, None));
            }
            publish(view, sources);
            return;
        }
    };
    sources.retain(|id, _| friends.iter().any(|friend| friend.user_id.as_str() == id));
    publish(view, sources);
    for friend in friends {
        if owner.is_none_or(|id| friend.user_id.as_str() == id) {
            refresh_one(link, session, view, sources, friend).await;
        }
    }
}

async fn refresh_one(
    link: &Weak<SharedAppsLink>,
    session: &MatrixSession,
    view: &watch::Sender<PluginsViewModel>,
    sources: &mut HashMap<String, Source>,
    friend: wonton_matrix::FriendSource,
) {
    let owner = friend.user_id.to_string();
    let mut source = sources
        .remove(&owner)
        .unwrap_or_else(|| Source::new(&friend));
    source.view.owner_display_name = friend.display_name;
    source.begin();
    view.send_modify(|view| {
        if let Some(shared) = &mut view.shared_apps
            && let Some(row) = shared
                .sources
                .iter_mut()
                .find(|row| row.owner_user_id == owner)
        {
            row.state = SharedAppsState::Loading;
        }
    });
    let Some(link_owner) = link.upgrade() else {
        return;
    };
    if let Some(result) = source
        .connect(session, &link_owner.hub, &link_owner.chat)
        .await
    {
        if let Err(problem) = result {
            source.fail(problem);
        }
        source.watch_closed(view.clone());
        sources.insert(owner, source);
    }
    publish(view, sources);
}

fn publish(view: &watch::Sender<PluginsViewModel>, sources: &HashMap<String, Source>) {
    let mut rows: Vec<_> = sources.values().map(Source::snapshot).collect();
    rows.sort_by(|a, b| a.owner_user_id.cmp(&b.owner_user_id));
    view.send_modify(|view| {
        view.shared_apps = Some(wonton_protocol::SharedAppsViewModel { sources: rows })
    });
}

pub(super) fn problem(reason: ShareFailure, http_status: Option<u16>) -> ShareProblem {
    ShareProblem {
        reason,
        http_status,
        identity: None,
    }
}

pub(super) fn failed(problem: ShareProblem) -> SharedAppsState {
    SharedAppsState::Failed { problem }
}
