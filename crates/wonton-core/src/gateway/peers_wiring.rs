//! 网关上服务器配对和 Bot 联系人的接线（字段在 [`Gateway`] 上，构造在这里）。

use std::sync::Arc;

use super::Gateway;

impl Gateway {
    #[must_use]
    pub fn with_peers(mut self, peers: Option<Arc<crate::peers::Peers>>) -> Self {
        self.peers = peers;
        self
    }

    #[must_use]
    pub fn with_contacts(mut self, contacts: Option<Arc<crate::contacts::Contacts>>) -> Self {
        self.contacts = contacts;
        self
    }
}
