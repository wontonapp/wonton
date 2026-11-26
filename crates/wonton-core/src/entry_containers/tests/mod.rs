//! 入口容器和连接器的期望状态：按设置建、删，安全选项逐项锁住。

/// App 里配的域名（没有设置文件）决定入口：没证书先不建、读不了不删、与设置文件冲突时按设置。
mod app_domain_tests;
/// 证书和 crt-list：换域名、换父域、同名续期都等新证书发布，才换名字并 reload。
mod certificates_tests;
/// 第三方客户端模型：public/lan crt-list 的内容、公开访问开关、重载失败时回滚。
mod clients_model_tests;
/// 创建请求：容器、网络、别名、安全选项、两种连接器、发布端口。
mod create_tests;
/// 错误按原因归类。
mod error_categories_tests;
/// 失败路径：重载失败跨轮重试、写下去后的容器动作失败、拉不到镜像不动在跑的容器。
mod failure_handling_tests;
/// 要不要入口、要不要删：无设置、撤设置、缺文件重建、幂等，核心不在容器里。
mod lifecycle_tests;
/// 已有同名容器：陌生人、上一版 compose、换端口重建、重建失败、假 Docker 的地址冲突。
mod recreate_tests;
/// 向入口主进程发 reload：失败报启动日志，忙、空回、半截、连上不回都重试。
mod reload_tests;
/// 共用的假 Docker 装置、设置和文件帮手、假 HAProxy 管理端口、联邦注册设置。
mod test_support;
