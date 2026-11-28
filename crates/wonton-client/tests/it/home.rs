//! 和家里服务器连接的行为测试：只经由导出给原生壳的接口（`ClientCore`、`Action`、订阅）驱动。
//! 隧道服务端、插件上游都跑在本机，只走回环地址，不依赖外网。

mod address_refresh;
mod background;
mod connect;
mod identity;
mod migration;
mod notices;
mod pairing;
mod publication;
mod relay_token;
mod reported_relays;
pub(crate) mod support;
mod switching;

mod quality;
mod rebuild;
mod relays;
mod retry;
mod slow_dial;
mod stuck_bind;
pub(crate) use support::{Home, Phone};
