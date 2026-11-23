# ADR-0114 插件 Agent 底座：家人账号、群聊说话人、事件、按人通知、卡片模板、深链、结果可见范围、文件交给插件

- 状态：提议（2026-11-21），影音相册线起草，插件底座线复审后合并
- 破坏性改动三处（逐条见“兼容与迁移”）：`lan.client` 口令按人、共享 Bot 单人时 `app.wonton/user` 带人、结果可见性引入 `Room`
- 关联：[ADR-0015](0015-small-components-standard-interfaces.md)（小部件、标准接口）、[ADR-0016](0016-mcp-bus-and-confirmations.md)（MCP 总线与确认）、
  [ADR-0020](0020-file-service.md)（文件服务）、[ADR-0024](0024-confirmation-scope.md)（每次问的三类）、[ADR-0038](0038-multi-step-cards.md)（卡片）、
  [ADR-0041](0041-app-links-and-shortcuts.md)（`wonton://` 链接）、[ADR-0045](0045-family-members.md)（家人）、[ADR-0047](0047-plugin-notices.md)（插件通知）、
  [ADR-0088](0088-family-accounts.md)（角色与按人身份）、[ADR-0097](0097-share-in.md)（分享进来）、[ADR-0104](0104-view-file-media.md)（看文件）、
  [ADR-0108](0108-hermes-default-kernel.md)（Hermes）、[ADR-0110](0110-one-hermes-container-per-person.md)（一人一个 Hermes 容器）
- 边界：[ADR-0113](0113-media-transport.md) 管长任务（备份上传、播放会话、离线下载）；本篇只管 Agent 用它时要的通用接口，不重写它的模型

## 背景

相册、音乐、影视三份方案各自拆出了一批“插件底座要先补的接口”，大半是同一件事（`notes/summary.md` 的“三块共用的底座项”表）。
它们有一个共同的形状：插件替某个家人在另一个应用里办事，而现在的总线只认得住“哪个 Bot”，认不出“替谁”：

- 家人的 Wonton 账号和应用里的账号没有对应关系，插件只能全家用一把 key（相册 F1、音乐 K6、影视卡 3）。
- 共享 Bot 在群里办事时，插件拿不到说话人（相册 F2/卡 3b、音乐 K0b④、影视卡 7）。
- 插件有事要说时，只能发不可回话的通知，发不到指定的人，也叫不醒空闲的 Agent（相册 F4、音乐 K8、影视卡 15/17）。
- 确认卡片只有标题加原始 JSON，结果文件只有 Bot 的归属人能看，通知只能到主人，插件拼不了能点开的深链，插件的文件进不了插件。

Agent 是 Hermes，一个家人一个容器（[ADR-0108](0108-hermes-default-kernel.md)、[ADR-0110](0110-one-hermes-container-per-person.md)）：每人的 Bot 分容器，
同一个人的多个 Bot 同容器、以 profile 分开。轮到谁的 Bot 说话，`X-Wonton-On-Behalf-Of` 就带谁，核心已经拿它算 AI 动手级别
（`plugin-spec.md:2456-2462`、`mcp/audience.rs:105-120`）。本篇把它接到的身份继续往下递。

## 已有的，不重建

下面八项里，有六项已经有了一半，只补缺口，不另起一套：

| 已有 | 在哪 | 本篇只补 |
| --- | --- | --- |
| 按人的网页登录（OIDC `sub` 就是本机用户名，代理登录模板带 `{user}`） | `plugin-spec.md:710-760`、`:861-880` | 账号的建、删和凭据，以及 MCP 侧按人取 |
| 应用按人分数据（`webui.accounts: per_user`） | `plugin-spec.md:672-676`、`protocol/src/manifest/web.rs:83` | 把它从“分享提示”升成平台能兑现的约定 |
| 说话人请求头与轮次记录 | `plugin-spec.md:2456-2462`、`mcp/upstream.rs:216-235` | 递给插件的 `_meta`，群聊单人时可以代人 |
| 确认卡片、源会话、按人推送 | [ADR-0016](0016-mcp-bus-and-confirmations.md)、`mcp/audience.rs` | 卡片模板、结果卡 |
| 深链 `wonton://tab/plugin-path/<插件>/<base64url 路径>` | [ADR-0041](0041-app-links-and-shortcuts.md)、`protocol/src/link.rs:66-75` | 插件自己拼、核心校验；通知字段改名 `link` |
| 通知、按级别的推送类别 | [ADR-0047](0047-plugin-notices.md)、`gateway/notify.rs` | 收件人、`link` |
| 分享进来进聊天、Bot 存文件 | [ADR-0097](0097-share-in.md)、[ADR-0027](0027-bot-save-file.md) | 通向插件（`inbox`）和通向共享文件夹 |
| 结果文件按编号、按人授权 | [ADR-0020](0020-file-service.md)、`files/bot.rs:120-199` | 按房间可见 |

## 决策

八项共用一个原则：**核心只递身份和指针，凭据和字节都留在插件自己的数据卷里**。核心多存一份就会多一处过期、多一处泄露面；
插件拿到的只是“替谁办”和“东西在哪”，应用原生怎么建号、怎么给权限仍归应用。下面每项的五段照任务书写。

### 1. 家人 ↔ 应用内账号：插件按命令建号，核心只管什么时候叫它

**现状。** 按人的身份已经有了：OIDC 的 `sub` 和 `preferred_username` 是本机固定用户名（`wonton-docs/plugin-spec.md:710-760`），
代理登录能按经过验证的人填 `{user}`（`:861-880`），`webui.accounts: per_user` 声明应用按登录人分数据（`:672-676`、
`crates/wonton-protocol/src/manifest/web.rs:83`）。但 `lan.client` 的第三方客户端口令一把插件一份，值存在
`crates/wonton-core/src/clients/passwords.rs`，只按插件和标签索引，详情接口只给主人看。MCP 侧只有清单里写死的
`{secret:…}` 请求头，一把 key 全家用。移除家人时“他在插件应用里的账号由应用自己管理”
（`docs/adr/0045-family-members.md:82`），也就是没人建、没人回收。

**缺口。** 谁在应用里建这个人的账号、口令/API key 存在哪、插件容器和 MCP 工具和 App 壳三方各自怎么按人拿到、
人退出时怎么回收，四件都没人负责。Immich 要每人一个库加一个“家庭”账号，Jellyfin 要每人一个用户再走 Quick Connect，
Navidrome 要每人一个用户，都不是“发一个口令就够”的形状。

**设计。**

- 清单加顶层 `member_accounts`：

  ```yaml
  member_accounts:
    service: photos
    commands:
      add: /usr/local/bin/wonton-member add
      remove: /usr/local/bin/wonton-member remove
      credential: /usr/local/bin/wonton-member credential
  ```

  形状照 [ADR-0110](0110-one-hermes-container-per-person.md) 的 Bot 命令：核心经 Docker exec 在 `service` 里执行，
  标准输入一条 JSON；退出码 0 才是成功，2 是输入非法，3 只给 `credential` 表示这个人的凭据失效，其余一律如实报“上游出错”
  并带标准错误。
- 三个命令都由核心调，插件不猜时机：

  | 命令 | 核心什么时候调 | 输入 |
  | --- | --- | --- |
  | `add` | 安装完、家人加入并获授权能打开这个插件、角色改动后变成能打开 | `{"user":"<本机用户名>","admin":false}` |
  | `remove` | 角色收回、移除家人、卸载且保留数据时 | `{"user":"<本机用户名>"}` |
  | `credential` | MCP 调用需要这个人的凭据、App 详情页索取第三方客户端账号 | `{"user":"<本机用户名>"}` |

  `admin` 由核心按 [ADR-0088](0088-family-accounts.md) 的 `plugin_admin` 算，插件据此在应用里给管理员。`add` 对已存在的
  账号只对齐资料和权限，不报错；`remove` 同样幂等，账号可以停用而不是删除（应用自己定，保数据）。
- 账号和凭据存在插件数据卷，文件 0600，核心不存。`credential` 在标准输出回
  `{"username":"…","password":"…"}` 或 `{"username":"…","token":"…"}`，核心只转发给当次请求方，不落盘。
- MCP 服务自己按 `_meta.app.wonton/user`（第 2 项）查自己的表，不从核心拿凭据。
- App 壳的网页仍走 OIDC 或 `sso: proxy`；Jellyfin 不支持前者也不适合后者，首次打开插件自己的 Quick Connect 页
  （`notes/video/plan.md:329`），网页里的身份已经由 OIDC 确定。`lan.client` 的详情改成每人只回自己那一条，
  口令按（插件、标签、人）存。
- 调用顺序和失败：`add` 失败就让这次授权停在“已授权、上游未就绪”，带重试按钮；安装时失败则整个安装失败并写明。
  重试和容器重建后，核心对当前授权名单里没有账号的人重放 `add`（`add` 幂等靠这个）。
- 凭据过期（用户在上游改了密码）：`credential` 退出码 3，核心把这条标记成待重设，App 提示去插件页，不静默降级。

**没选的方案。**

- 核心统一发口令：只够 HTTP Basic 一类应用，Immich 要邀请加库、Jellyfin 要用户加策略，核心就得懂每个应用。
- 只靠 `sso: proxy` 代人：Jellyfin 的网页不给代理信任头，写不进去。
- 核心存全部 key：核心成了全家凭据库，泄露面从插件数据卷搬到核心，应用改了密码核心也不知道。
- 一人一份插件实例：ADR-0045 已否，底价太重。

**给哪个组件。** `wonton-protocol`（清单字段和类型）、核心的插件生命周期（安装、授权、移除、重建重放）、
`clients` 详情接口、`wonton-docs/plugin-spec.md`；插件侧实现三个命令，不改上游应用。

### 2. 群聊里替谁办事：把这一轮的说话人如实递下去

**现状。** Bot 每次 MCP 请求带 `X-Wonton-On-Behalf-Of`，核心据此认说话人、算 AI 动手级别，并记进这一轮
（`wonton-docs/plugin-spec.md:2456-2462`）。服务端读请求头后有 `Caller.sender` 和 `turn_sender`
（`crates/wonton-core/src/mcp/server.rs:270-300`），但只用于 `audience.rs:105-140` 算级别。
`tool_call_identity`（`crates/wonton-core/src/mcp/mod.rs:347-381`）只认私人 Bot 的归属人，共享 Bot 走第一个分支就返回
`CallIdentity::EMPTY`。[ADR-0088](0088-family-accounts.md:166-167) 已经定了“共享 Bot 认得出这一轮只有一个人时就带这个人”，
代码没做。

**缺口。** 插件根本不知道是谁在群里说的这句，只能全家一套权限；让模型自己传 `person` 又常挑错
（`notes/photo/agent.md:88` 把这种写法当临时办法）。

**设计。**

- 核心在转给插件的 `tools/call` 的 `_meta` 里加两项：

  - `app.wonton/speakers`：本轮说话的家人用户名，去重。来源只有一个：这一轮进行中的轮次的 `turn_sender`，
    按它们在轮次里出现的先后排；单条消息就是 1 个，凑不到 8 个就不写 8 个。认不出来不带。
    和 `X-Wonton-On-Behalf-Of` 冲突时以请求头为准（头是 Bot 发出的原始事实，`turn_sender` 只是核心的副本）。
  - `app.wonton/room_people`：来源房间的成员里，既能打开这个插件、又能用这个 Bot 的人，去重。
    带的门槛是“核对过”：Bot 与**每个说话人**都在该房间的 `joined_members` 里（`audience::checked_origin` 用的那次查询，
    核的是 Bot 与收件人，这里要连说话人一起核）；核一次的结果按房间缓存 60 秒，别每请求登录一次。

- `app.wonton/user`（`crates/wonton-protocol/src/family.rs:50`）改成：私人 Bot 是归属人；共享 Bot 且 `speakers` 恰好一个
  人才带那一个；其余不带（和现在一样）。共享 Bot 多人同轮时不猜谁为主。
- **`user` 和 `app.wonton/capabilities` 一起动**：`capabilities` 按被带上的那个人重算角色（现在两者一起递，
  `upstream.rs:517-533`，只给 `user` 不给 `capabilities` 时插件会拿归属 Bot 的权限，共享 Bot 里说话人就算是管理员也拿不到
  `PluginAdmin`）。语义变更写进 ADR-0098 第 3 节，`family.rs:50` 的注释一起改。
- 这三项都只是提示：确认口径、AI 级别、放行时段仍由核心按 `ai_cap`、`audience` 算，插件不能拿 `speakers` 当授权。
  插件用 `app.wonton/capabilities` 和核心的级别判断动手幅度。
- 插件侧：一个工具只要有一个可选参数就按人写时，模型从 `speakers` 里取值填 `person`；只一个人时插件可以直接当默认。
  卡片抬头写“替 X”，多人时写“替 X、Y”。
- 不认识 `_meta` 的插件什么都不受影响：字段都可选，缺失时按“家里共用”处理（和现在一样）。

**没选的方案。**

- 只加一个 `on_behalf_of` 工具参数：模型会传错、会编人，而且参数到了插件又没法核。
- 每个房间一条 MCP 连接：Hermes 会 park 空闲连接，多开一份内存和加密会话成本，ADR-0111 实测过叫不醒那一套。

**给哪个组件。** 核心的 `mcp::mod::tool_call_identity` 和 `mcp::server`（把已经认下的 `sender` 递进 `_meta`）、
`wonton-protocol` 的键常量、`wonton-docs/plugin-spec.md`；插件侧由各块的工具自己读。

### 3. 事件投递：插件有事说，核心按人排队并叫醒他的 Agent

**现状。** 没有事件通道。`notify` 只进主人的“Wonton”私聊（`wonton-docs/plugin-spec.md:2178-2212`），发完就完了，没人回话。
MCP 订阅只收 `tools_list_changed`（`crates/wonton-core/src/mcp/server.rs:290-310`）。`notifications/resources/updated`
在 Hermes 上被 park 住，实测叫不醒（[ADR-0111](0111-bot-request-retryable-restart.md)）。

**缺口。** 新照片、下载完成、播放结束、新片入库这类事，要送到具体的人，还要能让空闲的 Agent 自己动起来去处理；
现在插件只能发一条不可回话的通知，Agent 得等主人先说话。

**设计。**

- 加**两个**权限 `events` 与 `files`，进 `plugin-spec.md:1404` 那张表、`Permission` 枚举、
  安装预览、商店元数据和四种语言说明。`files` 的含义是“能收核心放进来的字节”，`inbox` 必须声明它；
  `member_accounts` 只走 stdout 凭据，可以不声明。
- 申请了 `events` 的插件在插件网络里拿到别名
  `events.wonton.internal` 和注入变量 `WONTON_EVENTS_URL`（如 `http://events.wonton.internal:8080/events`），
  和 `notify` 一样不用令牌、按来源地址认插件。
- `POST` 一条 JSON，成功回 `202` 和 `{"event_id":"…"}`。事件的 `data` 原文存着、只在 `wonton__events` 里回，
  投递单里不放（投递单只带 `truncated` 与 `event_id`）；同一个人有两个私人 Bot 时，事件绑定收件 Bot，
  `wonton__events` 取的时候加 60 秒租约，别人同时取不到同一条。
- `person` 省略时取主人；主人打不开这个插件就退回按人通知（不叫醒），和通知那条的规则一致。

  | 字段 | 必填 | 说明 |
  | --- | --- | --- |
  | `kind` | 是 | 事件类型，小写 ASCII、`_` 和 `.`，1–64 个字符，建议 `<域>.<动作>`，如 `photo.added`、`download.done`、`play.finished` |
  | `person` | 否 | 收件家人的用户名；省略是主人。必须是能打开这个插件的成员 |
  | `title` | 是 | 一句话，最多 120 个字符 |
  | `body` | 否 | 纯文本，最多 2000 个字符 |
  | `path`、`link` | 否 | 同第 4 项，二选一；都写以 `link` 为准 |
  | `data` | 否 | 给插件的结构化载荷，任意 JSON，最大 4 KiB；不错给 Agent 的会话，只经 `wonton__events` 取 |
  | `dedupe_key` | 否 | 去重键，最多 64 个字符；同一发件人、`kind`、`dedupe_key` 10 分钟内的多条合并计 `count`，只叫醒一次 |
  | `level` | 否 | `info`（默认）、`warning`、`critical`；`warning` 以上在退回通知时用 |

- 核心按人排队（每个发件插件、每个人一条队列）：保留 7 天、每人 200 条，超出先丢最旧并标 `truncated`；每个插件每分钟 10 条，
  超了回 `429`，`Retry-After` 按窗口算。
- 投递顺序：
  1. 这个人自己拥有、能用这个插件、没停用的私人 Bot（多条时取最早的一个）；
  2. 他能用、没停用的共享 Bot；
  3. 都没有就退回第 4 项的按人通知，事件留在队列里，不叫醒。
- 叫醒就是核心以核心账号在那条 Bot 和他自己的私聊里发一条 `m.text`，文案用核心的四种语言（“相册有新照片”一类不写死，正文取 `title`），
  `content` 里带 `app.wonton.event: {"id":…,"kind":…,"count":…}`。消息到不了会话，
  记一次“投递失败”，事件仍在队列，下一次有事件或每 5 分钟重试一次。
- Agent 的一轮里能调两个核心工具：`wonton__events {since?, wait_seconds?}` 取自己收件队列里还没 ack 的事件
  （`wait_seconds` 最多 60 秒，给了就长轮询到有事件或到点）；`wonton__events_ack {ids:[…]}` 标已处理。没 ack 的 7 天后过期。
  两个工具只给申请了 `events` 的插件、且这个 Bot 是当前收件人的 Bot。
- 前提（待验证）：`MATRIX_ALLOWED_USERS` 只拼归属人和联系人（`crates/wonton-core/src/plugin/contacts.rs:65-77`），不含核心账号；
  要先加上，还得实测 Hermes 会不会把核心当合法说话人开一轮，而不是当第三方噪音丢掉。过不了就先退回“按人通知 + 下一条消息时取队列”。
- 事件只此一个入口：不给插件加订阅表，插件已有的 watch 表自己筛，核心不按 `kind` 过滤。

**没选的方案。**

- 只用 MCP 订阅通知：park 住的会话收不到，叫不醒（ADR-0111 已实测）。
- 插件自己以 Bot 身份发 Matrix：每个插件都得管 Bot、加密设备、多开一个客户端，还得自己挑收件人和算级别。
- 核心做订阅表、按 `kind` 路由：订阅是插件本身就知道的事（它维护 watch 表），核心再存一份只会两边不一致。

**给哪个组件。** 核心的 `gateway/events.rs`（新）、事件队列与重试、`mcp` 的两个核心工具、`plugin/contacts.rs` 的允许名单、
`wonton-protocol` 的事件类型、`wonton-docs/plugin-spec.md`；插件侧负责在事件到达时产生事件，不改上游应用。

### 4. 通知发给指定家人，不只发给主人

**现状。** `PluginNotice { plugin_id, plugin_name, level, title, body, path }`
（`crates/wonton-protocol/src/plugin_notice.rs:17-28`），核心以核心账号发进**主人**的“Wonton”私聊，限流按插件算
（`crates/wonton-core/src/gateway/notify_limit.rs`、`wonton-docs/plugin-spec.md:2201-2212`），规范写明“通知只发给用户本人，
家人不收”（`:2209`）。家人各自的通知房间和 `notify_person` 已经有了（`crates/wonton-core/src/files/host/people.rs:132-180`），
只是插件通知没走它。

**缺口。** 备份失败、下载完成这类事只该给相关的那个人（如“妈妈的手机备份失败”），现在全丢给主人，主人变成人肉转发。

**设计。**

- 通知体加两个可选字段：

  - `to`：收件人用户名数组，去重，最多 16 个；省略是主人。每个都必须是已加入、当前能打开这个插件的成员，否则 `400 invalid_input`
    并列出不认识的人。主人永远可以收（哪怕打不开插件：他是装的人）。
  - `link`：第 6 项的深链，绝对 URL 或 `wonton://`；和 `path` 同时出现时用 `link`，`path` 保留给不认识新字段的旧核心。
- 每条通知按 `to` 拆成每人一条，正文、级别、`path`/`link` 一样，发进各自已有的通知房间（`NoticeRoom` 按人缓存）。以核心账号发出，
  不走 Bot，不叫醒 Agent。
- 限流按（插件, 人）各算一套，仍是每分钟 10 条、24 小时 200 条；超限的提示也发给那个人，不发给主人。
- 结构化字段 `app.wonton.notice` 里带插件、级别、`link`（或 `path`），App 的插件通知类别和用户关掉它的开关不变。
- 兼容：不写 `to` 的现有插件行为一点不变。`to` 里有主人时和现在的路径相同。

**没选的方案。**

- 只在正文里写“给妈妈”：App 没法按收件人过滤，家人收到的还是在主人的私聊里。
- 每个收件人一个端点、插件各发一次：插件得知道家里有谁、谁能打开它，核心已经在算这些。

**给哪个组件。** `wonton-protocol` 的 `PluginNotice`、`gateway/notify.rs` 的分发、`notify_limit` 的键、`wonton-docs/plugin-spec.md`；
插件侧只多写字段。

### 5. 确认卡片和结果卡片用插件给的模板

**现状。** 卡片标题按 MCP `title` → `annotations.title` → `description` 依次取，正文是参数 JSON 前 600 字节
（`crates/wonton-core/src/mcp/cards/tool.rs:11-33`、`mcp/cards.rs:547-559`）；同一张卡片在确认后原地变成结果卡。
[ADR-0016](0016-mcp-bus-and-confirmations.md:61-66) 要求“对象不可见时明说，不把参数当核实过的对象”。

**缺口。** 用户看到的是一屏钥匙名和 JSON；插件明明知道“把 3 张照片加进「日本旅行」”，却只能让模型从参数里拼。

**设计。**

- `tools/list` 里每个工具的 `_meta` 可带 `app.wonton/card`：

  ```json
  "app.wonton/card": {
    "title": "把 {arg.count} 张照片加进「{arg.album}」",
    "lines": [
      {"text": "来源：相机胶卷"},
      {"text": "原件保留", "arg": "keep_originals"},
      {"text": "同名文件会被覆盖", "emphasis": "danger", "arg": "overwrite"}
    ],
    "result": {"summary": "已加入 {arg.count} 张"}
  }
  ```

  `text`、`summary` 是模板。占位符只认 `{arg.<顶层的标量参数>}` 和 `{arg.<数组参数>.count}`；对象参数只许再取一层
  `{arg.<对象>.<键>}`，不认下标、函数和默认值。
- 取值都是调用时的实际参数。字符串超过 80 个字符截断加省略号；`null`、空串、空数组渲染成“无”。被引用但没传的参数：
  在 `title` 里就让整张卡片退回现在的原始渲染；在行里只丢掉这一行。`result.summary` 引用缺失时退回“已完成”。
- `lines` 最多 12 行，每行渲染后最多 120 个字符，标题 80，`summary` 200；超限按字符截断，不报错。`emphasis` 只有
  `normal`（默认）、`warning`、`danger`，颜色由两端按设计系统画（[ADR-0026](0026-design-system.md)）。
- 渲染由核心做，插件给的是纯文本模板，不带 HTML/Markdown。模板坏掉（不是字符串、占位符写错）在 `tools/list` 校验时报出
  工具级警告，不拦工具本身。
- 结果 `_meta` 里核心写 `app.wonton/card_result: {"summary":"…"}`，Agent 能在结果里读到；卡片本身用它替换正文，
  原始 JSON 仍收进“详情”。确认按钮、风险行、AI 级别和“替谁”那一行仍由核心加（[ADR-0024](0024-confirmation-scope.md)、
  第 2 项），模板换不掉。
- 普通工具没有 `app.wonton/card` 时行为完全不变（现状），所以这是纯加成。

**没选的方案。**

- 让插件直接给整张卡片（结构、按钮、风险全交出去）：核心没法统一本地化，也没法按自己的判断加风险行和权限行。
- 走 MCP elicitation：ADR-0038 的 M1 先不做，Hermes 的版本也还不支持。
- 只让插件多给一个 `summary` 字符串：模型常在摘要里省事，参数一多仍旧是 JSON 墙。

**给哪个组件。** `wonton-protocol` 的卡片模板类型与校验、`mcp/cards` 的渲染、`wonton-docs/plugin-spec.md` 的 `tools/list` 一节；
插件侧自己写模板。

### 6. 深链回插件页面：插件自己拼，核心只校验前缀

**现状。** `wonton://tab/plugin/<插件>/<页面>` 和 `wonton://tab/plugin-path/<插件>/<base64url 路径>` 两种都能解
（`crates/wonton-protocol/src/link.rs:66-90`）；看板里 `wonton://` 只当一般链接（`crates/wonton-client/src/chat/links.rs:53-60`），
插件网页的完整 URL 会被认成“这个插件的页面”（`crates/wonton-client/src/chat/links.rs:20-44`）。通知的 `path` 只收相对页
（`wonton-docs/plugin-spec.md:2187`）。

**缺口。** 插件知道该打开哪一张照片、哪一张专辑，但说不出一个能点的地址：`path` 只能进插件根，不能带页面里的状态；
结果卡里也没有可点的入口。

**设计。**

- 需要深链的一律用现成的 `wonton://tab/plugin-path/<自己的插件 id>/<base64url(路径和 query)>`；插件能在任何返回给用户的地方拼，
  不用知道核心地址。核心只校验：scheme 是 `wonton://`、段是 `tab/plugin-path/`、插件 id 就是发件的这个插件，否则 `400 invalid_input`。
- 通知和事件加 `link`（第 4、3 项）；工具结果的 `_meta` 加 `app.wonton/deep_link`（第 5 项的结果字段），核心转成结果卡里一行
  可点的“打开”，失败只当普通文本。`path` 照旧收，由核心换成 `plugin-path` 形式的 `link`；两者都有时用 `link`。
- 校验放行第二形状：插件自己 origin 下的完整 `https://` URL（同 `links.rs:20-44` 的认法；副作用上文已说，这里只用来转形），
  核心改写成 `plugin-path`。别的 origin 和 `wonton://tab/` 下别的段一律不收，避免插件把人带去别处。
- 打开时按 [ADR-0033](0033-plugin-tabs.md) 的插件标签走；插件没装、被停用、这个人打不开时，链接仍显示，点了报“插件不可用”。
  App 没有对应标签就落到插件首页。
- 深链只当导航，不带授权：页面自己按 OIDC/proxy 认得的人取数据，链接不携带任何令牌。

**没选的方案。**

- 新开 `wonton://app/<插件>/…` 一类 scheme：现有解析器和别的客户端已经认 `tab/plugin-path`，新开只会多一套。
- 只让核心按 `path` 拼：插件拿不到核心的外部地址，也没法把页面状态塞进 `path`。

**给哪个组件。** `wonton-protocol` 的 `link` 解析与校验、`mcp/cards` 的结果行、`gateway/{notify,events}.rs` 的字段、
`wonton-docs/plugin-spec.md`；插件侧自己拼链接。

### 7. 工具结果在房间里谁能看

**现状。** 文件记录的访问集只有 `Owner`、`User`、`Bot`（`crates/wonton-core/src/files/record.rs:29-33`），Bot 登记时只加它自己
（`files/bot.rs:70-110`），之后转给归属人（`:186-199`）。看板里的 Bot 读不到家人发给它的文件：`for_bot` 只认 `Bot`
（`:120-135`）。按房间的授权只有一个客户端接口 `POST /_wonton/files/{id}/grants`，也只把房间里的 Bot 加进去
（`crates/wonton-core/src/gateway/files.rs:8-9`）。

**缺口。** 群里问“这周末去哪几张”，Bot 把照片结果发回群里，只有 Bot 和它的归属人（主人）能打开；别的家人点开是 403。
反过来，家人在群里发给 Bot 的照片 Bot 也读不到，因为记录里没有那个房间。

**设计。**

- `Accessor` 加 `Room(房间内部 ID)`（存本地 ID，不是别名的 `!xxx:server`，别名可改）。加的时候核的是 Bot 与**每个说话人**（`verify_room`），
  报的来源没核过就不加；人读的时候另核一次那个**读的人**在不在房间里。
- 鉴权时按当下的成员算，不拍快照：查 `joined_members` 现算这个人在不在房间。缓存的是该房间的 `joined_members` 列表 60 秒（人在不在每次在这份列表上查，列表本身可能旧 60 秒），
  所以退了的人在 60 秒内还能读到，超过就失去；记录本身的 24 小时过期规则不变。
- **“核对过”写死成一个函数**（`audience::verify_room(id, bot, 说话人…)`）：Bot 和每个说话人都在 `joined_members` 里才算；
  它只回答“是不是这个房间的人”，不给确认卡用（确认卡的 `checked_origin` 照旧单独走）。
- **降级要能操作**：旧核心读到 `Accessor::Room` 会让整条记录反序列化失败（`record.rs:28-64` 的 `try_from`），所以
  ①`spec_floor` 把用到 `member_accounts`（写了 `status` 才登记）、`inbox`、`events`、`files` 的最低 `spec` 抬上去，旧核心按版本号拒装并提示升级（`tools/list` 与结果里的 `_meta` 不受它管，旧核心忽略即回退 `keeper`）；
  ②给一条降级脚本（卡 11）：把记录文本里的 `room:<id>` 换成登记它的 `bot:<id>`，改不了就删那条并记日志；升级说明写清什么时候跑、会丢哪些授权；
  “降回旧核心时读不出的那条”按 keeper 处理；
  ③`plugin check` 对声明了这些却低于门槛的清单当场失败。
- 工具结果 `_meta` 加 `app.wonton/result_scope: "room" | "keeper"`。房间核对过时**也默认 `keeper`**。要开房间可见，插件得在 `tools/list` 的对应工具 `_meta` 里写
`app.wonton/room_results: true`（工具级、缺省 false、坏值算 false 并记一条警告）；只有声明了、且来源房间经 `verify_room` 核过，才按 `room`；插件写 `keeper` 就退回现在的行为
  （Bot 和归属人）。私聊里没有房间，一律 `keeper`。
- 反向：家人在房间里发出的附件，核心在发出时把同一个房间的 `Room` 挂到记录上；Bot 在自己这一轮的来源房间（同样核对过）能读。
- App 的附件面板标出来源：“这个房间可见”或“只有你”，作者能把它改成更窄的 `keeper`。给人看的复制/下载不额外要权限，
  但拿到的仍是编号，编号本身不再等于授权。
- 房间里的人后来被踢出、或房间被删：最迟 60 秒后失效；同一条记录如果还有归属人，归属人照旧能开。

**没选的方案。**

- 登记时把成员名单抄进记录：后进群的人打不开，退了的人还一直能打开，而且要有人负责重算。
- 公开编号、谁拿到谁能开：和“群聊里查到的私人照片不能全房间可见”直接冲突。

**给哪个组件。** `wonton-protocol` 的访问者类型（见接口清单）、`files/*` 的鉴权和登记、MCP 结果归纳、`gateway/files`、
`wonton-docs/plugin-spec.md`；App 附件面板的可见范围标记归壳。

### 8. 把文件交给插件，收件进共享文件夹

**现状。** 出箱和 Bot 存文件都有了（[ADR-0027](0027-bot-save-file.md)），分享进来只进聊天（`docs/adr/0097-share-in.md:18-21`）。
工具结果的搬运上限是 1 MiB 一件、48 MiB 一轮、100 个文件（`wonton-docs/plugin-spec.md:2352,2399-2401`）。

**缺口。** 插件有了文件编号也读不到字节：MCP 的结果通道只回给 Agent，插件容器里没有地方收。分享照片进 Immich 要先进聊天、
再让 Bot 存、再让 Agent 调上传，三步。反方向，主人把新下的片子放进共享文件夹，插件不知道要扫。

**设计。**

- `inbox` 声明里带一条 `scan` 命令（形状同 `member_accounts` 的命令：容器内执行、退出码 0 成功、2 输入非法、其它算失败），
  核心放完字节调它；同一目录 30 秒内的多次放进合并成一次调用，失败只记日志并在下一次放进时重试，不阻塞分享本身。
  没写 `scan` 就等于插件自己定期扫，核心不催。
- 工具在 `tools/list` 的 `_meta` 里声明哪个参数收文件：`"app.wonton/files": ["files[]"]`，元素是顶层数组参数的 Rust 名称，
  最多 4 个参数。调用时这些参数的值是文件编号数组；上限同结果通道（一件 1 MiB、一轮 48 MiB、100 个文件）。
- 清单声明收件箱：

  ```yaml
  inbox:
    service: photos
    path: /inbox          # 容器里的绝对路径，挂的是插件数据卷
  ```

  要声明 `files` 权限；没声明 `inbox` 的插件，`app.wonton/files` 里的参数原样传编号。
  核心按编号取到字节，拷进 `<path>/<8 位随机>/<安全名>`，把参数换成 `inbox:<子目录>/<名字>` 字符串；插件不用认识核心的编号，
  也不经网络回取。
- 安全名：去掉斜杠和控制字符、Unicode 归一、最长 120 字节，重名按 `-2`、`-3` 顺延。子目录按次生成，同一次调用的文件都在一个子目录里，
  插件处理完可以整个删；核心 24 小时后也删，插件先删了也不报错。
- 没有挂载的 `inbox`、路径不可写、取字节失败，回工具错误 `source_unavailable`（`app.wonton/failure`），不空跑。
- 另外给一个核心工具 `wonton__give_file {file_id, plugin}`：Agent 把当前会话里的文件放进某个插件的收件箱，返回 `inbox:` 路径。
  只给能打开那个插件、且这个插件声明了 `inbox` 的场合。
- 共享文件夹方向：分享页在“发进聊天”之外加“存进共享文件夹”。收件人是主人自己时直接放进他选的文件夹并触发插件的 `scan_start`
  （[ADR-0097](0097-share-in.md) 的分享页多一个去向，主流程不变）；给家人时仍走聊天，由对方自己决定存不存。

**没选的方案。**

- 让插件按文件编号回核心取字节：插件要再拿一套核心鉴权，记录归谁、能读多久都得再定一遍，还多一次网络往返。
- 传 `wonton-share:` 路径：插件没挂那个文件夹，路径对它是天书。

**给哪个组件。** `wonton-protocol` 的清单字段和工具 `_meta`、`files/*` 的拷贝与清理、MCP 调用前的参数改写、
`wonton-docs/plugin-spec.md` 的 `inbox` 一节；分享页去向归壳。

## 理由

- 三块八项指向同一个短板：核心认得出人马和文件，但认不出“这一件事替谁、给谁、谁能看”。补的是这条最短的路，
  不是三套媒体特例。
- 都沿用已有的形状：按人身份用 `_meta.app.wonton/*`（和 `failure`、`confirmation`、`origin` 同前缀），操作走命令加 JSON，
  链接走已有的 `wonton://tab/plugin-path`，结果仍由核心画。新的只有名字，没有新协议。
- 凭据和字节留在插件数据卷：核心保持“厚在规则、薄在数据”，应用改密码、换库、加字段都不用动核心。
- 每一项都可选：不写模板回原始 JSON，不声明 `inbox` 传编号，不写 `to` 发给主人，不报来源就是 `keeper`。旧插件升级不用跟着改。
- 可见性默认收窄：只有报的来源房间核对过才给房间看，多人同轮不猜主说话人，Agent 的身份只当提示，授权仍由核心算。

## 代价

- 清单和规范一次加 `member_accounts`、`inbox` 两组字段和五个 `_meta` 键，校验、预览、`plugin check --official` 和各语言的
  文档都要跟上，插件作者也要多写模板和命令。
- 核心多四个要持久化、要清理的东西：事件队列（7 天）、收件箱（24 小时）、按（插件, 人）的通知限流、房间访问者的实时成员查询。
  进程重启后事件留在磁盘、收件箱残留照删，都得测。
- `events` 是新的权限面：安装预览、商店元数据、四种语言的说明都要加，还要防插件刷事件把 Agent 一直叫醒。
- **三处破坏性改动**（都要升级说明和降级路径，不是一处）：`lan.client` 从插件一份口令改成按人（存储键、详情接口、复制界面、迁移）；
  `app.wonton/user` 在共享 Bot 单人时开始带人（语义变更）；结果可见性引入 `Accessor::Room`（旧核心读到会读不出那条记录）。
- 房间可见是放开：一条结果发给整房间，事后想收窄只能由作者改；私密内容要靠插件写 `result_scope: keeper`。
- 模板由插件写、不做翻译，同一插件要维护多种语言的模板；模板坏了的回退只有原始 JSON 兜底，观感差。
- 测试面变大：房间成员进出、同轮多人、事件合并、深链前缀越权、收件箱重名和越界都要有端到端用例，不能只测类型。

## 兼容与迁移

- 加字段一律可选，但 `Manifest` 是 `deny_unknown_fields`（`Provides` 是 flatten map，新键会落进 `capabilities` 再被 `ProviderService` 拒）：
  **旧核心会按 `spec` 版本拒装并提示升级**，不是静默忽略。
  用到新字段的清单要升 `spec` 并在 `spec_floor.rs` 登记；`plugin check` 当场拦低位清单。
- 旧插件在新核心上行为不变：通知仍到主人、结果仍 `keeper`（插件没在 `tools/list` 里声明支持 `room` 就不开门）、参数仍传编号。
- 唯一要迁的一次：`lan.client` 的口令表从（插件, 标签）改成（插件, 标签, 人）。旧值升级时复制给主人一份并标“待重设”，
  不猜该给谁。
- 共享 Bot 单人对话时 `app.wonton/user` 从“不带”变成“带那个人”，`capabilities` 跟着按那人重算：ADR-0098 第 3 节同批改，
  personal_data 的实现要按新语义放行；插件不读这个字段也必须能跑。
- `notify` 的 `path` 保留，`to`、`link` 是新增；`plugin_notice.rs:1-10` 的文档注释（还写“发给主人”）一起改；`to` 里有人已移除时整条 400，不悄悄少发。

## 待验证

全部未核实，按影响排序；前四条不过就改设计，不硬推：

1. **核心账号能叫醒 Hermes。** 核心账号不在 `MATRIX_ALLOWED_USERS` 里（`crates/wonton-core/src/plugin/contacts.rs:65-77` 只拼归属人和联系人）。
   实测：在开发栈给一个 Hermes Bot 发一条带 `app.wonton.event` 的 `m.text`，看它是否开一轮并调 `wonton__events`。
   加密私聊里核心账号能不能发得进去也要看。
2. **Jellyfin 能按人建号再走 Quick Connect。** 用管理员 API 建用户、设策略，再用 Quick Connect 页登录；`notes/video/plan.md:329`
   自己就标了没实测。不过就退回“首次输一次密码”，`credential` 改回一次性口令。
3. **Immich/Navidrome 的建号命令。** Immich 建用户、建库、加“家庭”账号的共享；Navidrome 的管理员建用户。三家用同一套命令形状是否够，
   还是要给 `add` 加应用特有字段。
4. **（实验卡已做，2026-11-23）工具 `_meta` 能不能带到插件。** rmcp 不丢：插件工具的 `_meta` 经核心到 Agent 原样保留；
   核心自己写进 `tools/call` 的 `_meta`（`app.wonton/user`、`app.wonton/capabilities`）也到得了插件。但核心现在不把调用方自己塞的 `_meta`
   往下递（只用自己的身份键和 rmcp 的 `progressToken`），所以 `tools/list` 的 `app.wonton/card`、`app.wonton/files` 直接可用，
   `tools/call` 的新键（`speakers` 等）必须由核心在 `upstream.rs::call_with_identity` 里写。不用换路。
   测试：`gateway::mcp_tests::tool_meta_survives_the_bus_and_core_call_meta_reaches_the_plugin`。
5. **房间成员查询的开销。** 每次读文件都查 `joined_members` 在几十人的房间里能不能接受；不行就加 30 秒内的短缓存，仍以实时为准。
6. **命令在容器里的用户和属主。** 插件容器常以 `PUID`/`PGID` 跑，核心 exec 带哪个用户、写出的 0600 文件谁读得到。
7. **事件合并和限流一起的表现。** 10 分钟内 200 张照片合并计 `count` 时，Agent 一轮能不能一次取完、会不会反复被叫醒。
8. **通知多收件人的放大。** 一条通知拆给 16 人算 16 次限额还是 1 次，按人算后 200 条/天的余量够不够。
9. **深链在两端壳里的落地。** `plugin-path` 已有解析，但通知点开、结果卡点开、没插件时的回落没有端到端用例。
10. **收件箱的子目录清理。** 插件按约定删掉子目录后核心再删不报错；容器重建时残留能不能扫掉。

## 验证计划

接口 PR 只改类型和文档，至少带：模板校验（缺参、超长、坏占位符）、`speakers` 的排序去重和 `user` 的三条规则、深链前缀越权、
`to` 的收件人校验、`inbox` 路径和文件名的非法用例、`Accessor::Room` 的序列化往返。

实现卡按项各带一条先红后绿的行为测试，端到端落在 194 开发栈：真装一个插件，走安装→授权家人→群里单人说话→Agent 调工具→
结果回群→家人点开→移除家人。事件那条用一个会发事件的假插件，验叫醒和 ack；文件那条验 24 小时清理和 `source_unavailable`。

## 边界

- 长任务、传输、播放会话、离线下载、断点续传、任务队列和重试策略归 [ADR-0113](0113-media-transport.md)。
  本篇只给 Agent 调它们时要的身份、事件、卡片和文件通道；0113 的进度事件用本篇的事件形状发，不另造一套。
- 不做插件间直接调用、不做 MCP elicitation、不做插件自定义卡片的完整渲染、不做按房间的长期 ACL 管理界面。

