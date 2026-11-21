//! 扫码配对和添加设备的行为测试：只经导出给原生壳的接口驱动。服务端是只走回环地址的隧道入口，配对的处理方按发出去的
//! 码决定登记还是拒绝，登记就是把设备加进隧道的白名单；经隧道来的 HTTP 请求由一个模拟的核心回答。

mod support;

mod cancelling;
mod code_text;
mod connecting;
mod deadlines;
mod first_pairing;
mod invalid_details;
mod invites;
mod joining;
mod language;
mod rejections;
mod reset;
mod route_refresh;
mod slot_switch;
mod stuck_bind;
