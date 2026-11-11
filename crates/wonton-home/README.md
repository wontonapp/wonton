# wonton-home

家名注册和 frps server plugin。运行方式、测试 CA、DNS、Compose 与家端接线见
[开发栈](../../deploy/dev/entry-cloud/README.md)。

PowerDNS 管权威记录，acme-dns 管受限 TXT 更新，frps 管 SNI 密文转发；没有一个现成组件把
endpoint Ed25519 身份、单次邀请、永久家名、CAA 冷却和 frp 名字归属连起来，因此只自研这层管理服务，
不实现 DNS、ACME 或隧道协议。云端不会收到家里的证书私钥。

所有管理请求（含注册）签原始 body；防重放 nonce 随状态一起持久化。重复注册必须同一 endpoint 和
原邀请；返回同一个家、轮换 frp token（只保存其 SHA-256），所以丢失第一次 HTTP 响应可以恢复。
删除保留 tombstone，停用与删除先持久化拒绝准入，再删除 DNS；恢复必须先恢复 DNS。
CAA 第一次立即写权威 DNS，换账号等待 24 小时，同值不延长等待，提交旧值撤销待生效变更。

状态文件通过同卷 rename + fsync 更新；daemon 持有独占文件锁。管理 CLI 只通过 0600 Unix socket
向这个 daemon 提交命令，不从另一个进程直接改 JSON。备份整个状态卷（含 acme-dns 更新凭据），
不得同时启动两个写同一卷的服务。命令 `invite <seconds>` 只向 stdout 输出一次性邀请，
`suspend <home> <reason>` 与 `resume <home>` 完成 DNS 操作后才成功返回。

frps 的 Login/NewProxy/NewUserConn/Ping 绑定 `user` 与 `metas.token`；CloseProxy 回收配额槽。
仅接受一个具体 apex 或非 `lan` 单标签 HTTPS 域名。每入口服务端限速 5 MB/s，每家五入口，
并发 100、UTC 自然月总流量 100 GB。每 30 秒读取 frps 本地 Prometheus 指标，指标超过 65 秒不更新时
新代理/连接和 Ping 拒绝；Login 仍验身份以允许控制面恢复。流量和计数基线持久化，frps counter
重置后从新值累计。并发计数最多滞后一轮采集。停用的新连接立即拒绝，既有连接由 frpc 下一次
30 秒心跳中断，不合作的 frpc 由 frps 90 秒 heartbeatTimeout 断开，DNS 缓存不能绕过 hook。

后台每 30 秒按已持久化的期望状态调和 DNS（包括到期的 CAA），不是对失败的用户请求偷偷重试。
API 的失败不代表 DNS 完全没有变化；客户端用相同 endpoint 发新签名请求恢复，不能换身份重新注册。

## 依赖许可证

链接进 Rust 二进制：anyhow/axum/getrandom/reqwest/serde/serde_json/sha2/tokio/tracing/
tracing-subscriber/url 为 MIT 或 Apache-2.0，iroh 为 MIT 或 Apache-2.0，subtle 2.6.1 为 BSD-3-Clause，
fs2 0.4.3 为 MIT 或 Apache-2.0；测试 tempfile 为 MIT 或 Apache-2.0。使用 workspace 版本，
不链接 GPL/AGPL。独立容器的许可证见开发栈 README。

行为测试用本地 HTTP 测试替身观察真实管理 API 的状态转移、DNS 副作用、签名/重放、
并发单次邀请、CAA 和 hook 配额；不把替身当作真实 ACME 验证。集成验证必须使用开发栈的
PowerDNS/acme-dns/Pebble/frps。Pebble DNS-01 成功不证明生产 CA 的 CAA accounturi 策略有效。
