//! 隧道的行为测试：服务端、客户端、本地代理、上游 HTTP 服务和测试中继都跑在本机，
//! 不做地址发现，不依赖外网。

mod embedded;
mod federation;
mod forwarding;
mod lifecycle;
mod local_proxy;
mod pacing;
mod pairing;
mod relay;
mod resume;
mod stall;
mod support;
