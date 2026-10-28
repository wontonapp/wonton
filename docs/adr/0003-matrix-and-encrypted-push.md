# ADR-0003 消息系统：自托管 Matrix + 加密推送中继

- 状态：已接受（2026-10-28），M0 验证

## 背景

旧方案自研了消息同步（Edge 持久存储加 iroh 同步），三端各写一套同步逻辑，而且没有远程推送：
App 被杀后收不到任何消息。

调研结论：

- 凡是有自有 App 的自托管产品，推送都由厂商运营一个中继，因为 APNs、FCM 的证书只在 App 发行者手里。
  Home Assistant、Nextcloud、Zulip、群晖、ntfy、Rocket.Chat、Matrix/Element 都是这样。
  Nextcloud、Zulip、群晖会先加密通知内容再交给中继；ntfy、Element 只发消息 ID。
- 商业 IM 云（如腾讯云 IM）推送和厂商通道现成，但消息正文在第三方云上，按用量计费
  （专业版国内 1499 元/月、境外 2999 元/月起），境内外要分别建应用，和“数据在家”矛盾。
- 国内 Android 必须接厂商通道；小米、vivo、OPPO（正式权限）要求企业开发者账号和商店上架。

## 决策

- 每台服务器内置 Tuwunel（Rust 写的 Matrix homeserver），默认关闭联邦。它是会话、历史、多端同步、
  家人群聊的唯一事实源。
- 客户端用 matrix-rust-sdk，同步用 Simplified Sliding Sync（Tuwunel 已支持 MSC4186）。
- Agent 和插件都以 Matrix 用户（bot）的身份收发消息。
- 推送链路：Tuwunel 的 HTTP pusher 指向核心推送代理；核心用目标设备的公钥加密通知内容，再交给我们
  运营的推送中继（gorush，支持 APNs、FCM、华为 HMS）；中继只看到密文。iOS 用通知扩展解密显示，
  Android 在推送服务里本地解密。
- 国内 Android：主体和上架完成前，提供可选的“保持连接”模式（前台服务）；之后接厂商通道（M2）。

## 理由

- 用标准协议和现成实现替代自研同步：历史、多端、已读、附件、群聊、通话标准都是现成的。
- 正文只存在家里；云端只多了一个无状态的中继。
- Matrix 生态可复用：Hermes 已有 Matrix 平台，mautrix 系列能桥接其他 IM，Element 可作为备用客户端。

## 代价与风险

- 要学 Matrix 的房间和事件模型；Agent 的流式输出要用消息编辑或旁路实现。
- Tuwunel 是社区项目，不是 Element 官方出品；大量加入联邦大房间时出现过性能问题（我们默认不开联邦）。
- 推送中继的滥用防护要自己做：服务器注册、令牌、限速。
- Element X、FluffyChat 和 Dart 版 Matrix SDK 是 AGPL，不能使用；只用 Apache-2.0 的 matrix-rust-sdk。

## 待验证（M0）

- Tuwunel 经隧道同步的性能，JWT 登录签发，pusher 行为。
- iOS 通知扩展的内存限制下解密的可行性（优先用系统自带的加密能力，不在扩展里跑完整核心）。
- Hermes Matrix 平台在 Tuwunel 上的表现。
- 范围调整（2026-10-28）：iOS 推送需要付费的 Apple 开发者账号（免费的个人开发团队不支持推送能力）。
  M0 先在 Android 上验证加密链路（有谷歌服务走 FCM，没有就用 UnifiedPush 自建 ntfy），iOS 通知扩展的
  解密在有账号后补做。是否把 UnifiedPush 作为正式渠道另行决定。

## 验证结果（M0-2）

验证版本：Tuwunel v1.9.4，matrix-sdk 与 matrix-sdk-ui 0.19.1。可复跑：
`cargo test -p wonton-matrix --test tuwunel -- --include-ignored`（每个测试起一台一次性的 Tuwunel，配置同
`deploy/dev/matrix`；测试标了 `#[ignore]`，所以要加这个参数），本地环境见 `deploy/dev/matrix/README.md`。

- **结论：可行。** Tuwunel 关闭联邦跑在 Docker 里；两个账号用 Simplified Sliding Sync 互发文字和图片，
  2 MiB、8 MiB 的附件原样往返；客户端用内存或 SQLite 存储都通过。
- **同步方式**：客户端用 matrix-sdk-ui 的 `SyncService`，和 Element X 一样：一条 Sliding Sync 连接同步
  房间列表（每个房间只带最新一条消息，邀请也从这里到达），另一条收加密用的 to-device 消息；打开房间时
  按房间订阅拉完整时间线。Tuwunel 在 `/versions` 里声明 `org.matrix.simplified_msc3575`，客户端创建时
  就检查，不支持直接报错。
- **延迟**（debug 构建）：

  | 链路 | 首次同步 | 对方收到文字 p50 / p95 | 对方看到 8 MiB 图片 | 下载 8 MiB |
  | --- | --- | --- | --- | --- |
  | 同机 Docker | 29 ms | 23 / 30 ms | 32 ms | 10 ms |
  | 局域网（Wi-Fi 到有线主机，ping 0.85 ms） | 77 ms | 48 / 100 ms | 274 ms | 755 ms |

  有新事件时长轮询立即返回，消息延迟约等于一次往返加服务器处理。
- **端到端加密**：另外试了加密私聊（matrix-sdk 的 `create_dm`），文字和图片都在对方解密，局域网下首条
  消息 119 ms 到达并解密。M0 的封装建的是不加密房间：推送代理要读到内容才能生成加密通知的预览，房间
  是否加密随推送方案（M0-3）一起定。
- **登录**：设备和 Bot 都用核心签发的 JWT 登录，方案、过期与吊销见 ADR-0009 的验证结果。
- **经隧道**（#21）：可行。隧道服务端的上游指向 Tuwunel，测试经隧道客户端的本地代理访问
  （`WONTON_MATRIX_HOMESERVER` 指向本地代理，步骤见 `deploy/dev/matrix/README.md`）。`messaging` 测试
  （JWT 登录、Sliding Sync 收到邀请、互发文字和图片、下载核对）同机和跨局域网各连续 3 次通过；长轮询、
  2 MiB 附件的上传和下载都正常。其余 5 个登录与吊销测试要用管理账号，仍各自起容器、不经隧道。同样的流程
  另测了一组延迟（每轮 40 条文字，debug 构建）：

  | 链路 | 首次同步 | 对方收到文字 p50 / p95 | 对方看到 8 MiB 图片 | 下载 8 MiB |
  | --- | --- | --- | --- | --- |
  | 同机，直连 Tuwunel（对照，3 轮） | 24–32 ms | 18 / 25–38 ms | 47–60 ms | 8–20 ms |
  | 同机，经隧道（3 轮） | 21–33 ms | 19 / 30–41 ms | 142–154 ms | 112–120 ms |
  | 局域网，经隧道（5 轮） | 43–70 ms | 43–50 / 50–77 ms | 823–851 ms | 760–787 ms |

  局域网一组：测试和隧道客户端在一台 100 Mbit/s 有线的 Linux 主机上，Tuwunel 和隧道服务端在一台 Mac 上，
  iroh 走 IPv6 直连（往返 0.7–12 ms）。隧道几乎不增加消息延迟（同机多约 1 ms），和上表“局域网”一行相当。
  大附件受隧道吞吐限制：同机约 70 MB/s；局域网约 11 MB/s，跑满了客户端的 100 Mbit/s 网卡，和 ADR-0004
  测到的直连吞吐一致（上表 274 ms 那一组的上传方不受这个网卡限制）。现在隧道直接转发到 Tuwunel；产品里
  Matrix 请求经核心网关转给 Tuwunel，网关还没有这条路由。
