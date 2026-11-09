//! 已配对设备的 endpoint ID 白名单。

use std::{collections::HashSet, sync::Arc};

use iroh::EndpointId;
use tokio::sync::watch;

/// 已配对设备的 endpoint ID 白名单，运行时可以增删。
///
/// 服务端只接受白名单里的设备；吊销一台设备会立即断开它现有的隧道连接。克隆得到的是同一份白名单。
#[derive(Debug, Clone)]
pub struct Allowlist {
    ids: Arc<watch::Sender<HashSet<EndpointId>>>,
}

impl Allowlist {
    /// 用一组已配对设备创建白名单。
    pub fn new(ids: impl IntoIterator<Item = EndpointId>) -> Self {
        let (ids, _) = watch::channel(ids.into_iter().collect());
        Self { ids: Arc::new(ids) }
    }

    /// 加入一台设备。返回它之前是否不在白名单里。
    pub fn allow(&self, id: EndpointId) -> bool {
        self.ids.send_if_modified(|ids| ids.insert(id))
    }

    /// 吊销一台设备，并断开它现有的连接。返回它之前是否在白名单里。
    pub fn revoke(&self, id: &EndpointId) -> bool {
        self.ids.send_if_modified(|ids| ids.remove(id))
    }

    /// 这台设备当前是否已配对。
    pub fn contains(&self, id: &EndpointId) -> bool {
        self.ids.borrow().contains(id)
    }

    /// 订阅白名单变化。watch 只保留最新值，订阅方处理慢也不会积压。
    pub(crate) fn subscribe(&self) -> watch::Receiver<HashSet<EndpointId>> {
        self.ids.subscribe()
    }
}

impl Default for Allowlist {
    fn default() -> Self {
        Self::new([])
    }
}
