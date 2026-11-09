//! 隧道的行为测试：服务端、客户端、本地代理、上游 HTTP 服务和测试中继都跑在本机，
//! 不做地址发现，不依赖外网。

mod embedded;
mod federation;
mod forwarding;
mod lifecycle;
mod local_proxy;
mod pacing;
mod pairing;
mod peer_pair;
mod plugin_ports;
mod port_udp;
mod port_udp_fragments;
mod port_udp_lifecycle;
mod probe;
mod relay;
mod relay_table;
mod reset_resume;
mod resume;
mod revocation;
mod stale_addrs;
mod stall;
mod support;
mod suspend_resume;
