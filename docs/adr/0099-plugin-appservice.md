# ADR-0099 插件声明 Matrix appservice：核心代为登记

- 状态：提议（2026-11-12）。取舍待主人定（见“待主人定”）
- 补充 [ADR-0007](0007-agents-and-bots.md)（Matrix 身份除了 Bot，多了一种由插件自己管的命名空间）、
  [ADR-0036](0036-plugin-privileges.md)（新权限 `matrix.appservice` 的级别和“谁拿不到”）

## 背景

海外 IM 走 mautrix 桥进 Matrix（主人已定）。聊天线要做 telegram、signal、discord、slack 等桥插件：上游都是 AGPL、独立容器、
官方镜像，以 Matrix [Application Service](https://spec.matrix.org/latest/application-service-api/) 接入。插件接 Matrix 只有
`matrix.bot` 一种方式：核心给一个账号、代登录一台设备。桥要的是另一样东西：一组 token 加一段用户名、别名的命名空间，
能随时建账号（远端每个联系人一个“影子用户”）并以它们的身份发消息。核心没有登记 appservice 的能力。

Tuwunel v2.0.0（`deploy/install/compose.yaml` 钉的版本，tag 提交 `a3feae1`，2026-11-10）的 appservice 有三种登记方式，
见 [docs/appservices.md](https://github.com/matrix-construct/tuwunel/blob/v2.0.0/docs/appservices.md)：

1. 管理房间 `#admins:<服务器名>` 里发 `!admin appservices register` 加 YAML 代码块：存进数据库，热生效，重启后还在；同 id 再登记就替换。
2. `tuwunel.toml` 里的 `[global.appservice.<id>]`：启动时读，管理命令删不掉、换不掉。
3. `appservice_dir` 目录：启动时扫，增删文件要重启 Tuwunel。

本篇的实测结论都在“验证结果”一节。

## 决策

### 1. 清单

新权限 `matrix.appservice`，加一段 `x-wonton.appservice`：

```yaml
x-wonton:
  permissions: [matrix.appservice, network.egress]
  appservice:
    service: bridge                 # 收 Tuwunel 推送的服务
    port: 29317                     # 这个服务上 appservice 的监听端口
    registration: registration.yaml # 上游生成的登记文件，相对 WONTON_APP_DATA
    users: [telegram_*, telegrambot]
    aliases: [telegram_*]
    bot: telegrambot                # 桥的 bot 用户，主人和它私聊（第 6 节）
    encryption: false               # 桥的配置是否开了端到桥加密，决定私聊建不建成加密房间
```

- `users`、`aliases` 每项是 Matrix localpart 的字面量，可以以 `*` 结尾表示前缀。前缀必须以 `_` 结尾，去掉 `*` 至少 3 个字符。
  Bot 的用户名只有小写字母、数字和 `-`（`bot_localpart`、`check_bot_name`），所以以 `_` 结尾的前缀永远盖不到 Bot；
  主人、家人、好友的用户名允许 `_`（`owner::check_username`），靠第 2 节的查重。
- `bot` 必须是 `users` 里的一个字面量（不能只被前缀盖住），装完给主人建私聊用。
- 不支持 `rooms` 命名空间。
- 声明了 `matrix.appservice` 就必须有 `appservice` 段，反之亦然，只写一边清单不合法（`invalid_input`），同 ADR-0036 的写法。
- `kind: agent`、声明了 `matrix.bot` 或 `bots` 的插件不能声明 `matrix.appservice`（`invalid_input`）。理由见第 5 节。
- 核心给 `service` 注入 `WONTON_APPSERVICE_HOMESERVER`（网关的 Matrix 地址，同 Bot 的 `WONTON_MATRIX_HOMESERVER`）、
  `WONTON_APPSERVICE_DOMAIN`（服务器名）、`WONTON_APPSERVICE_OWNER`（主人的 Matrix 用户 ID）。插件用它们渲染上游配置
  （`homeserver.address`、`homeserver.domain`、`permissions`）。

### 2. 登记文件：上游生成，核心读、核对、改写

**token 由上游生成，核心不造。** mautrix 的镜像入口在 `/data/registration.yaml` 不存在时用 `-g` 生成登记文件和 token 并退出，
下次启动才跑桥（[docker-run.sh](https://github.com/mautrix/telegram/blob/main/docker-run.sh)，2026-11-12 查）。
上游的 token 同时写进它自己的 `config.yaml`，核心另造一套就要让每个插件再把 token 接进上游配置，Agent 也更难一次写对。

**核心怎么读：**

- 路径在插件数据目录下，核心逐段不跟符号链接打开（`openat2` 的 `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS`），只收普通文件，上限 64 KiB。
  这个目录插件可写：不这样做，插件把文件换成指向核心密钥的符号链接，核心就会读出别的东西。
- 核心容器以 root 运行（`deploy/dev/core/Dockerfile` 最终阶段没有 `USER`），上游镜像写出的 `root:root 0600` 或 `1337:1337 0600`
  都读得到。rootless Docker 下所有容器在同一个用户命名空间里，结论应当不变（**未实测**）。
- 解析失败、读不到、文件超限时，错误详情里不回显文件内容（内容里有 token）。

**核对**（任何一条不过都不登记，插件不装、不启动）：

| 字段 | 规则 | 不过时的类别 |
| --- | --- | --- |
| `namespaces.users`、`aliases` | 每条 `exclusive: true`；`regex` 按 `^@<字面量>(.*)?:<转义的服务器名>$` 识别（别名用 `#`），识别不了的写法一律拒；每条都能对上清单里的一项。文件可以比清单少，不能多 | `invalid_input` |
| `namespaces.rooms` | 必须为空 | `invalid_input` |
| 命名空间和现有用户 | 不能盖住任何现有本机用户：主人、家人、好友墓碑、Bot、`wonton-core`、`conduit`、其他 appservice 的命名空间和 `sender_localpart`；再向 Tuwunel 列一次用户（Synapse 兼容的 `list_users`）核对，盖住的只能是这个插件自己以前留下的影子用户 | `conflict` |
| `sender_localpart` | 同上，不能是现有用户；Tuwunel 登记时会建这个用户 | `conflict` |
| `id` | 不和其他插件的、也不和 Tuwunel 里已有的登记重名 | `conflict` |
| `as_token`、`hs_token` | 非空 | `invalid_input` |
| `url` | 不核对，核心整个换掉（第 4 节） | — |

`id` 保留上游的：mautrix 启动时用自己配置里的 id 调 `/_matrix/client/v1/appservice/{id}/ping`，改了 id 就对不上。

**保留命名空间。** 登记成功后，核心把命名空间和 `sender_localpart` 记进核心数据目录（0600），之后分配家人、好友、Bot 的用户名
时一并查重，撞上的回 `conflict`，写明被哪个插件占着。删数据卸载后也不释放（同好友墓碑）：Tuwunel 的影子用户删不掉，核心又是唯一的
JWT 签发方，放出去的名字被家人用上，JWT 登录就会登进那个现成的影子账号（`users.create` 不查 appservice 的命名空间，
见 [jwt.rs](https://github.com/matrix-construct/tuwunel/blob/v2.0.0/src/api/client/session/jwt.rs)）。

**token 的保管**，按 ADR-0064 对 `collect` 一类的口径：存核心密钥库，任何接口不返回，包括主人；日志、审计、错误详情都不写。
核心留一份是为了在 Tuwunel 丢了登记时（数据卷重建）能补登，以及网关认 as_token（第 4 节）。插件数据目录里那份是上游自己要用的，
随“保留数据”留或删。

### 3. 登记方式：核心进管理房间发命令

- 核心的管理账号 `@wonton-core` 用 `admin: true` 注册时，Tuwunel 只把它**邀请**进管理房间
  （[grant.rs](https://github.com/matrix-construct/tuwunel/blob/v2.0.0/src/service/admin/grant.rs)），
  而“是不是管理员”看的是有没有加入这个房间
  （[admin/mod.rs](https://github.com/matrix-construct/tuwunel/blob/v2.0.0/src/service/admin/mod.rs) 的 `user_is_admin`）。
  核心第一次要登记前解析 `#admins:<服务器名>`、加入。管理房间不加密，核心的原始 HTTP 客户端能直接发。
- 命令是 `m.text`，正文 `!admin appservices register` 换行加 YAML 代码块。Tuwunel 以 `m.notice` 回复，`m.relates_to` 指向命令事件；
  核心用 `/relations/{命令事件}` 取回复，成功是 `Appservice registered with ID: <id>`，失败以 `Command failed with error:` 开头。
  等不到回复按 `matrix_unavailable`。
- 每次登记前核心先把 `url` 换成第 4 节的中继地址，YAML 由核心重新序列化，不原样转发插件的文件。
- 管理房间的时间线里会留着这条带 token 的命令。房间里只有 `@conduit` 和 `@wonton-core`：主人经 JWT 建号走 `users.create`，
  不触发“第一个用户成为管理员”（`grant_admin_to_first_user` 只在完整注册路径上），所以主人不在里面。命令发出后核心撤回（redact）它，
  减少明文留存；Tuwunel 自己的登记表里本来就存着 token。

**生命周期：**

| 时机 | 核心做什么 |
| --- | --- |
| 首次安装 | 起容器 → 等登记文件出现（和安装等就绪共用一个期限）→ 核对 → 登记 → 重启 `service`（mautrix 生成文件后退出，靠重启策略回来会有退避）→ 照常等健康 |
| 重装、升级、核心启动时拉起插件 | 文件已在：起容器**之前**先核对、登记，桥一启动 token 就有效 |
| 文件内容变了（上游重新生成、token 轮换） | 每次起 `service` 前比对文件摘要，变了就重新核对、再登记。实测同 id 再登记后旧 as_token 立即 401 |
| 升级时清单的命名空间变宽 | 按 ADR-0036 的“权限变宽”走确认 |
| 卸载（保留或删除数据） | `!admin appservices unregister <id>`。实测后旧 token 立即 401，Tuwunel 丢弃待发的推送；影子用户和房间留下（第 7 节） |
| 核心启动 | 和 Tuwunel 对账：`appservices list` 加 `show-config`，核心记录里有、Tuwunel 没有或不一致的补登；Tuwunel 有、核心没记录的不动（可能是主人手工登记的） |

**失败即停的错误类别：**

| 情况 | 类别 |
| --- | --- |
| 清单写错；登记文件格式不对、命名空间写法认不出、比清单宽 | `invalid_input` |
| 命名空间、`sender_localpart`、`id` 撞上现有用户、别的插件或已有登记；Tuwunel 回 `Duplicate as_token` | `conflict` |
| 期限内没生成登记文件 | `unhealthy`，详情写“插件没有生成 appservice 登记文件” |
| Tuwunel 不通、进不了管理房间、等不到回复 | `matrix_unavailable` |
| Tuwunel 回了 `Command failed`（核对已过、不是重名） | 新类别 `matrix_rejected`，详情里放 Tuwunel 的原话，去掉 YAML。App 不认识时落到 `Other` |

### 4. 网络：两个方向都经核心

插件网络里没有 Tuwunel（`compose.yaml` 里它只在 `core` 网络），插件访问 Matrix 经核心网关的 `matrix.wonton.internal`。

- **桥 → Tuwunel**：照旧走 `matrix.wonton.internal`。网关认来源插件声明了 `matrix.appservice`、`Authorization` 里是**它自己登记的**
  as_token（核心按摘要查表，不调 whoami），才转发。`?user_id=` 冒充谁由 Tuwunel 按命名空间把关（实测越界回 `M_EXCLUSIVE`）。
  公开房间的规则（`room_rules`）和 pusher 只能指向推送代理的规则照旧；Bot 的发送限额和跳数（`send_limits`）不适用：影子用户转的是远端的人，
  补历史时会成批发。
- **Tuwunel → 桥**：登记里的 `url` 换成 `http://appservice.wonton.internal/<id>`。核心在 `core` 网络上开一个只给这个用的监听（同推送的
  `push.wonton.internal:80`），按 `<id>` 去掉前缀转给这个插件的 `service:port`。实测 Tuwunel 2.0 会在带路径前缀的 `url` 后接上
  `/_matrix/app/v1/...`。这个监听不在插件网络上，插件够不着。Tuwunel 推送时 `hs_token` 同时放在 `Authorization` 和查询串
  `access_token=` 里（实测），核心转发和日志都不能记查询串。
- **联邦代理要排除这个名字。** Tuwunel 的 appservice 客户端和联邦一样套 `[global.proxy]`
  （[client/mod.rs](https://github.com/matrix-construct/tuwunel/blob/v2.0.0/src/service/client/mod.rs) 的 `appservice_builder` 建在 `base()` 上，
  `base()` 套代理；只有 unix socket 的登记 `no_proxy`）。私用名模式下 `by_domain` 的 `include` 是空表，即所有域名都走联邦 CONNECT
  代理，只排除了 `push.wonton.internal`（`matrix/federation_config/routing.rs`）。`appservice.wonton.internal` 要加进 `exclude`。
  这条读代码得出，**未实测**。

### 5. 权限级别：高

appservice 能做的：

- 建任意多个命名空间里的用户，并以它们的身份发消息、建房间、拉主人进房间（实测影子用户建房邀请主人成功）。
- 收到命名空间里的用户所在房间的全部事件：桥建的房间，以及主人把影子用户或桥 bot 拉进的房间（比如拉进家庭群，桥就看得到家庭群）。
- 不能冒充命名空间外的人，看不到主人别的房间，碰不到宿主。

这些靠第 2 节的核对才成立。不核对时，实测一个写 `@.*:<服务器名>` 的登记照样被 Tuwunel 接受，能以主人身份 whoami，
还收到了管理房间里的事件，包括别的 appservice 登记命令里的 token。Tuwunel 自己不查命名空间重叠
（`admit_registration` 里写着 `TODO: Check for collisions between exclusive appservice namespaces`，实测两个登记同一段独占命名空间都成功）。

所以定为**高**（ADR-0036 的三级），和 `network.lan` 同级：醒目展示，Bot 来装走确认卡片（ADR-0033、ADR-0036 的“两条不变”第 2 条，
危险模式例外照 ADR-0091）。提示文字由核心给（中文，英文同义）：

> 这个插件会在你的服务器上建一批账号（名字以 telegram_ 开头），代表外部聊天里的人给你发消息、拉你进群。它能看到这些账号所在的
> 聊天里的全部消息；你把它们拉进别的群，它也能看到那个群。看不到你的其他聊天。

`telegram_` 由核心按清单填。远端账号被封、凭据存在插件里这类风险属于插件说明，不进权限提示。

**Agent 和 Bot 插件不能声明它。** 网关对 Bot 的约束（发送限额、跳数、按房间授权，ADR-0083）都挂在 Bot 身份上，appservice 的影子用户
一个都不受；带 shell 工具的 Agent 再拿到它，一次提示注入就能用影子用户绕过这些约束。桥不需要 Agent 能力，把两者分开没有损失。

### 6. 主人和桥 bot 的私聊：主人的客户端建

mautrix bridgev2 只在“有人邀请桥 bot 进一个两人房间”时把这个房间记为那个人的管理房间、发欢迎语，管理房间里命令不用前缀
（[matrixinvite.go](https://github.com/mautrix/go/blob/main/bridgev2/matrixinvite.go) 的 `handleBotInvite`、
[queue.go](https://github.com/mautrix/go/blob/main/bridgev2/queue.go)，2026-11-12 查）。所以邀请必须由主人本人发出。

- 核心在插件的 ViewModel 里给出桥 bot 的用户 ID 和 `encryption`。
- App 的 Rust 客户端核心看到一个有桥 bot、还没建过私聊的插件，就以主人身份建一个 `is_direct` 房间邀请它，`encryption: false` 时不开加密；
  建过的记在主人的 account data（`app.wonton.appservice_dm`，插件 id → 房间），主人退出私聊后不再自动建。插件详情页给“打开聊天”，
  没有就再建一个。
- 用户目录照旧不列 appservice 用户（`show_appservice_users_in_user_directory` 保持默认 `false`），入口靠这一步。
- 只给主人建。桥的 `permissions` 由插件按 `WONTON_APPSERVICE_OWNER` 渲染，第一版只给主人（见“待主人定”）。

### 7. 卸载以后

影子用户删不掉、桥建的房间留着：历史还在，房间不再更新。和 Bot 一样（`plugin/bots.rs`：不删数据卸载保留账号和房间）。
重装同一个插件接着用这些房间和影子用户。命名空间按第 2 节永久保留。

### 8. 端到端加密

- **mautrix 桥**：`encryption.allow`、`default`、`require` 默认都是 false，桥建的房间不加密。开端到桥加密要
  `encryption.msc4190: true`（appservice 代影子用户管设备，MSC4190），“不建议”的免 `/sync` 模式还要登记里的 `org.matrix.msc3202`
  （[mautrix 文档](https://docs.mau.fi/bridges/general/end-to-bridge-encryption.html)，2026-11-12 查）。
  客户端还要接受“影子用户的消息由桥 bot 的设备加密”（文档的 invisible crypto 一段）。
- **Tuwunel v2.0.0**：登记字段支持 `device_management`（MSC4190）和 `org.matrix.msc3202`（MSC3202 加 MSC4203 的 to-device 推送），
  见上面的 appservices.md。核心不改这两项，原样跟上游的登记文件走。
- **对主人的影响**：不加密时，桥房间和主人与桥 bot 的私聊在 Tuwunel 数据库里是明文，主人在私聊里发的登录验证码、两步验证密码也是。
  服务器是主人自己的，远端（Telegram 普通聊天、Discord、Slack）本来就不是端到端加密；Signal、WhatsApp 的消息到了桥这里也已经解开。
  加密只防“Tuwunel 的数据库被拿走”这一种情况。
- **我们的 App** 用 matrix-rust-sdk，没有改过解密的信任要求（仓库里搜不到 `TrustRequirement`），按默认值应当能解开桥 bot 设备加密的
  影子用户消息，界面会不会出警告**未核实**。

开不开见“待主人定”。

## 理由

- 管理房间命令热生效，核心装一个插件不用重启 Tuwunel；`appservice_dir` 每装卸一次就要重启，所有手机断线，升级时迁移数据库最长等 5 分钟（`stop_grace_period`）。
- 读上游生成的文件，插件作者照上游文档配就行，不用学我们的 token 接法，符合“Agent 能一次写对”。
- 核对放在核心一处，因为 Tuwunel 不查重叠、不查现有用户（实测）；核心本来就是分配用户名的唯一一方，能把查重做全。
- 两个方向都经核心，插件网络和 `core` 网络照旧隔开，网关对插件的认证（ADR-0023 的“只信任核心的地址”）不变。
- 契约是 Matrix Application Service API 和它的登记 YAML（开放标准）；提供方是核心加 Tuwunel；消费方是 mautrix 桥。换成别的
  appservice（Hookshot、heisenbridge）不用改核心。

## 没选的做法

- **`appservice_dir` 加重启 Tuwunel。** 见“理由”；核心还要能写 Tuwunel 挂载的目录，而那个目录现在只读挂载。
- **写进 `tuwunel.toml`。** 同样要重启，而且管理命令删不掉。
- **核心生成 token、自己拼登记文件**（ADR-0064 的 `generate`）。核心对 token 心里有数，不用读插件写的文件；但每个插件都要把 token 接进
  上游配置，上游入口照样要求 `/data/registration.yaml` 存在，还得自己生成一份占位。上游的 `-g` 流程最省事，读文件的风险用第 2 节的
  不跟链接、核对、改写挡住。
- **原样登记插件的文件。** 实测一个 `@.*` 的登记能冒充主人、收到管理房间的消息。
- **把 Tuwunel 接进插件网络，或用 unix socket 共享目录。** 前者让插件绕过网关直连 Tuwunel；后者要把插件可写的目录挂进系统容器，还要两边同一个用户。
- **核心冒充桥 bot 建私聊。** 房间建得出来，但桥不会把它当管理房间（第 6 节），命令要带前缀，还是核心在替桥做事。
- **核心用 JWT 以主人身份建私聊。** 会给主人凭空多一台设备，核心也不该持有主人的会话。
- **当成 `matrix.bot` 的扩展。** Bot 是一个身份、一台核心管的设备；appservice 是一段命名空间、自己管设备，网关规则完全不同。
- **双重傀儡（double puppeting）一起做。** mautrix 现在只支持“另登记一个 `url: null`、非独占 `@.*` 的 appservice，把它的 as_token 给桥”
  （[mautrix 文档](https://docs.mau.fi/bridges/general/double-puppeting.html)）。那等于把主人的账号交给每个桥，级别和本篇不是一回事，单独定。

## 代价与风险

- **名字被占。** 一个插件占了 `telegram_` 以后，同名前缀的另一个插件装不上，家人也不能叫 `telegram_x`，卸载后也不释放。
- **管理房间是新依赖。** 回复的文字格式不是 API（`Command failed with error:` 是 Tuwunel 的字符串），Tuwunel 升级改了就要跟着改；
  核心升级 Tuwunel 时要跑第 4 个 PR 的 Docker 测试。
- **影子用户不受 Bot 限额。** 桥坏了刷屏，核心不拦，只能停插件。
- **桥只能占独占命名空间。** 需要非独占命名空间的 appservice（双重傀儡、一些监控类）装不了。

## 待验证

- 联邦代理排除 `appservice.wonton.internal` 后，私用名模式下 Tuwunel 能直接推送到核心（第 4 节，读代码推断）。
- 真实 mautrix 镜像（signal 或 telegram）在核心登记前的行为：生成文件后退出、重启后 whoami 失败时是退出还是重试。
- 撤回管理房间里的命令后，Tuwunel 是否立即删掉原文。
- rootless Docker 下读插件数据目录里 0600 文件。
- 开端到桥加密时，App 对影子用户消息的显示。

## 验证结果

2026-11-12，本机 OrbStack（Docker 29.8.2），`ghcr.io/matrix-construct/tuwunel:v2.0.0@sha256:3dde1e9b…`，服务器名 `test.local`，
另起一个 Python 小服务当 appservice。容器、网络和镜像测完已删。

| 测了什么 | 结果 |
| --- | --- |
| Synapse 兼容接口注册 `wonton-core`（`admin: true`），解析 `#admins`、加入 | 成功；房间没有 `m.room.encryption`，成员只有 `@conduit` 和 `@wonton-core` |
| `!admin appservices register` | `m.notice` 回复 `Appservice registered with ID: tg`，`m.relates_to` 指向命令 |
| 两个登记用同一段独占命名空间 `^@telegram_.*` | 都成功 |
| 同一个 as_token 登记两次 | 失败：`Duplicate as_token: tg tg3` |
| 命名空间 `^@.*:test\.local$` | 登记成功；`whoami?user_id=@owner` 返回主人；管理房间的回复被推送给了它 |
| 独占命名空间盖住登记前就存在的 `@telegram_alice` | as_token 能冒充她 |
| 命名空间外 `?user_id=@owner` | 400 `M_EXCLUSIVE` |
| 坏 YAML | `Command failed with error: … Could not parse appservice config as YAML …` |
| 重启 Tuwunel | 登记还在，token 照常可用 |
| 同 id 换 token 再登记 | 旧 as_token 立即 401，新的可用 |
| `unregister` | token 立即 401；影子用户还在；不存在的 id 回 `Appservice not found` |
| `url` 带路径前缀 `http://bridge:29333/_wonton/appservice/wonton.telegram` | 推送到 `<前缀>/_matrix/app/v1/transactions/…`，`ping` 通 |
| appservice 注册命名空间外的用户名 | 400 `M_USER_IN_USE` |

## 待主人定

见 PR 描述，定了以后改写成“已定”。

## 实施顺序

1. 清单字段、权限级别和提示、协议字段（只改接口，先合；同步改 wonton-docs 的插件规范）。
2. 核心登记：读文件、核对、保留命名空间、管理房间命令、生命周期和对账。
3. 网络：`appservice.wonton.internal` 中继、联邦代理排除、网关认 as_token。
4. App 建私聊，加真实桥镜像的冒烟。

## 关联

[ADR-0003](0003-matrix-and-encrypted-push.md)、[ADR-0007](0007-agents-and-bots.md)、[ADR-0014](0014-reserved-capabilities.md)、
[ADR-0015](0015-small-components-standard-interfaces.md)、[ADR-0023](0023-egress-proxy.md)、[ADR-0024](0024-confirmation-scope.md)、
[ADR-0033](0033-plugin-tabs.md)、[ADR-0036](0036-plugin-privileges.md)、[ADR-0045](0045-family-members.md)、[ADR-0050](0050-cross-server-chat.md)、
[ADR-0064](0064-input-secrets.md)、[ADR-0066](0066-plugin-forms.md)、[ADR-0083](0083-bot-contacts-and-group-chat.md)、[ADR-0091](0091-danger-mode.md)
