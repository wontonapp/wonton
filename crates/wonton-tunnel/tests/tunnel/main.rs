//! 隧道的行为测试：服务端、客户端、本地代理和上游 HTTP 服务都跑在本机，endpoint 只走回环地址直连，
//! 不连中继、不做地址发现，所以不依赖外网。

mod embedded;
mod forwarding;
mod lifecycle;
mod local_proxy;
mod pacing;
mod pairing;
mod resume;
mod stall;
mod support;
