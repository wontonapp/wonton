//! 测试里的家：隧道、配对、推送登记和 Bot 账号，连的都是 Tuwunel 那一侧。

use std::sync::Mutex;
use std::time::Duration;

use iroh::EndpointId;
use serde_json::Value;
use tokio::sync::watch;
use wonton_matrix::MatrixSession;
use wonton_tunnel::pair::{
    PairHandler, PairProtocol, PairResponse, PairingCode, PairingSecret, SECRET_LEN,
};
use wonton_tunnel::{Allowlist, TunnelServer};

use super::{Home, loopback_endpoint, relay};

pub(crate) const BASE_DOMAIN: &str = "wonton.localhost";
/// 和 deploy/dev/matrix/tuwunel.toml 的 server_name 一致。
pub const OWNER: &str = "@owner:wonton.localhost";

impl Home {
    /// 家里的隧道断了：手机连不上，Tuwunel 和直连它的 Bot 不受影响。
    /// 隧道的 endpoint ID，推送里写的就是它。
    pub fn server_id(&self) -> String {
        self.tunnel_key.public().to_string()
    }

    pub async fn stop_tunnel(&mut self) {
        self.tunnel.take().expect("隧道没在运行").shutdown().await;
    }

    /// 用同样的身份和端口恢复隧道。
    pub async fn start_tunnel(&mut self) {
        let endpoint = loopback_endpoint(self.tunnel_key.clone(), self.tunnel_port).await;
        let (tunnel, listener) = TunnelServer::spawn_embedded_with_pairing(
            endpoint,
            self.pairing.allowlist.clone(),
            PairProtocol::new(self.pairing.clone()),
        );
        self.tunnel = Some(tunnel);
        self.relay = relay(listener, self.core_addr);
    }

    /// 同样的身份换一个端口，手机看来就是这台服务器换了地址。
    pub async fn move_tunnel(&mut self) {
        self.stop_tunnel().await;
        let endpoint = loopback_endpoint(self.tunnel_key.clone(), 0).await;
        self.tunnel_port = endpoint.bound_sockets()[0].port();
        let (tunnel, listener) = TunnelServer::spawn_embedded_with_pairing(
            endpoint,
            self.pairing.allowlist.clone(),
            PairProtocol::new(self.pairing.clone()),
        );
        self.tunnel = Some(tunnel);
        self.relay = relay(listener, self.core_addr);
    }
    /// 一个配对码，扫了会和这台服务器配对。
    pub fn pairing_code(&self) -> String {
        let tunnel = self.tunnel.as_ref().expect("隧道没在运行");
        let secret = PairingSecret::from_bytes([7; SECRET_LEN]);
        PairingCode {
            server_id: tunnel.endpoint().id(),
            relays: Vec::new(),
            direct_addrs: tunnel.endpoint().bound_sockets(),
            base_domain: BASE_DOMAIN.to_owned(),
            core_url: url::Url::parse(&format!("http://{BASE_DOMAIN}/")).unwrap(),
            secret,
        }
        .to_uri()
    }
    /// 登记推送时核心回给设备的 pushkey。
    pub fn pushkey(&self) -> &str {
        &self.core.pushkey
    }

    /// 推送登记接口收到的请求。
    pub fn push_requests(&self) -> Vec<(String, Value)> {
        self.core.push_requests.lock().unwrap().clone()
    }
    /// 挂起（`true`）或放行（`false`）推送登记接口的响应；挂起期间请求照记不误。
    pub fn hold_push_registrations(&self, hold: bool) {
        self.core.hold_push_registrations.send_replace(hold);
    }

    /// 下一个配对请求的回应晚 [`SLOW_PAIR`] 才发：核心成功换槽之后、服务端结论之前。
    pub fn slow_next_pairing(&self) {
        *self.pairing.slow.lock().unwrap() = 1;
    }

    /// 订阅推送登记请求的到达次数；等的一方靠事件变化再查，不睡固定间隔。
    pub fn push_registrations_seen(&self) -> watch::Receiver<u64> {
        self.core.push_registrations_seen.subscribe()
    }
    pub fn ticket(&self) -> String {
        let tunnel = self.tunnel.as_ref().expect("隧道没在运行");
        let id = tunnel.endpoint().id();
        let addr = tunnel.endpoint().bound_sockets()[0];
        format!(
            r#"{{"server": {{"endpoint_id": "{id}", "direct_addrs": ["{addr}"]}},
                "base_domain": "{BASE_DOMAIN}", "core_url": "http://{BASE_DOMAIN}"}}"#
        )
    }
    /// 一个 Bot 账号：直接连 Tuwunel（Bot 跑在家里的服务器上，不经隧道），设好显示名并开始同步。
    pub async fn bot(&self, localpart: &str, display_name: &str) -> MatrixSession {
        let bot = self.matrix.login(localpart, display_name).await;
        bot.set_display_name(display_name)
            .await
            .expect("Bot 设置显示名");
        bot.start_sync().await.expect("Bot 开始同步");
        bot
    }

    /// 同 [`Self::bot`]，但登录走 `homeserver_url`（比如 [`ToDeviceHold`] 的中转），之后的请求都走它。
    pub async fn bot_via(
        &self,
        homeserver_url: &str,
        localpart: &str,
        display_name: &str,
    ) -> MatrixSession {
        let bot = self
            .matrix
            .login_via(
                homeserver_url,
                localpart,
                display_name,
                wonton_matrix::StoreConfig::InMemory,
            )
            .await;
        bot.set_display_name(display_name)
            .await
            .expect("Bot 设置显示名");
        bot.start_sync().await.expect("Bot 开始同步");
        bot
    }
}

/// 配对的假服务端：配上就把设备放行。chat 的测试通常只配一次，不区分发出去的码。
#[derive(Debug)]
pub(super) struct Pairing {
    allowlist: Allowlist,
    /// 接下来这么多个配对请求在回应前睡 [`SLOW_PAIR`]：核心已经成功换了槽位，服务端的结论还没到。
    slow: Mutex<u32>,
}

/// 比一次调度长、比超时短：够测试在服务端回话前把状态读一遍。
const SLOW_PAIR: Duration = Duration::from_secs(3);

impl PairHandler for Pairing {
    fn pair(
        &self,
        device: EndpointId,
        _secret: &PairingSecret,
        _device_name: &str,
    ) -> PairResponse {
        let slow = {
            let mut slow = self.slow.lock().unwrap();
            let hit = *slow > 0;
            *slow = slow.saturating_sub(1);
            hit
        };
        if slow {
            std::thread::sleep(SLOW_PAIR);
        }
        self.allowlist.allow(device);
        PairResponse::Paired
    }
}
impl Pairing {
    pub(super) fn new(allowlist: Allowlist) -> Self {
        Self {
            allowlist,
            slow: Mutex::new(0),
        }
    }
}
