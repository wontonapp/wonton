# ADR-0118 聊天分三层（本地、推送、连接）；房间不再端到端加密，推送带正文

- 状态：已接受（2026-11-25），主人拍板（原话要点见“已定”）。修订 [ADR-0003](0003-matrix-and-encrypted-push.md) 的“推送里不带正文”和房间加密的前提，
  也修订 [ADR-0005](0005-native-clients-thick-core.md)（备份与恢复密钥）、[ADR-0008](0008-model-gateway-magpie.md)（Hermes 的 `MATRIX_E2EE_MODE`）、
  [ADR-0009](0009-identity-pairing-sso.md)（Bot 主房间的端到端验证）、[ADR-0016](0016-mcp-bus-and-confirmations.md)（Wonton 私聊、卡片显示在会话里）、[ADR-0038](0038-multi-step-cards.md)（卡片的明文降级）、
  [ADR-0042](0042-multiple-servers.md)（推送内容）、[ADR-0045](0045-family-members.md)（邀请页的说法）、[ADR-0046](0046-device-verification.md)（删除）、
  [ADR-0049](0049-cloud-hosting.md)（托管的数据与隐私）、[ADR-0050](0050-cross-server-chat.md)（跨服务器的机密性）、[ADR-0076](0076-device-verification-fallback.md)（删除）、
  [ADR-0087](0087-pairing-key-handoff.md)（配对不再交接密钥）、
  [ADR-0101](0101-notifications.md)（通知里的正文）、[ADR-0096](0096-calls-matrixrtc-widget.md)（不加密房间里的通话）
- 关联：本地层 `feat/chat-local-first`（卡 im-local-first），连接层 `feat/tunnel-fast-reconnect`（卡 im-fast-connect）

用户看到的变化：收到的通知里直接有正文，不用点开也不用等连接；打开 App 先看到本机已有的聊天，不再空等隧道；点开通知到能说话快得多。
换来的是聊天内容在家里那台机器上不再是密文——能登上它拿到 root 的人、我们代管的云主机宿主、跨服务器聊天时对方那台机器，都读得到。
云端（我们的中转、推送中继、苹果、谷歌）仍然只见密文；手机厂商自己的系统通道只看到通用的“新消息”，小米、OPPO、vivo 那几家的通知上暂时不显示正文，点开才取回（见“一条通道一条通道地确认”）。

## 背景

主人 2026-11-25 的判断：家里人和主人之间、主人和自己的 NAS 之间不必互相防；Bot 的对话历史本来就以明文躺在 NAS 上，核心也是 Matrix 的网关，
端到端加密这一层对云吞是冗余的，却让每条通知都要先经隧道取一次才看得到内容。

要的效果是三层各管一件事：本地层先给本机存的历史，推送层把该看的正文送到通知，连接层负责同步和大文件、建连要快。

这和 ADR-0003 当时的取舍相反。那时的理由是“核心同时是 Matrix 网关，改成不加密以后核心也读得到，所以不加密没意义”。
现在接受这个后果：既然只防云端，房间加密去掉，推送用设备公钥加密（RFC 8291）就够。

## 决策

### 1. 三层各管一件事

- **本地层**：作用域建好、本机有这台服务器可恢复的会话时，立刻从本机 SQLite 恢复会话、房间列表和历史，不等隧道；连不上时如实显示“连不上家里的服务器”，同步自己重试。
  首次登录（本机没有会话）仍然要等连接。取舍和契约见本地层的卡（分支 `feat/chat-local-first`），本 ADR 只固定“先本地、后网络”。
- **推送层**：不加密房间的通知带正文，核心按 Web Push（RFC 8030、8291）用设备公钥加密后交给推送服务。手机在本机解开就直接显示。
- **连接层**：iroh 隧道负责同步、取消息和文件。连接信息（中继表、核心报告的直连地址、本机连通过的历史地址）都在本机文件里，建连不等网络发现；
  推送进程和 App 是两个进程，共写同一份连接信息，推送进程学到的地址 App 下次直接用（见连接层的卡，分支 `feat/tunnel-fast-reconnect`）。

### 2. 推送层：通知带正文，不引入新的推送 SDK

链路不变：核心推送代理 → 我们的推送中继（gorush 加鉴权）→ APNs / FCM / HMS；Android 另有保持连接前台服务和 UnifiedPush。
极光这类聚合推送 SDK 不进（多一层第三方，还要多一份凭据和隐私声明）。各自怎么做到、做不到时退回什么见下一节。

**Tuwunel 这一侧**。pusher 注册照旧不带 `format`（ADR-0003 的“接口：推送”），Tuwunel 就按完整格式推，`notification.content` 是事件内容。
不加密房间里，正文、消息类型都在 `content` 里；加密房间只有 `m.room.encrypted` 的密文（实测见“验证结果”）。核心因此不用改 Tuwunel，只解析现成的字段。

**核心这一侧**：

- 解析 `notification.content` 的 `body`、`msgtype`，放进发给设备的明文，字段跟着 `server`、`room_id`、`event_id`、`sender`、`sender_display_name`、`room_name`、`prio` 一起，
  用设备公钥加密（`DevicePayload` 加可选 `body`、`msgtype`）。
- 通知的响铃规则仍由 Tuwunel 的推送规则定；插件网络里 Bot 的 pusher 仍然只能指向核心的推送代理（ADR-0003），不改。
- **大小**：加密前的明文总预算 3 KiB 封顶，按渠道再收紧：密文直接当请求体的渠道（UnifiedPush）3 KiB（规范：密文 1–4096 字节，明文至多 3993）；
  要套进 JSON、密文再 base64 的渠道（APNs、FCM、HMS）2 KiB（base64 涨三分之一，外面的载荷上限是 4 KB）。
  正文先去换行（合并成空格）再截到 120 个字符，按 UTF-8 字符边界切，不加省略号；附件消息的 `body` 是文件名，`msgtype` 说明类型，不带 URL 和缩略图，
  点开房间后再从服务器取全。这样还放不下时按 房间名 → 发送者显示名 → 正文 的顺序丢字段，`server`、`room_id`、`event_id`、`sender` 一定留着。
- 没有 `content` 的通知（计数的、加密房间、别的服务器上老版本核心）照旧只发唤醒信号，手机取回后再补正文；取不到时按原因如实写，沿用 ADR-0003 的说法。
  邀请的 `content` 只有 `membership`、`is_direct`，没有 `body`，按“有人邀请你”处理；`content.counts.unread` 这些附带字段先不读，未读数仍由核心自己算（ADR-0101）。

**手机这一侧**：

- **Android**：推送服务里解开就直接显示正文，不再为了正文经隧道取一次。
- **iOS**：通知服务扩展用 CryptoKit 解 Web Push 那层，把正文写进通知内容。
  正文已经在推送里，扩展不用起核心、不用开隧道、不用解 Megolm，ADR-0101 第 2 节的内存、存储锁和隧道所有权约束因此放宽成“尽力而为”，不再是准入条件。
  通知上的回复和按钮仍然落到核心（ADR-0101 第 1 节），这段要进程和隧道，照旧。
- 锁屏上给不给看内容按 ADR-0101 第 5 节的开关，关掉时只写“新消息”。

### 3. 一条通道一条通道地确认

决定是“能本机解就显示正文”，不是“猜”。每个渠道都对着官方文档核过（2026-11-25 查），做不到的本机解密的，退回系统能直接显示的通用文字，
不把密文当正文，也不假装已经解出。

| 渠道 | 带正文、不点开就显示 | 依据、限制和退回 |
| --- | --- | --- |
| iOS APNs（经我们的中继） | 能。`apns-push-type: alert` 加 `aps.mutable-content: 1`，`aps.alert` 里先放“新消息”，密文放自定义键；通知服务扩展解完改写正文 | [《Modifying content in newly delivered notifications》](https://developer.apple.com/documentation/usernotifications/modifying-content-in-newly-delivered-notifications)（扩展只在 `mutable-content: 1` 且有 alert 时运行；扩展没跑或超时会显示原始 `aps.alert`，也就是“新消息”）；载荷上限 4 KB（[《Sending notification requests to APNs》](https://developer.apple.com/documentation/usernotifications/sending-notification-requests-to-apns)） |
| FCM | 能。data 消息加 `priority: high`：后台和进程被杀时都由 `onMessageReceived` 收到，应用自己弹通知；高优先级会把设备从 Doze 唤醒 | [《Receive messages in Android apps》](https://firebase.google.com/docs/cloud-messaging/android/receive-messages)（后台一栏：Data → `onMessageReceived`）、[《Set and manage Android message priority》](https://firebase.google.com/docs/cloud-messaging/android-message-priority)、[Firebase 官方博客 2025-04-17](https://firebase.blog/posts/2025/04/fcm-on-android)；载荷 4096 字节含 base64。被用户“强行停止”的 App 在再次打开前收不到；国产 ROM 的后台限制另一层（见下） |
| UnifiedPush（自建 ntfy 等分发器） | 能，最干净。分发器把原始 POST 字节交给 App 里的连接库，连接库在本机解 RFC 8291，分发器只见密文 | [UnifiedPush 规范 AND_3.0.0](https://github.com/UnifiedPush/specifications/blob/main/specifications/android.md)：推送消息必须是推送服务收到的原始 POST 数据、必须按 RFC 8291 加密、1–4096 字节（明文至多 3993） |
| 保持连接（Android 默认） | 能，不经推送服务：核心自己出本地通知，正文完整 | ADR-0003 的“设计：保持连接” |
| 华为 HMS | 有条件。透传消息（`data`）在前台和后台都进 `onMessageReceived`；但送达受系统与应用是否驻留影响，高优先级透传要申请权限（EMUI 10.0 以上） | [《获取消息数据》](https://developer.huawei.com/consumer/cn/doc/HMSCore-Guides/android-basic-receivemsg-0000001087370610)、[《受限说明》](https://developer.huawei.com/consumer/cn/doc/HMSCore-Guides/restrictions-0000001050040064)（2024-10-21，“透传消息的送达受 Android 系统和应用是否驻留后台影响”“高优先级透传消息需要申请权限”，内容上限 4 KB）。没权限或被限时退回通知栏消息，只写“新消息” |
| 小米 | 不能在手机上解。2022-09-12 起停止提供透传消息，只剩通知栏消息：系统直接弹出，不需要应用启动、不受自启动管理影响，点开前应用完全不可感知 | [《停止提供透传消息下发的公告》](https://dev.mi.com/xiaomihyperos/documentation/detail?pId=1530)、[《推送常见问题汇总》](https://dev.mi.com/distribute/doc/details?pId=1545)（2026-10-19，“在 MIUI 系统上，通知栏类型的消息，是不需要应用启动就能弹出的……app 是完全不可感知的”）。退回：只写“新消息” |
| OPPO | 不能。推送服务只做通知栏消息，标题和正文是系统字段，由系统展示；全部文档里没有透传 | [推送服务介绍](https://open.oppomobile.com/new/introduction?page_name=push)（“推送服务目前支持通知栏信息”）、[《通知栏消息》](https://open.oppomobile.com/documentation/page/info?id=11236)（title 必填，content 限 50 或 128 字）。退回：只写“新消息” |
| vivo | 不能。vpush 通知由系统展示，“不需要应用驻留后台”，没有透传 | [《消息分类说明》](https://dev.vivo.com.cn/documentCenter/doc/359)（2026-09-21）。退回：只写“新消息” |
| Web（M3） | 能。浏览器的 Push API 原生解 RFC 8291，页面接到明文 | ADR-0003 选 RFC 8291 的理由不变 |

退回的写法统一：标题是 App 名，正文是“新消息”，点开进房间按 `room_id`、`event_id` 定位并取回。**不把正文以明文交给系统展示的渠道**——
想显示就得把正文交给小米、OPPO、vivo 和华为的系统通道，那“云端看不到内容”就破了；先只给通用文字，以后要按渠道再定。

国产 ROM 的另一层限制（小米“应用被杀死又没开自启动就收不到”、HMS 的驻留条件）不靠加开关绕过：`PushUnavailable` 之外的失败都照实分类，
保持连接始终是能自己保底的选项。

### 4. 房间不再端到端加密

新建的房间都不带 `m.room.encryption`：Bot 主房间、私聊、家庭群、好友房间、跨服务器私聊。建房的几处（`create_direct_room`、`invite_as_contact`、家庭群建房）都去掉这段 `initial_state`；
`create_notice_room` 本来就不加密，不动。

逐个处置：

| 项 | 结论 |
| --- | --- |
| [ADR-0046](0046-device-verification.md) 新设备的 SAS 确认、`EncryptionStatus::verification_flow`、两端的确认页 | 删除。没有房间密钥要交接，新设备同步下来就是历史的全部 |
| [ADR-0005](0005-native-clients-thick-core.md) 聊天备份、恢复密钥、秘密存储、备份页、“输入恢复密钥”提醒 | 删除。备份原本只为加密历史而在设备间传递密钥，现在历史在服务器上 |
| [ADR-0087](0087-pairing-key-handoff.md) 配对码之外自动带上备份密钥 | 删除。配对只剩登录 |
| [ADR-0038](0038-multi-step-cards.md) 卡片“题目和答案是明文”的降级 | 不再是降级，全库一致；原文留着当历史，加一句指向本 ADR |
| [ADR-0008](0008-model-gateway-magpie.md)、[ADR-0009](0009-identity-pairing-sso.md) 里 Hermes 的 `MATRIX_E2EE_MODE=required` | 改成 `off`（adapter 接受 `off`、`optional`、`required`，`off` 连加密依赖都不装）。主房间和好友房间不加密，Hermes 不用带 Matrix 密码学，也就没有自动建交叉签名这回事 |
| 交叉签名、设备密钥、room key、`/keys` 相关客户端代码 | 客户端核心保留 matrix-sdk 的加解密能力，收到别的客户端（如 Element 用 https 直连）建的加密房间还能读；不再做任何面向用户的加密流程 |
| `EncryptionStatus` 的模块与界面 | 删掉验证、备份、恢复密钥的部分，只留“有没有解不开的消息”（`Undecryptable`、`key_notice`）的读取；模块名照旧 |
| 通话（[ADR-0096](0096-calls-matrixrtc-widget.md)）的媒体密钥 to-device | 走法不变（MatrixRTC + Element Call，媒体帧加密，SFU 和中继只见密文）；房间不加密后这些密钥不再套 Olm，家里的服务器读得到、也就解得出媒体，和聊天内容同一个边界。实际行为以卡 call-e2e-android 在不加密房间里的实测为准 |

**代价要如实写**：

- 有 NAS 的 root、能读盘、能读备份和快照的人，读得到全部聊天。
- 托管（ADR-0049）的宿主读得到虚拟机的盘和内存；那里原来靠“不防我们”的话术，现在连话术都不用改，事实更彻底。
- 跨服务器聊天（ADR-0050）对方服务器的盘上也是明文。
- 传输层不变：隧道是 iroh 的 QUIC，两端按 endpoint ID 认。推送正文用设备公钥加密：推送服务、中继、苹果、谷歌只见密文。
- 聊天的机密性从此只有两个边界：设备与云端之间（推送加密、iroh），以及“谁拿得到服务器”。家人邀请页（ADR-0045）那一句“聊天端到端加密，服务器上读不到”按事实改。

### 5. 安全边界写成测试约定

“推送服务只见密文”从口头约定变成测试断言：推送服务收到的字节里，房间名、发送者、正文的原文字节都找不到。
现有的核心推送测试已经这样断言房间名和发送者（ADR-0003 的验证结果），扩展到正文，新字段一并加进去；Tuwunel 的集成测试补一条不加密房间的用例：
确认 `content.body` 到了网关，而网关收到的字节里正常找不到解码后的正文。

系统展示的渠道（小米、OPPO、vivo、HMS 的通知栏消息退回）多一条：发出去的 payload 里只允许出现通用文字“新消息”，房间名和正文都不出现。

### 6. 迁移：绿地，不迁

已有加密房间不转换：留着还能在本机读，只是新设备同步下来的旧加密历史没有密钥备份就解不开。开发和测试数据直接重建（AGENTS.md 的“兼容”）：
开发栈重建 Matrix 数据，测试夹具改成不加密，`cargo test` 里跟加密相关的用例删掉或改成“加密房间仍能读”。

## 理由

- 少一层：没有 Megolm、备份、验证、恢复密钥、跨进程密钥交接，客户端和通知扩展都薄了，iOS 通知扩展最大的技术风险（24 MB 内存、两个进程抢存储和隧道）随之消失。
- 通知立刻有内容：不用点开、不用等隧道；Android 的冷启动和推送进程都不必为了一个预览去建连。
- 云端的可见性和以前一样：中转和推送服务仍然只见密文，苹果、谷歌也是；变的只是“自己家里那台机器”和代管宿主。
- 端到端加密在云吞的收益本来就有限：Bot 的历史在 NAS 上是明文，核心经手全部流量，Matrix 数据库和核心在同一台主机上（ADR-0038 当时也是这么说的）。

## 没选的做法

- **房间继续加密，只给推送加明文预览**（ADR-0101 原来没选的“按房间开关预览”）：能看正文，服务器也看不到，但两套并存：加密还要、备份还要、验证还要，通知扩展还是得解 Megolm。
  主人要的就是把这一层拿掉。
- **只让 Bot 房间不加密，私聊和家庭群继续加密**：行为分两套，家人之间的通知还是没有正文，两层模型各留一半。
- **推送明文直接交给推送服务，不加密**：中转、苹果、谷歌读得到，违背“云端看不到内容”。
- **自定推送载荷的加密格式（HPKE 自定）**：RFC 8291 是现成的，UnifiedPush 连接库收到就解，不需要我们写密码学；ADR-0003 选它的理由不变。
- **用核心保管的密钥加密推送**：设备公钥已经够，核心不用留一把推送私钥。
- **把整个 matrix-sdk 的加密模块从客户端剪掉**：能更薄，但 Element 这类第三方客户端仍可能建加密房间；先保留，以后单独定。
- **接极光这类聚合推送 SDK**：一家接完能同时到各家，但多一层第三方（另有一份凭据、SDK 和隐私声明），还要再学它的消息模型；gorush 已经能把 APNs、FCM、HMS 发出去，先不加。
- **系统展示的渠道也把正文以明文交给厂商**（像商业 IM）：通知上就能看正文，但小米、OPPO、vivo、华为都能读到内容，“云端看不到内容”不再成立。先只给通用文字，按渠道开是以后的事。

## 代价与风险

- 上限由最窄的渠道定：UnifiedPush 的密文必须 1–4096 字节，APNs 载荷 4 KB、FCM 4 KB 含 base64。正文只能给 120 个字符，长消息和附件在通知里只剩开头或文件名，进去才看全。
- 系统展示的渠道（小米、OPPO、vivo，以及拿不到高优先级透传权限的华为）通知上只有“新消息”，这几家上“不点开就能看到正文”做不到，点开才取回。
- 截断、丢字段和按渠道收紧预算都是硬约定，任何新字段都要过一遍大小测试；没放进去的字段要用“取不到正文”的既有分类兜底，不能空正文。
- 锁屏可见范围变大：默认显示正文，手机在别人手里就看得到（ADR-0101 的开关可关）。
- 旧加密房间换设备后读不到历史（没有恢复密钥）。
- 家人和跨服务器对方失去“连服务器管理员都看不到”的说法，邀请页和文档都要照实改。
- 核心第一次同时拿正文和推送规则，通知内容和 Tuwunel 的响铃判断可能出现不一致（比如静音房间推了正文）；以 Tuwunel 的规则为准，核心不改它的判断。

## 验证结果（2026-11-25）

- 探针（真实 Tuwunel v2.0.0，`crates/wonton-matrix/tests/tuwunel/push.rs` 的临时用例 `push::zz_probe_unencrypted_push_payload`，不进本 PR；`CARGO_SLOT_LOCAL=1 cargo test -p wonton-matrix --test tuwunel -- push::zz_probe --include-ignored --nocapture`）：
  - 不加密私聊里的文字：网关收到的 `notification` 是 `type: m.room.message`，带 `content.body`（原文“你的快递到了”）、`content.msgtype: m.text`，另有 `room_id`、`event_id`、`sender`、`sender_display_name`、`counts.unread`。**正文随完整格式来，核心不用改 Tuwunel。**私聊没有房间名，也没有 `prio`（高优先级被省略）。
  - 图片：`content` 是 `{"body": "猫.jpg", "msgtype": "m.image", "url": "mxc://…"}`，没有缩略图数据。
  - 600 字的正文原样到达（`content.body` 长度 600），Tuwunel 不截断，大小限制只能核心做。
  - 私聊邀请：`type: m.room.member`，`content` 是 `{"is_direct": true, "membership": "invite"}`，带 `user_is_target: true`，没有 `body`。
  - 同一房间连发时 Tuwunel 会合并还在途的推送（第一次跑 60 秒只等到文字那条，图片那条被并掉）；探针改成每条等到了再发下一条。
  - 加密私聊的通知只有 `type: m.room.encrypted`，正文是密文（现有用例 `push::a_bot_reply_is_pushed_through_the_proxy_and_decrypted_by_the_woken_phone` 一直这么断言）。
- Hermes 的 `MATRIX_E2EE_MODE` 取值：在 `nousresearch/hermes-agent:v0.22.7` 镜像里 `plugins/platforms/matrix/adapter.py:6` 写明 `off|optional|required`，`off` 时连加密依赖都不检查（同文件 :783-789）。
- `python3 scripts/adr-index.py --check`：通过。

## 待验证

- 各渠道的真实上限与截断行为：UnifiedPush（自建 ntfy）、FCM、APNs、HMS 的 4 KB 是不是按字节算、超了是截断还是整条拒收。要真机真通道各发一条超长正文。
- 不加密房间里的通话：媒体帧加密还在不在、媒体密钥的 to-device 是不是明文、SFU 是否仍只看到密文，以卡 call-e2e-android 的实测为准（它要把聊天和通话都在不加密房间里跑通）。
- 华为的高优先级透传权限能不能申请到、申请要多久；拿不到时退回通知栏消息的到达表现。
- 小米、OPPO、vivo 的通知栏消息在 App 被杀、系统被杀后台之后的到达率（官方都说系统通道不受影响，要真机确认）。
- iOS 通知扩展只解 CryptoKit 这一层的实际内存和耗时（应当远低于 ADR-0101 的 24 MB 约束，但要量）。
- 反方向的兼容：用 Element 建的加密房间，通知里只有唤醒信号，客户端的“取不到正文”路径要仍然可用。

## 已定（主人 2026-11-25）

- 不考虑家人之间、主人和 NAS 之间的信任问题：NAS 是用户自己的，Bot 的历史本来就能在 NAS 上读到，端到端加密对云吞冗余。
- 云端看不到内容，手机不用点开通知就能看到正文。
- 分三层：本地先显示本机历史，推送带正文，连接层做同步和大文件、建连要快。
- （补充）推送约 4 KB 上限，正文超长只放开头一段，附件只写类型和文件名，点开后补全。
- （补充）不引入新的推送服务：沿用 核心推送代理 → 我们的推送中继（gorush 加鉴权）→ APNs / FCM / HMS，Android 另有保持连接和 UnifiedPush；极光这类聚合 SDK 不用。
- （补充）渠道逐个核实官方文档，写明查的日期和链接；做不到的写清退回什么，不伪装。
