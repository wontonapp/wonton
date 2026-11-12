//! 配过的服务器列表（ADR-0042）。只记一台：列表就是当前连着的这台，添加、切换、改名和移除都还做不到。

use tokio::runtime::Handle;
use tokio::sync::watch;
use wonton_protocol::{CoreError, HomeViewModel, ServerRow, ServersViewModel};

pub(crate) struct ServersLink {
    view: watch::Sender<ServersViewModel>,
}

impl ServersLink {
    pub(crate) fn new(runtime: &Handle, mut home: watch::Receiver<HomeViewModel>) -> Self {
        let view = watch::Sender::new(servers_view(&home.borrow()));
        let sender = view.clone();
        runtime.spawn(async move {
            while home.changed().await.is_ok() {
                let next = servers_view(&home.borrow());
                sender.send_if_modified(|view| {
                    let modified = *view != next;
                    *view = next;
                    modified
                });
            }
        });
        Self { view }
    }

    pub(crate) fn view(&self) -> ServersViewModel {
        self.view.borrow().clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<ServersViewModel> {
        self.view.subscribe()
    }

    /// 添加、切换、改名和移除都要按服务器分开存数据，没做到之前如实拒绝。
    pub(crate) fn unsupported() -> Result<(), CoreError> {
        Err(CoreError::NotReady {
            reason: "这个版本还只能连一台服务器".to_owned(),
        })
    }
}

fn servers_view(home: &HomeViewModel) -> ServersViewModel {
    ServersViewModel {
        servers: home
            .server_id
            .iter()
            .map(|id| ServerRow {
                id: id.clone(),
                name: "家里的服务器".to_owned(),
                current: true,
            })
            .collect(),
        adding: false,
    }
}
