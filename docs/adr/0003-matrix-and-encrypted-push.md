# ADR-0003 消息系统：自托管 Matrix + 加密推送中继

- 状态：已接受（2026-10-28），M0 验证

## 背景

旧方案自研了消息同步（Edge 持久存储加 iroh 同步），三端各写一套同步逻辑，而且没有远程推送：
App 被杀后收不到任何消息。

调研结论：

- 凡是有自有 App 的自托管产品，推送都由厂商运营一个中继，因为 APNs、FCM 的证书只在 App 发行者手里。
  Home Assistant、Nextcloud、Zulip、群晖、ntfy、Rocket.Chat、Matrix/Element 都是这样。
  Nextcloud、Zulip、群晖会先加密通知内容再交给中继；ntfy、Element 只发消息 ID。
- 商业 IM 云（如腾讯云 IM）推送和厂商通道现成，但消息正文在第三方云上，按用量计费，
  境内外要分别建应用，和“数据在家”矛盾。
- 国内 Android 必须接厂商通道；小米、vivo、OPPO（正式权限）要求企业开发者账号和商店上架。

## 决策

- 每台服务器内置 Tuwunel（Rust 写的 Matrix homeserver），默认关闭联邦。它是会话、历史、多端同步、
  家人群聊的唯一事实源。
- 客户端用 matrix-rust-sdk，同步用 Simplified Sliding Sync（Tuwunel 已支持 MSC4186）。
- Agent 和插件都以 Matrix 用户（bot）的身份收发消息。
- 推送链路：Tuwunel 的 HTTP pusher 指向核心推送代理；核心用目标设备的公钥加密通知内容，再交给我们
  运营的推送中继（gorush，支持 APNs、FCM、华为 HMS）；中继只看到密文。iOS 用通知扩展解密显示，
  Android 在推送服务里本地解密。
- Android 默认用“保持连接”（前台服务），UnifiedPush 放在设置里作为高级选项（主人 2026-11-04 定）；厂商通道在主体和上架完成后接（M2）。

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
  解密在有账号后补做。

## 设计：加密推送（M0-3）

**链路**

1. 平台给 App 一个推送地址和本设备的推送公钥，即 Web Push 的推送订阅。Android 上由 UnifiedPush 分发器给出
   （M0 用自建的 ntfy），连接库在设备上生成 P-256 密钥对和认证密钥；FCM、APNs 也包装成同样的推送地址（见“渠道”）。
2. App 经隧道把订阅登记给核心（接口见下）。核心按隧道认出的设备记下，回给它一个 pushkey 和推送代理的地址。
3. App 经 matrix-sdk 在 Tuwunel 上注册 HTTP pusher，`pushkey` 和 `data.url` 用核心给的。
4. 有新消息时，Tuwunel 调核心推送代理的 Matrix Push Gateway 接口。核心按 pushkey 找到设备，把房间 ID、事件 ID、
   发送者和房间名按 RFC 8291 加密，按 RFC 8030 发到推送地址。推送服务只看到密文。
5. App 在本机解开推送，经隧道向 Tuwunel 取回这条消息，用本机的房间密钥解密，再弹出通知。取不到正文时照实说明
   原因，比如“新消息（内容暂时取不到：连不上家里的服务器）”。

**推送里不带正文**

- Bot 的主房间是端到端加密的，主人自己建的私聊也加密，推送代理拿到的只有密文，做不了预览。另一种做法是让不加密的
  房间由核心带上预览，但那样要把 Bot 房间改成不加密；核心同时是 Matrix 流量的网关，就能读到全部聊天，所以没选。
- 推送因此只是唤醒信号，和 Element X 的做法一样（它推 `event_id_only`），正文只在手机上解密。代价是每条通知都要
  经隧道取一次消息，耗时和耗电见验证结果。
- 房间、事件 ID 和发送者名也加密，推送服务和中继看不出谁在给谁发消息，只看得到时间和大小。

**加密：Web Push（RFC 8291 的 aes128gcm）**

- 没有自定 HPKE 格式：RFC 8291 就是“用设备公钥加密推送、推送服务读不到”的标准。Android 的 UnifiedPush 连接库
  （Apache-2.0）收到后自动解密；浏览器的 Push API 原生支持，M3 的 Web 端可以直接用；iOS 通知扩展用 CryptoKit 的
  P-256 和 AES-GCM 就能解。
- 核心因此是一个标准的 Web Push 发送方（RFC 8030），每个渠道都只是一个推送地址。加密用 `web-push-native`
  （MIT 或 Apache-2.0）。

**渠道**

- M0：UnifiedPush 加自建 ntfy，不需要账号。推送地址由分发器给出，核心直接发给它，ntfy 就是中继。
- M2：FCM、APNs 经我们运营的中继：中继为 App 的 FCM 或 APNs 令牌开一个推送地址，Web Push 进、gorush 出，用 VAPID
  （RFC 8292）认出发来推送的家里服务器并按它限速。App 换渠道时核心不用改。

**设备的推送密钥**

- 在设备上生成，私钥不出设备（Android 由 UnifiedPush 连接库生成，存在 App 的私有存储里）。核心只拿到公钥和认证
  密钥；认证密钥加上推送地址就能伪造推送，核心把它当秘密存（文件权限 0600，不进日志）。
- 吊销设备时核心删掉它的登记，每次推送前还会再查一次设备登记表，已吊销的设备收不到推送。Tuwunel 下次推给它时，
  推送代理在 `rejected` 里回这个 pushkey，Tuwunel 随即删掉这个 pusher；删除 Matrix 设备时，Tuwunel 也会删掉它的
  pusher（v1.9.4 的 `remove_device`）。

## 接口：推送（M0-3）

**1. 登记：`PUT <核心地址>/_wonton/push/registration`**

- 只认隧道身份，同 Matrix 登录令牌；核心域名上的跨站修改拦截对它生效。
- 请求体是 W3C Push API 的 `PushSubscription` JSON：`{"endpoint": "…", "keys": {"p256dh": "…", "auth": "…"}}`。
  `endpoint` 是 http 或 https 地址；`p256dh` 是 65 字节的未压缩 P-256 点，`auth` 是 16 字节，都是 base64url。
- 响应 200：`{"pushkey": "…", "gateway_url": "http://push.wonton.internal/_matrix/push/v1/notify"}`。同一台设备再次
  登记时换成新的地址和公钥，pushkey 不变。
- 错误同 Matrix 登录令牌的格式：401 `unauthenticated`（不是经隧道的已配对设备）；400 `invalid_input`；
  500 `storage`。
- App 随后注册 pusher：`kind: http`，`app_id` 用 App 的包名，`data: {"url": gateway_url}`。不设 `format`：推送代理要
  用到 Tuwunel 完整格式里的发送者名和房间名。

**2. 取消登记：`DELETE <核心地址>/_wonton/push/registration`**，响应 200 `{"removed": true|false}`。

**3. 推送代理：`POST http://push.wonton.internal/_matrix/push/v1/notify`**

- Matrix Push Gateway API v1，只给 Tuwunel 用。正式安装和开发栈都把 `push.wonton.internal` 设为核心在 `core`
  网络上的别名，`WONTON_PUSH_LISTEN=push.wonton.internal:80` 只绑定这个网络的地址；不发布到宿主，也不监听插件
  网络。专用监听只接受推送 Host，其他 Host 返回 404；未设置这个变量时不启动专用监听。
- Tuwunel 的 `ip_range_denylist` 留空以允许内部服务（ADR-0050），推送直连专用监听，不经联邦 CONNECT 代理；
  安装配置的 `proxy.by_domain.exclude` 保留这个域名。
- Matrix 推送网关没有鉴权，pushkey 是核心生成的 256 位随机串，兼作凭据；推送代理另外只接受 Tuwunel 的地址来的
  请求（按核心配置里的 Matrix 服务器地址现查，容器重建后地址会变），局域网、插件网络和隧道来的都拒绝。
- 只有计数、没有事件的通知（Tuwunel 在未读数变化时发）不转发。
- 发给设备的明文（加密前）是 Matrix 推送通知的子集，加上发推送的服务器：
  `{"server", "room_id", "event_id", "sender", "sender_display_name", "room_name", "prio"}`，只有前三项一定有。
  `server` 是核心隧道的 endpoint ID，手机配了多台服务器时靠它找到该连哪台（ADR-0042）。
- Web Push 请求带 `Content-Encoding: aes128gcm`、`TTL: 86400`，`Urgency` 为 `high`（`prio` 为 `low` 时为
  `normal`）。
- 响应：推送服务收下了（2xx），或者 pushkey 没登记，回 200 `{"rejected": [...]}`。推送地址回 404、410 表示订阅已
  失效，核心删掉登记并 rejected。推送服务暂时不可用（连不上、5xx、429）时回 502，Tuwunel 稍后重试。
- 插件网络里的 Bot（`matrix.wonton.internal`）只能注册指向推送代理的 pusher：`POST /_matrix/client/{版本}/pushers/set`
  里 HTTP pusher 的 `data.url` 必须正好是上面这个地址，别的种类（如邮件）不放行，删除照常放行；不合规的回 403
  `M_FORBIDDEN`（`X-Wonton-Error: permission_denied`）。Tuwunel 能出网而插件默认不能，指向外网的 pusher 会让 Tuwunel
  把 Bot 的通知推到外面，不加密房间里还带着正文；`ip_range_denylist` 只挡私网地址，挡不住这个。转给 Tuwunel 的是检查
  过的那一份（重新序列化），原文里有重复的键也不会让两边看到不同的地址。
- 主人经隧道的设备（`matrix.<base>`）不受这个限制：主人的设备信得过，Element X 这类第三方客户端也要注册自己的
  推送网关（ADR-0015：换客户端不用改服务器）。
- 同样会让 Tuwunel 替客户端去外网取东西的还有 URL 预览（`preview_url`）。Tuwunel v1.9.4 默认不开：`url_preview_*`
  的白名单都是空的，请求一律回 403，取之前就拒绝，我们的配置没有打开它。联邦也关着（见“决策”）。

**4. App 一侧**（`wonton-protocol` 的 `push` 模块）：壳把推送订阅交给核心（`Action::SetPushEndpoint`），平台收不了
推送时如实告诉核心（`Action::PushUnavailable`），进展看 `ChatViewModel::push`。收到推送、解开后交给
`ClientCore::resolve_push`，核心取回消息、解密，给出要显示的通知。通知里核心只给类别和原文（消息摘要、邀请、
取不到正文的原因），占位、“有人邀请你”这些话由壳按 App 的语言写（ADR-0043）。

**Android**（`apps/compose/androidApp` 的 `push` 包，UnifiedPush 连接库 3.3）

- 启动时先申请通知权限，再向分发器登记；没有分发器、没给通知权限、分发器拒绝或注销了本 App，都用
  `PushUnavailable` 照实告诉核心。
- 连接库解不开推送时，会把原文当明文交过来（`PushMessage.decrypted` 为假），App 丢掉这种推送：只有家里的服务器知道
  本机的公钥和认证密钥，解不开就不是它发的。
- 收到推送后自己拿一个唤醒锁，等核心取回消息再弹通知；同一个房间的通知合并成一条。锁屏隐藏敏感内容时，显示不带
  发送者和正文的版本。
- 按类别分渠道：聊天消息、等你确认和插件通知（ADR-0047）各一个系统通知渠道，主人从设置的“通知”页进入对应渠道分别开关；两者保持高重要性，不降低原来消息通知的级别。

**iOS（Apple 开发者账号到位后）**

- App 用 CryptoKit 生成 P-256 密钥对和认证密钥，放进和通知扩展共享的钥匙串；推送地址由中继为 APNs 令牌开。
- 通知扩展也要经隧道取回消息、解密 Megolm。扩展的内存上限很低，在里面跑 iroh 和 matrix-sdk 是那一批的主要风险。

## 设计：保持连接（Android）

Android 收新消息有两种方式，存在本机、每台设备各选各的（`PushMethod`，默认保持连接）。iOS 只有系统推送，壳不显示这个选项。

- **保持连接**：已配对、选了保持连接时（`HomeViewModel::keep_connection`），壳开一个前台服务，进程和隧道在后台不被挂起，
  Matrix 同步照常。App 在后台时，核心按 Matrix 推送规则挑出同步到的、该通知的消息，用和推送同样的方式解出通知
  （`ChatViewModel::background_notifications`），壳照着弹系统通知；解不开时先弹占位，房间密钥晚到就回填——核心把
  解出的正文再交一次，壳用同一个通知标识替换占位（UnifiedPush 的推送路径没有这个机会，扩展或进程返回后就改不了
  已经发出的通知）。回到前台时清空。家里的推送登记和 pusher 删掉，同一条消息不会既推送又本地通知。
- **UnifiedPush**：照上面的“加密推送”，App 被杀也能收到，但要装分发器。

推送规则由客户端评估，结果和 Tuwunel 给 pusher 的判断一致：两边读的是同一份账号的推送规则。

前台服务的类型用 `specialUse`（Android 14 起前台服务必须声明类型，Android 15 起类型决定能跑多久）：`dataSync`
在 Android 15 上每天累计只能跑 6 小时，过了就停，撑不住“一直连着”；`connectedDevice` 要求有配套外设，隧道对端
是家里自己的服务器，不算。`specialUse` 没有系统加的时限，代价是要向 Google Play 声明用途：清单里的服务标
`foregroundServiceType="specialUse"`，用 `PROPERTY_SPECIAL_USE_FGS_SUBTYPE` 写一句话说明（本 App 写的是
“为后台保持到用户自建家庭服务器的隧道连接，端到端加密消息持续到达”），上架时在 Play Console 的声明里对每个
specialUse 服务给出同样的说明和一段演示视频。国内应用市场没有这套类型制度，按各市场的后台保活规范走。

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
  消息 119 ms 到达并解密。产品里的私聊都加密，推送不读正文，见“设计：加密推送”。
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
  测到的直连吞吐一致（上表 274 ms 那一组的上传方不受这个网卡限制）。这组测试里隧道直接转发到 Tuwunel；
  产品里 App 经核心网关的 `matrix.<基础域名>` 访问 Tuwunel（ADR-0009）。

## 验证结果（App 内聊天的客户端能力，2026-10-28）

`crates/wonton-matrix` 补齐了 App 聊天要用的能力，同样对一次性的 Tuwunel 验证（`tests/tuwunel` 的 `chat`、
`encryption` 两组，连续多遍全过）。

- **登录状态**：登录后取出登录状态（用户、设备、访问令牌和刷新令牌）存在本机，重启时恢复会话，不经服务器
  登录，还是同一台设备。设备密钥、交叉签名的私钥和房间密钥都在 SQLite 存储里，用同一个目录恢复后照样可用。
- **房间列表**：用 matrix-sdk-ui 的 `RoomListService`（和 Element X 一样），排序也用它的。每个房间给出名字、
  是否私聊、是否加密、成员状态、最新一条的预览（`RoomExt::latest_event`）和未读数（本机按已读回执算）。
- **时间线**：条目和 SDK 的时间线一一对应，结构增量（`VectorDiff`）按顺序转出；成员资料变化另发同下标的 `Set`，
  不重置列表。按顺序应用增量重建的列表与重新取的一致。能向前翻到房间开头。房间状态事件只显示房间改名和成员离开、被封禁（一行说明，用显示名）；建房、
  加密、权限、谁加入、改昵称不显示（时间线的事件过滤），新房间不会先来一串系统事件。标为已读后发出已读回执，本方
  的未读清零，对方的消息上看到“已读”；已读位置（`m.fully_read`）同时移到最新一条，房间开着时“以下是新消息”不会
  停在中间（只发已读回执时，它会一直停在打开前读到的地方）。
- **图片缩略图**：发图时可以带发送端生成的缩略图，和原图一起上传（加密房间里各自加密）；收到的图片给出缩略图
  的句柄，测试里解密后和发送的一致。
- **打开房间不等网络**：只读本机已有的成员事件（同步时随消息发送者一起来），房间有更新时刷新显示名；全部成员
  在打开后于后台向服务器取（`Timeline::fetch_members`）。取成员完成和房间更新都触发本地成员表刷新；订阅保留带发送者
  ID 的原始条目，SDK profile 缺名字时按成员表重新求值，只给显示发生变化的条目补发 `Set`，不靠定时轮询。matrix-sdk-ui
  0.19.1 的成员查询与 `are_members_synced` 读取之间可能插入成员落库，产生不会再被缺失资料扫描更新的 `Ready(None)`；
  因此不能只等 SDK 自己补名字。新登录的设备上，同步只带最新一条的发送者，更早的发送者要靠后台取成员才有显示名。SDK 的
  `members` 在成员没同步全时会去服务器拉（超时 60 秒、重试 3 次），而 Sliding Sync 里房间只要出现一次
  `limited` 就被标成成员未同步，几乎每次打开都要多一次往返；隧道断开时请求挂住，打开房间要等很久，本机缓存的
  历史也出不来。测试里把服务器冻住（请求挂住、没有回应），从没打开过的房间仍在 1 秒内打开并给出缓存的消息；
  改之前 5 秒都打不开。
- **打开房间时的缺口**：打开前只经房间列表同步过最新一条（房间列表每个房间只带一条）时，更早的消息隔着一个
  缺口，Tuwunel 不一定随房间订阅补发，要向前翻页，界面滚到顶时自动翻。测试里不翻页时，并发跑 6 遍左右会有
  1 遍等不到更早的消息。
- **房间列表的第一项**是算出来的第一份列表，不是占位的空表，界面分得清“还在加载”和“真的没有房间”。
- **端到端加密**：
  - Tuwunel 允许首次上传交叉签名密钥时不再认证（MSC3967 的做法），所以 JWT 登录后客户端能自动建好交叉签名
    身份，本设备随之被验证。
  - 设置恢复（密钥备份加秘密存储）得到恢复密钥；同一账号的另一台设备一开始是“未验证、缺秘密”，输入恢复
    密钥后被验证、恢复完整，重启后保持不变。恢复失败按原因分：恢复密钥格式不对或校验不过是“输入非法”，网络或
    服务器出错照常归类（隧道断开时不会说成密钥不对），账号没有秘密存储是“尚未就绪”。
  - 账号已经设置过恢复时拒绝再设置：再设置一次会换掉秘密存储的密钥，手里的旧恢复密钥就作废了（测试里实际
    踩到过）。
  - 未验证的设备也拒绝设置：它没有交叉签名的私钥，秘密存储里就缺这些私钥，别的设备输对恢复密钥也一直是“缺
    秘密”（模拟器上实际踩到过）。账号里已有的设备不用重新登录，一旦别的设备设置了恢复，就从“未设置”变成“缺秘密”。
  - 恢复密钥丢了，或者备份残缺（输对恢复密钥仍是“缺秘密”），在任何一台设备上都能重置：删掉备份和秘密存储，换新的
    交叉签名身份，发起的设备成为新身份下已验证的设备，再开启备份、给出新的恢复密钥。备份里的旧消息取不回，其他设备输入
    新密钥后恢复完整。Tuwunel 换身份只要求登录用的 JWT 这一个认证阶段，用核心发的一次性登录令牌回答，主人不用输密码。
    本来就有全部私钥的设备输残缺备份的密钥也显示完整，只有没有私钥的设备才看得出残缺。
  - 加密私聊里文字和图片都在对方解密，附件在本机解密；对方重启后历史消息仍能解密。
  - 会话配置的 `Debug` 不输出本地代理的会话凭据和存储口令。
- **发现**：
  - 核心签发 JWT 时自动建的账号没有显示名（资料是空的），别人那里显示用户 ID。Bot 这类账号要自己设置显示名，
    主人的账号由客户端或核心设置。
  - Tuwunel 的 Sliding Sync 不给房间的 heroes 带显示名，SDK 只能拿用户 ID 当私聊的名字；客户端改用本机成员
    事件里对方的显示名。
  - matrix-sdk 0.19 接受私聊邀请时，在加入请求返回后才检查房间是否还是“受邀”，再把房间记进 `m.direct`；同步
    抢先处理了加入时就漏记，这个私聊从此显示成群聊（并发跑测试时多数会漏）。它记的时候拿本机存的 `m.direct`
    改完整份写回，本机这份要等同步回来才更新，一次同步之内先后接受两个私聊邀请，后一个会把前一个冲掉。客户端
    不用 SDK 的加入，加入前读邀请里的私聊标记和邀请人，加入后用 `mark_as_dm`（先从服务器取最新的再改）记私聊。
  - Tuwunel v1.9.4 先让新事件对同步可见，再更新房间的当前状态。这中间的一次 Sliding Sync，时间线里有本人的加入
    事件，`required_state` 里本人还是邀请；SDK 只按 `required_state` 定状态，房间一直显示“受邀”，直到房间里来新
    事件（CI 上偶发，抓同步应答确认）。客户端加入后等同步标成已加入，3 秒还没有就单独订阅这个房间，服务器按当前
    状态重发。上游问题 [tuwunel#685](https://github.com/matrix-construct/tuwunel/issues/685)，修复
    [tuwunel#686](https://github.com/matrix-construct/tuwunel/pull/686)；升到 v2.0.0 后去掉订阅这一步，main 的 Docker 测试
    `chat::stale_join::a_join_the_server_reports_as_invited_still_ends_joined` 又失败（2026-11-12），所以保留。
  - Tuwunel 按（设备，Sliding Sync 连接号）一条一条地处理同步。重启后沿用原来的连接号时，新连接的第一个同步请求要排在
    死掉那条连接还没结束的 30 秒长轮询后面，起步干等半分钟（CI 上并发跑测试时超过 60 秒，`chat::card_plugin_refresh` 因此红过）。
    客户端不再一直用同一个号，改在两个固定号之间轮换（`room-list-a` / `room-list-b`，上次用哪个记在存储里），
    也不读旧号下存的位置（轮换后对不上），同步从 `pos=None` 起。代价就是重启
    等于冷启动：房间列表和打开的房间先给最新一段，更早的历史要往回翻才补上（壳滚到顶时就翻）。同实例内的重启不换号，
    不受影响。实测同一条用例改前每次起步停 17–30 秒、改后 5 秒内起步，16 核压满时也一样。
  - SDK 处理 `m.direct` 时不算房间的显著变化，房间列表不会因此更新；客户端另外观察 `m.direct`。

## 验证结果（M0-3，自动化测试）

验证版本：Tuwunel v1.9.4，matrix-sdk 与 matrix-sdk-ui 0.19.1。可复跑（后两条要 Docker，CI 的“Rust Docker 测试”作业会跑）：

- `cargo test -p wonton-core push`
- `cargo test -p wonton-matrix --test tuwunel -- push egress --include-ignored`
- `cargo test -p wonton-client --test it chat::push -- --include-ignored`


- **推送代理**：推送服务的替身是真实的 HTTP 服务，用 RFC 8291 附录 A 的设备密钥解开它收到的推送，里面只有房间、
  事件 ID、发送者和房间名，没有正文；推送服务收到的字节里找不到这些明文。pushkey 不认识、设备已吊销、推送服务回
  410 时回 `rejected`；推送服务回 503、429 或连不上时回 502；回 413 时不让重试，登记留着。
- **Tuwunel 的 pusher**（真实的 Tuwunel）：
  - 默认的 `ip_range_denylist` 下，直接写私网地址的 pusher 注册时就被拒（400）。`push.wonton.internal` 经出站代理
    到得了推送网关，`Host` 原样保留。
  - 加密房间的新消息按完整格式推：`type` 是 `m.room.encrypted`，带发送者和显示名，里面找不到明文。ruma 序列化时
    省略默认值，所以高优先级的通知没有 `prio` 字段。
  - 推送网关回 `rejected` 后，Tuwunel 删掉这个 pusher。
  - URL 预览：Bot 的令牌请求 `preview_url`（v1 和老的 v3）都得到 403（`egress` 一组）。
- **手机取消息**：App 被杀后，用同一个数据目录恢复会话，不开同步就取消息。Bot 回复时新建了 Megolm 会话，房间密钥经
  to-device 发出时手机不在线；matrix-sdk-ui 的 `NotificationClient` 跑加密同步补上密钥，解出 Bot 的回复。客户端核心
  一创建就交给它推送也行，核心先等会话从本机恢复。
  - 发现：`NotificationClient` 用只在内存里的一份状态，刚加入的私聊它常常不知道是私聊（`m.direct` 还没同步回
    本机），房间名会变成对方的名字；客户端把两个人的房间按一对一显示，同推送规则（`room_member_count == 2`）。
  - 推送里的事件服务器上没有（比如伪造的推送）时，通知里照实写取不到，不显示成正常的消息。

## 未验证（M0-3）

主人决定（2026-10-29），推送的端到端验证等 Apple 开发者账号到位后和 iOS 一起做，Android 用主人带谷歌服务的真机。
下面这些都还没验证：

- Android 上 App 被杀、锁屏时弹出通知的整条链路：分发器把推送交给 App，App 在系统给的后台时间里连上隧道、取回
  消息。唤醒耗时、隧道建连时间、耗电也都没测，它们是“推送里不带正文”的代价。Android 14 起缓存进程会被冻结，处理
  要在分发器给的前台时间（UnifiedPush 规定 5 秒）加上冻结前的宽限里做完；来不及的话，改用前台服务或 WorkManager
  的加急任务。
- 真实的 ntfy：核心发、ntfy 存、ntfy App 转、连接库解开，以及 ntfy 的日志和存储里没有明文。CI 的 Docker 主机拉
  不到 Docker Hub 的镜像，所以没有做成自动化测试。
- FCM（要主人的 Firebase 项目）、APNs 和 iOS 通知扩展（要 Apple 开发者账号）：中继、VAPID、扩展里经隧道取消息。
- 推送中继的滥用防护（服务器注册、令牌、限速）。
