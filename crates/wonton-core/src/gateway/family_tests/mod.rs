//! 家人（ADR-0045）的网关行为测试：真实隧道上配对（`tunnel_tests` 的 `Home` 和 `Phone`），路由分层用
//! 进程内的设备身份（`Harness`，塞 `TunnelPeer` 扩展，和隧道入口给每个请求放身份的做法一致）。

mod devices;
mod identity;
mod membership;
mod pairing;
mod permissions;
mod profile;
mod support;

// 别的网关测试模块按 `family_tests::…` 取这些夹具，拆开后保持原来的路径可见。
pub(super) use support::*;
