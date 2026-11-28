//! 自己这边的身份：配对时发给对方的服务器名、主人和回连地址。

use wonton_tunnel::peer_pair::PeerIdentity;

use super::Peers;
use crate::tunnel::Tunnel;

/// 自己这边的身份，配对时发给对方。主人的显示名不在这里：它在每次握手时现取（主人会改资料）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnIdentity {
    pub server_name: String,
    pub owner_user_id: String,
}

impl OwnIdentity {
    pub(crate) fn to_wire(&self, tunnel: &Tunnel, owner_display_name: String) -> PeerIdentity {
        PeerIdentity {
            server_name: self.server_name.clone(),
            owner_user_id: self.owner_user_id.clone(),
            owner_display_name,
            relays: tunnel
                .relays()
                .iter()
                .map(|relay| wonton_tunnel::relays::relay_url_text(&relay.url))
                .collect(),
            addrs: tunnel.direct_addrs(),
        }
    }
}

impl Peers {
    /// 这次的配对握手里要发给对方的身份。主人的显示名现取，主人改资料后不用重启核心。
    pub(crate) fn own_identity(&self) -> PeerIdentity {
        self.own.to_wire(&self.tunnel, (self.owner_display_name)())
    }
}
