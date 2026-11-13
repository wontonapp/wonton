# ADR-0009 身份、配对与单点登录

- 状态：已接受（2026-10-28），M0 选型已定（见验证结果）；加设备和找回的做法 2026-10-29 改定（见决策和理由）；小程序单点登录的默认做法和主人用户名的规则由 ADR-0022 修订；一个 App 连多台服务器的做法由 ADR-0042 定；家人的账号、设备和配对码由 [ADR-0045](0045-family-members.md) 定；Matrix 的服务器名 2026-11-06 改成每台唯一（见“补充：Matrix 的服务器名”），默认改用 ADR-0021 的家名、对整个 Matrix 开放联邦见 [ADR-0051](0051-friends-and-plugin-sharing.md)；插件后端访问签发方的地址由 ADR-0063 取代

## 背景

旧方案的账号和设备绑定由一个云账号中心负责。新产品不接任何云账号。

## 决策

- **服务器即账号**：安装后，服务器上还没有配对的设备时，局域网状态页和安装终端显示首次配对码（服务器 endpoint ID、
  中继和直连地址、一次性密钥）。App 扫码即成为主人，不需要注册。第一台设备配上后首次配对码作废，不再出现。
- **设备身份**：每个 App 安装生成密钥对；核心记录已配对设备，主人可以随时吊销。隧道只接受已配对设备。
- **Matrix 登录**：核心为配对设备和 Bot 签发 Tuwunel 登录（Tuwunel 支持由运营方签发 JWT 登录）。
- **家人**：主人可以邀请家人（M2），每人一个 Matrix 用户和私人空间；一个 App 可以连多台服务器，做法见 [ADR-0042](0042-multiple-servers.md)。
- **小程序单点登录**：所有插件网页都经核心网关鉴权；核心提供 OIDC 单点登录和网关代登录，新插件默认用应用自身的
  登录（ADR-0022）。
- **添加设备**：在已配对的 App 里点“添加设备”，App 向核心要一个一次性配对码，显示给新设备扫。
- **找回**：设备都丢了时，在服务器终端执行 `sudo wonton pair` 出一个配对码。
- **信任边界**：服务器终端和已配对的设备。能在服务器上执行命令、或者手里有已配对的设备，才能加设备；能接触局域网
  不算可信。

## 理由

不依赖任何云账号，也就没有账号数据要保护；配对体验和“扫码加好友”一样简单。

局域网里不只有主人：访客的手机、智能家电、被攻破的设备都在同一个网段，网页还能借 DNS 重绑定从浏览器里读局域网的
页面。所以状态页只在装好到第一台设备配上之间出码，并且只回应 `Host` 是 IP 地址的请求；之后加设备要经过服务器终端
或已配对的设备。

## 待验证（M0 选型）

- OIDC 提供方：Tuwunel 内置的 OIDC 服务器能否同时服务插件；否则选 Pocket ID（BSD-2-Clause）或
  Rauthy（Rust，Apache-2.0）。
- JWT 登录签发流程与过期、吊销。

## 验证结果（M0-2）

JWT 登录可行，按下面的方案实现。验证版本：Tuwunel v1.9.4、matrix-sdk 0.19.1。可复跑：
`cargo test -p wonton-matrix --test tuwunel -- --include-ignored login`；签发逻辑见
`crates/wonton-matrix/src/testkit/issuer.rs`，Tuwunel 配置见 `deploy/dev/matrix/tuwunel.toml`。

**签发**

- 核心持有一把只用于 Matrix 登录的 Ed25519 密钥。Tuwunel 的 `[global.jwt]` 配 `format = "EDDSA"`、
  `algorithm = "EdDSA"`，`key` 只放公钥，Tuwunel 自己签不出令牌。
- 声明：`sub` 是用户名（localpart，Tuwunel 拼上 `server_name` 得到 MXID），`iss = "wonton-core"`，
  `aud = "tuwunel"`，`iat`，`exp = iat + 120 秒`。Tuwunel 配 `require_exp = true` 和对应的 `issuer`、
  `audience`；过期、缺 `exp`、签发方或接收方不符、别的密钥签的令牌都被拒（`M_FORBIDDEN`）。
- 配对设备和 Bot 走同一流程：核心签发令牌，设备或 Bot 自己调用 `POST /_matrix/client/v3/login`
  （`type = org.matrix.login.jwt`），首次登录自动建号。服务器关闭注册和密码登录，没有别的登录方式。
  设备自己登录，设备 ID、显示名和刷新令牌都归设备，核心不用保存设备的访问令牌。

**过期**

- 登录 JWT 只用来换一次访问令牌。Tuwunel 校验 `exp` 有 60 秒的时钟偏差容忍，实际在 `exp` 后 60 秒才失效。
- Tuwunel 不检查 `jti`：有效期内同一个令牌可以重复登录，每次都新建设备。所以有效期要短，令牌只经已配对
  设备的隧道下发，用完即弃。
- 访问令牌：登录时申请刷新令牌（Tuwunel 支持），访问令牌按 `access_token_ttl`（默认 7 天）过期，SDK
  自动换新并发出事件，客户端核心据此保存新令牌。刷新令牌默认不过期（`refresh_token_ttl = 0`），需要时
  可以设空闲期限或绝对期限。

**吊销**

- 核心用 `registration_shared_secret` 调 Synapse 兼容的 `POST /_synapse/admin/v1/register`（`admin: true`）
  注册自己的管理账号，访问令牌只存在核心密钥库，再用 Tuwunel 实现的 Synapse 管理接口：
  - 吊销设备：`DELETE /_synapse/admin/v2/users/{user}/devices/{device}`，访问令牌和刷新令牌一起作废；
    该设备进行中的同步几毫秒内失败，同账号的其他设备不受影响。重新配对就是再签发一次登录。
  - 移除 Bot 或家人：`POST /_synapse/admin/v1/deactivate/{user}`，现有会话失效，之后有效的 JWT 也会
    被拒（`M_USER_DEACTIVATED`）。
  - 查设备：`GET /_synapse/admin/v2/users/{user}/devices`。
- 客户端判断“被吊销”看 SDK 的令牌失效事件（`soft_logout = false`），不看错误码：令牌失效后 SDK 会先用
  刷新令牌重试，Tuwunel 对已作废的刷新令牌返回 403 `M_FORBIDDEN`，而规范要求 401 `M_UNKNOWN_TOKEN`。
- 还没用掉的登录 JWT 不能单独吊销，只能等它过期，或者轮换签发密钥：`[global.jwt]` 可热加载，核心改写
  配置文件里的 `key` 后给 Tuwunel 发 `SIGUSR1`，旧密钥签的令牌立即失效，不用重启。因此生产环境的公钥
  写在核心生成的配置文件里，不放环境变量（运行中的容器改不了环境变量）。

## 验证结果（M0-6）

小程序单点登录可行，OIDC 提供方选 **Ory Hydra**（Apache-2.0）。可复跑：`deploy/dev/core/smoke.sh`
（Grafana 示例插件打开即以主人身份登录，角色按 `groups` 声明映射为管理员）；本机 Chromium 实测结果
相同，Grafana 的 WebSocket 经网关可用。

**选型**

要求：核心已经知道用户是谁（设备会话），提供方要接受核心给出的身份，不能再弹自己的登录页；能经 API
登记和删除客户端（插件装卸时）；ID 令牌带 `email`、`name` 等标准声明；有 amd64 和 arm64 镜像；占用小。

| 候选 | 实验结果 |
| --- | --- |
| Tuwunel v1.9.4 内置 OIDC | 不合适。它服务 Matrix 客户端的新认证方式：给普通插件发的访问令牌就是完整的 Matrix 令牌（实测能调 `whoami`、`joined_rooms`），每次登录还新建一台 Matrix 设备；ID 令牌只有 `sub` 等基本声明，没有 `email`、`name`；登录页要密码；动态注册只发公开客户端，强制 PKCE，http 回调地址被拒。 |
| Pocket ID v2.19.1 | 能做到，但要依赖它的内部实现。只有通行密钥登录；管理 API 能给用户签一次性登录码，客户端可以跳过授权确认，浏览器拿到会话 Cookie 后授权直接返回 code（实测）。但网关得拦截它的前端路由 `/interaction` 并替浏览器兑换登录码；会话 Cookie 带 `Secure`，局域网 http 下不可用；还要同步一份用户库。空载 18 MiB。 |
| Rauthy 0.37.2 | 不合适。登录只能用密码或通行密钥，没有把身份交给外部的接口；单机也要配 Hiqlite（Raft）、加密密钥、WebAuthn 和 SMTP；空载 88 MiB。 |
| Dex v2.46.1（authproxy 连接器） | 可行：回调路径读反向代理注入的用户头。但客户端只能写在静态配置里（改了要重启）或走 gRPC API。空载 11 MiB。 |
| Ory Hydra v26.2.0 | 采用。它本身就是不带界面的 OAuth 2.0 / OIDC 服务器，登录和授权确认交给核心的两个端点，核心经 REST 管理接口接受；客户端经 REST 登记和删除；声明由核心逐次给出。镜像约 20 MB（压缩），空载 16–22 MiB，SQLite 存储，迁移 1.6 秒。 |

**流程**（`crates/wonton-core/src/gateway/`、`oidc.rs`）

1. 核心会话在核心域名 `<base>` 上（M0 用主人令牌登录，配对上线后换成设备身份）。每个插件子域名
   有自己的插件会话：浏览器先到核心换一个 60 秒内有效、只能用一次、绑定目标域名的交接码，再到插件域名
   兑换。插件之间、插件和核心之间不共享 Cookie，网关的 Cookie 不会转发给插件。
2. 安装 `sso: oidc` 的插件时，核心在 Hydra 登记客户端（ID 为插件 `id`，随机密钥，回调为插件网页地址
   加 `redirect_paths`），同时登记插件的登出回跳地址（默认与额外路径见[插件规范](../plugin-spec.md)），
   把签发方和各端点地址注入插件；卸载时删除客户端。核心启动恢复插件时按保存的清单和原密钥重新登记。
3. 应用把浏览器带到 `auth.<base>`（网关转发给 Hydra 的公开端点）→ Hydra 跳到核心的登录端点 →
   核心按会话接受登录（`subject` 是用户 ID）→ 授权确认端点只发放客户端可申请的 scope，并按 scope 附上
   声明（`name`、`preferred_username`、`email`、`groups`）→ 回到应用。Hydra 不记住会话，每次授权都
   回到核心检查。实测从未登录到进入 Grafana 首页共 12 次跳转。`groups` 默认只有 `owner`；清单的
   `webui.oidc.owner_groups` 给单个客户端加组名（应用按组名对角色，核心不认识应用的角色）。邮箱声明及应用发信
   遵循 [ADR-0056](0056-internal-mailbox.md)。
4. 插件后端在自己的网络里访问签发方和令牌、用户信息端点，地址见 ADR-0063（已取代原来的 `auth.<base>` 别名）。
   Hydra 只在核心的私有网络里，插件访问不到它的管理端口（冒烟脚本验证）。
5. 应用走 OIDC end-session 登出时，带回跳地址的请求只允许登记过的地址；没有回跳参数时由 Hydra 的
   `urls.post_logout_redirect` 送回核心首页，不停在身份提供方自己的默认页。

**SQLite 并发（2026-11-13 验证）**

Hydra 使用官方 DSN 参数 `max_conns=1&lock=false`：由 `database/sql` 的单连接队列串行执行数据库操作，
不叠加 Pop 的语句锁。Pop 的事务锁和语句锁分别持有时，非事务写入可能持语句锁等待 SQLite 写锁，
挡住已有事务继续执行和提交；实测四条并发授权链会报 `database is locked`，等待超过 Hydra 的写响应期限后，
网关收到连接中断。只增加等待期限或重放 HTTP 请求不能解决锁竞争，还会重复消费一次性 verifier/code。
安装和开发 Compose 使用同一组连接参数；不改上游镜像、HTTP 期限或非幂等请求的重放边界。

`deploy/dev/core/smoke.sh` 的 `sso` 步骤覆盖四个独立会话的 PKCE 授权链，在授权确认结束和首次 Basic token 兑换前
各设一道屏障，断言每个授权码只提交一次且 userinfo 认出主人。修改前的配置出现 502 和已消费 verifier，修改后全部成功。


**遗留**

- App 内 WebView 经隧道打开：见下一节。WebView 的 HTTP 代理设为客户端核心的本地代理，`Host` 原样保留，
  插件地址和签发方地址在手机上和服务器上一致。
- 网关和 Hydra 只跑 http。App 经隧道打开插件网页不再加 HTTPS，iroh 已经加密（主人 2026-11-04 定）。第三方 App 的 HTTPS 见 ADR-0021。
- 核心会话在内存里，核心重启后要重新登录。
- 家人账号（M2）：`subject` 按用户区分，`groups` 给出家庭角色。

## 验证结果（#21，网关接受隧道身份）

核心内置隧道入口（`crates/wonton-core/src/tunnel.rs`）：iroh endpoint 和隧道服务端跑在核心进程里，每条隧道
流直接交给网关的 HTTP 服务，QUIC 握手认证过的设备 endpoint ID 放进请求扩展（`TunnelPeer`）。局域网的 TCP
请求里没有这个扩展，请求头也伪造不了它。可复跑：`cargo test -p wonton-core gateway::tunnel_tests`（真实
隧道，只走回环地址），以及 `deploy/dev/core/smoke.sh` 的“已配对设备经隧道打开插件”一步。

- **设备登记**：这组验证里主人用令牌经管理 API（`POST /_wonton/api/devices`）登记设备，扫码配对见下面的
  “接口：扫码配对”。
  登记表存在 `core/devices.json`，设备记着归属人：主人的设备不写，家人的设备登记在家人名下（ADR-0045）。吊销（`dev.sh revoke`）立即断开该设备的隧道连接；
  网关对每个请求再查一次登记表，吊销和断开之间漏进来的请求也不放行。
- **放行范围**：插件网页、核心首页、OIDC 登录和授权确认端点接受隧道身份。经隧道的已配对设备不需要核心
  会话，也不需要插件会话交接；Hydra 回到核心的登录端点时，核心直接按设备身份接受登录。冒烟实测：
  - 未登记的设备在隧道层就被拒（本地代理回 403 `unauthorized`），请求到不了网关。
  - 登记后打开 Grafana，经过 9 次跳转（只有 OIDC）直接进入首页，用户是 `owner`；浏览器登录要 12 次。
  - 全程网关不发 Cookie。吊销后的下一个请求立即被拒。
- **管理 API 要防网页借隧道身份**：隧道身份和 Cookie 一样随请求自动带上，App 里任何插件网页都能借它向核心
  发跨站请求（例如用 `<form>` 提交安装插件）。所以设备管理 API 只认 `Authorization: Bearer`；插件管理 API 也接受
  已配对设备，但拒绝网页发起的请求（ADR-0017 的“信任和权限”）。
- **前提**：网关信任隧道身份，前提是设备上只有本 App 用得了它的本地代理。本地代理监听回环地址，同机其他
  App 也能连上（ADR-0004 的结论），所以设备一侧的本地代理必须校验调用方，见 ADR-0004 的验证结果（#21）。
- **App 内打开**：iOS 模拟器的 WKWebView 和 Android 模拟器的 WebView 经本地代理打开
  `http://app-grafana.wonton.localhost:18480/`，都直接进入 Grafana，用户是 `owner`，没有看到任何登录页。
  做法和遇到的问题见 ADR-0004 的验证结果（#21）。


## 接口：App 登录和使用 Matrix（M1）

App（客户端核心）经隧道登录和使用 Tuwunel，不需要知道 Tuwunel 在哪里。两个入口都只给经隧道接入的已配对设备
（网关看隧道身份，局域网请求不行）。设备管理 API 只认主人令牌，只有发配对码的接口也接受已配对设备（见“接口：扫码
配对”的“主人发的码”）。实现见
`crates/wonton-core/src/gateway/matrix.rs` 和 `crates/wonton-core/src/matrix/`。

**1. 取登录令牌：`POST <核心地址>/_wonton/matrix/login-token`**

- 没有请求体，鉴权只看隧道身份。核心域名上的跨站修改拦截对它生效，响应也不带 CORS 头：插件网页能让 WebView
  带着隧道身份发请求，但拿不到令牌。
- 响应 200（JSON）：

  | 字段 | 说明 |
  | --- | --- |
  | `homeserver_url` | Matrix 客户端接口的地址：`<scheme>://matrix.<基础域名>[:端口]`，经隧道访问，`Host` 原样保留 |
  | `server_name` | Tuwunel 的 `server_name`，即用户 ID 冒号后的部分 |
  | `user_id` | 设备所属的用户，如 `@owner:wonton.localhost`。主人的设备是主人，家人的设备是这个家人（ADR-0045） |
  | `login_type` | `org.matrix.login.jwt` |
  | `token` | 一次性的登录 JWT，声明见上面的验证结果（M0-2） |
  | `expires_in` | 令牌还能用多少秒（120） |

- 错误：`{"error": {"kind": …, "message": …}}`，`X-Wonton-Error` 头同 `kind`。401 `unauthenticated`（不是经隧道的
  已配对设备）；403 `cross_site`（其他网页发来的跨站请求）；503 `matrix_unavailable`（核心没有接入 Tuwunel，或
  Tuwunel 不可用）。

**2. 登录**：在 `expires_in` 秒内调 `POST {homeserver_url}/_matrix/client/v3/login`，请求体
`{"type": login_type, "token": token, "refresh_token": true, "initial_device_display_name": "…"}`。设备 ID、访问令牌和
刷新令牌都归设备，核心不保存；令牌被吊销或丢失后再取一次登录令牌。

**3. Matrix 客户端接口：`matrix.<基础域名>`**

- 网关把 `/_matrix/client/` 和 `/_matrix/media/` 下的请求转发给 Tuwunel；联邦接口（`/_matrix/federation/`、
  `/_matrix/key/`）和管理接口（`/_synapse/admin/`）不转，得到 404 `M_UNRECOGNIZED`（`X-Wonton-Error: not_found`）。
- 先于前缀判断，挡住上游可能规范化成前缀以外接口的写法：路径按字面的 `/` 切段，百分号解码后是 `.` 或 `..` 的段
  （包括 `%2e%2e`、`.%2e`），以及反斜杠，得到 400 `M_UNRECOGNIZED`（`invalid_input`）。编码过的 `/`、`\`
  （`%2F`、`%5C`）放行：ruma 把路径参数里的 `/`、`\` 编成它们，Megolm 会话 ID 是标准 base64、大约一半带 `/`，按会话
  取密钥备份（`GET /_matrix/client/v3/room_keys/keys/{房间}/{会话}`）要用到。实测 Tuwunel v1.9.4 对 `%2F`、`%5C`、
  `..`、`%2e%2e` 都不规范化，绕不到管理接口，这一层是纵深防御。
- 只接受经隧道的已配对设备，局域网请求得到 403 `M_FORBIDDEN`（`unauthenticated`）；Tuwunel 不可用时 503
  `M_UNKNOWN`（`matrix_unavailable`）。错误都用 Matrix 的格式（`errcode`、`error`），另带 `X-Wonton-Error`；网关
  自己不回 401，免得 SDK 以为访问令牌失效。
- 客户端核心的本地代理只转发基础域名和它的子域名（ADR-0004），`matrix.<基础域名>` 在其中。

核心没有配置 Matrix 时，两个入口都返回 503 `matrix_unavailable`。

## 接口：扫码配对（M1）

App 扫一个一次性的配对码，成为主人的设备。线协议的类型和契约测试在 `crates/wonton-tunnel/src/pair.rs`，App 一侧的
操作和状态是 `Action::PairHome`、`HomeViewModel::pairing`（`crates/wonton-protocol`）。

**配对码**：二维码里是一个地址 `wonton://pair?v=0&id=…&relays=…&addr=…&domain=…&core=…&secret=…`。

| 参数 | 说明 |
| --- | --- |
| `v` | 格式版本，不兼容地变化时加一。App 认不出的版本提示更新 App，不当成坏码 |
| `id` | 服务端的 endpoint ID |
| `relays`、`addr` | 中继表、直连地址，合起来至少一个：n0 的地址发现在国内靠不住（ADR-0004）。`relays` 是 JSON 数组，每项 `{url, token?, tier}`（和 `/_wonton/api/tunnel` 的同一个序列化），令牌跟着表走（ADR-0048） |
| `domain` | 基础域名。本地代理只转发它和它的子域名 |
| `core` | 核心自己的地址，经隧道访问，主机名就是基础域名 |
| `secret` | 一次性的配对密钥：128 位随机数，32 位十六进制小写 |

认不出的参数忽略，同一个版本里可以往码里加可选的字段。

iOS 收到系统打开的 `wonton://pair?…` 链接时，只在未配对且不在添加服务器、也没有正在进行的配对时打开配对页，
预填配对码并展示码里的基础域名。链接本身不发送配对请求；用户可以改设备名，点“配对”才发 `PairHome`。
格式不对的码也先预填，确认后把核心拒绝的具体原因显示在配对页。已配对、添加服务器和配对进行中不接收外来配对链接。

**配对**

1. 还没登记的设备用 ALPN `wonton/pair/0` 连上服务端（和隧道的 `wonton/tunnel/0` 在同一个 endpoint 上），开一条双向流，
   写入请求后结束发送：`{"type": "pair", "secret": "…", "device_name": "…"}`。`device_name` 是主人在设备列表里认设备用的名字，
   App 配对时填，去掉首尾空白后为空就报 `InvalidInput`，不登记一台没有名字的设备。配对成功后它随连接信息存在设备本机，
   Matrix 登录和推送登记拿它当设备显示名，确认弹窗里看到的也是它。
2. 服务端回一个 JSON 后结束：`{"result": "paired"}`，或者 `{"result": "rejected", "reason": "…", "message": "…"}`。请求和
   回应都不超过 4096 字节，超过就断开。
3. 登记的是 QUIC 握手认证过的设备 endpoint ID，请求里没有身份字段。成功后设备改用隧道的 ALPN 连接，之后和已登记的设备
   一样。
4. 同一台设备、同一个码重发是幂等的：码记着用它的设备，只要这台设备还在登记表里，重发直接再回
   `{"result": "paired"}`，不重复登记、不换身份。别的设备拿用过的码来、或者这台设备已经被吊销，回 `used`。
   回应可能在慢中继上没赶上，客户端自己也按下面的限时先确认一次。

| `reason` | 含义 | App 的 `PairingFailure` |
| --- | --- | --- |
| `unknown_code` | 服务端没发过这个码 | `Rejected` |
| `used` | 码已经被别的设备用掉，或者用过它的设备已被吊销 | `Used` |
| `expired` | 码过期了 | `Expired` |
| `closed` | 这种码不再能用来配对，例如已经有配对设备时的首次配对码 | `Closed` |
| `bad_request` | 请求格式不对或太长 | `Protocol` |
| `server_error` | 服务端自己出错了，例如设备登记表写不进去 | `Upstream` |
| 认不出的值 | 比 App 新的服务端给的原因 | `Rejected`，显示 `message` |

连不上服务端是 `TransportUnavailable`；服务端不接受 `wonton/pair/0`（版本不配）是 `Protocol`。扫到的文字不是配对码时，
`PairHome` 直接返回 `InvalidInput`。

配对的时间按段分开：绑定 iroh endpoint 最多 10 秒、卡住就换一次新的绑定（最多两次，见
[ADR-0032](0032-connection-quality.md) 的“绑定不放宽”）；一次交换里建连、开流、写完请求、等服务端回应各最多 30 秒。
以前整段交换只有一个 30 秒的时限：慢链路上握手和开流就吃掉大半，请求交出去了、回应还在路上就到点，服务端已经
登记、设备这边报失败，两边对“配上没有”看法相反（半配对）。分开以后一段慢不会占掉别的段的预算。

**开流之后超时、或者连接先被对端关掉，一定再确认一次**：服务端对还在登记表里的同一台设备重发时回 `paired`，
算配上；确认拿到 `used`（设备已被吊销、或码本来就是别的设备用掉的）就和别的拒绝一样报失败，不留一个登记表里没有
它的连接信息。以前只在“请求整个写完”之后才确认，开流那一段超时就漏掉了。读回应时连接先被对端关掉（服务端回完
就关、`LINGER` 到点也关）同样分不清，按“可能已经配上”再问一次。确认也超时或同样分不清才报失败，拿同一个码手动
重试还能配上。

**建连那一段超时不确认**，直接报连不上：没有 0-RTT，握手在客户端没完成之前请求一个字节都发不出去，服务端不可能
已经登记；再问一次只会让死服务端的失败从 30 秒拖到 60 秒。

配对页当场拒绝存不下的码：码里的地址和域名要能变成一份合法的连接信息，否则配上以后存不下，码已经用掉，只能重新
发码再扫一遍。

失败立即结束“配对中”，不等 QUIC endpoint 清理完成才发布结果。每一段都在客户端日志里留一行（连接中、绑定花了多久、
哪一段超时、为什么要再确认、最终的失败和详情），以前失败只写进界面，`adb logcat` 和核心日志里一条都没有。
识别到二维码时，两端立即给一次系统触感反馈，停止本次扫描，在取景框的位置显示连接进度；触感表示读到了码，不表示配对成功。
iOS 用成功通知触感，Android 11 起用确认触感、Android 10 用虚拟按键触感，尊重系统触感设置，不播放提示音。
好友和服务器互扫共用的相机组件默认仍连续扫描，只有设备配对启用单次识别。
两端按错误类别显示失败页，替换扫码和粘贴表单；过期、已用过、连不上服务器分开说。失败页提供“重新扫描”，原地清掉本次
扫描结果并重新打开相机，不退出添加服务器；可重试的失败仍能用同一个码再试，返回不算配对成功。试过的直连地址、
中继、耗时、限时和底层原因由客户端核心按 App 语言写进默认收起的详情，不包含配对密钥。

**码的有效期**：一次性，用过的码只有用它的那台设备、并且它还在登记表里时才能重发（码记在核心内存里，核心重启后连它
一起丢），10 分钟后过期。码只在核心的内存里，核心重启后全部作废。

一次性码兼作重发的凭据，就必须对同一台设备幂等：慢中继上服务端可能已经登记成功、回应却没赶上，客户端按上面的
限时超时后拿同一个码重试，旧的做法撞上 `used` 报失败——服务器有这台设备，App 没有连接信息，两边对“配上没有”看法
相反，主人这台设备停在未配置、家人那台报“服务器不认这台设备”。没选“客户端超时后先连一次隧道确认”：那要另建一整条
隧道再发请求，比再发一次配对请求慢得多，还要为没配上的情况回滚本机状态；没选“码可以反复用”：两台设备同时扫同一个
码会都配上。

**状态页**：局域网浏览器用 IP 地址打开核心的端口（`http://<服务器 IP>:<端口>/`），看到运行状态和可用的配对码。只回应
`Host` 是 IP 地址的请求：别的网站把自己的域名解析到局域网 IP（DNS 重绑定）后，从浏览器里读不到配对码。

**主人发的码**：`POST /_wonton/api/pairing-codes`，请求体可选 `{"direct_addrs": ["IP:端口"]}`（写进码里的直连地址；
不给时用核心算的直连地址，跟着宣告地址和局域网检测走，ADR-0019），回应
`{"code": "wonton://pair?…", "expires_in": 600}`，已经有设备时也能用。认两种身份：主人令牌（服务器上的命令、开发和冒烟），
以及隧道认证过的已配对设备（App 里的“添加设备”）。

**添加设备**：已配对的 App 发 `Action::InviteDevice`，客户端核心经隧道调上面的接口，不带直连地址（由核心给），拿到的码
放进 `HomeViewModel::invite`（`Ready`，带过期时间），App 画成二维码给新设备扫，
新设备照常用 `PairHome`。还没配对时 `InviteDevice` 返回 `NotReady`；连不上、本设备被吊销、服务器出错分别是 `Failed` 里
`HomeFailure` 的 `TransportUnavailable`、`Unauthorized`、`UpstreamUnavailable`，回应认不出是 `Protocol`。

## 验证结果（M1，扫码配对）

接口按上一节实现：核心发码、验码和登记设备在 `crates/wonton-core/src/pairing.rs`，局域网状态页在
`crates/wonton-core/src/gateway/lan.rs`，`wonton/pair/0` 的两端在 `crates/wonton-tunnel/src/pair/`，App 一侧在
`crates/wonton-client/src/home/pairing.rs`。可复跑：`cargo test -p wonton-core pairing`（经真实的隧道入口）、
`cargo test -p wonton-client --test it pairing::`、`cargo test -p wonton-tunnel --test tunnel pairing`，以及
`deploy/dev/core/smoke.sh` 的“已配对设备经隧道打开插件”一步。

- **首次配对码**：服务器上还没有设备时，状态页给一个码，还剩不到 2 分钟时换新码，旧码到期前仍然有效。第一台设备配上
  后状态页只显示运行状态；没用掉的首次配对码随之作废（`closed`），两台设备同时扫也只有一台配上：判断和登记在设备
  登记表的同一把锁里。
- **重发的幂等**（#159）：配上过的码只有那台设备在登记表里时再发才回 `paired`，别的设备、或者吊销以后的同一台
  设备都回 `used`，家人的码同理；记号在内存里，核心重启后重发回 `unknown_code`。客户端在请求发完、回应没赶上的
  超时后自己再确认一次；两次都超时才报失败，拿同一个码手动重试还能配上。可复跑：`cargo test -p wonton-core pairing`（`cargo test -p wonton-client
  --test it pairing::` 里的 `a_reply_after_the_deadline_still_pairs`、`retrying_the_same_code_after_a_failure_pairs` 等）。
  验证时是回环上的假服务端，没有在真实中继上跑。
- **主人发的码**：管理 API（`POST /_wonton/api/pairing-codes`，主人令牌）和服务器上的 `wonton-core pair` 发的码，已经
  有设备时也能用。
- **安装终端**：`wonton-core pair --first`（管理 API `POST /_wonton/api/pairing-codes/first`）显示和状态页上同一个首次
  配对码，换码时跟着换，第一台设备配上后退出；已经有设备时 API 回 409 `paired`，命令只说明怎么加设备。
- **添加设备**：已配对的设备经隧道要码不需要主人令牌，局域网里不带令牌仍是 401
  （`cargo test -p wonton-core a_paired_phone_gets_a_code`）。App 一侧 `InviteDevice` 拿到的码让另一台设备配上；要码被拒时
  403、5xx、认不出的回应分别报 `Unauthorized`、`UpstreamUnavailable`、`Protocol`（`cargo test -p wonton-client --test it pairing::`）。
- **状态页**：码里的直连地址是浏览器打开状态页用的 IP 加隧道端口，同一个局域网里的手机也连得上；`Host` 是域名的请求
  （DNS 重绑定）得到 404。页面只列出各组件是否就绪和插件数量，不列主人装的插件名；随核心附带的插件没在运行时逐个
  写明状态（插件规范的“预装插件”）。
- **App**：扫到的码先在本机解析，不是配对码直接拒绝；配上后存下码里的服务端地址（不存密钥），重启后直接连上。服务端
  不认识 `wonton/pair/0` 时 TLS 握手以 `no_application_protocol` 失败，App 报协议版本不配。
- **设备登记表**：每台设备多记配对时报的名字和登记时间；只有 endpoint ID 的旧登记表照样读得进来。
- **App 的页面**（iOS 26.5 模拟器、Android 模拟器 API 36，连 OrbStack 虚拟机里一行安装的服务器，2026-10-29）：没配对时
  “家”就是配对页。iOS 模拟器不能扫码，粘贴安装终端上的首次配对码配上，随后经隧道登录了主人的 Matrix 账号；“添加设备”
  画出二维码和码的文字，这个码让另一台设备配上。Android 模拟器上相机权限按设计处理（拒绝一次后还能再问，允许后出取景
  画面）；它经隧道的 HTTP 请求会挂住（#17），“添加设备”30 秒后显示“没要到配对码”和原因。
- **真机**（2026-10-30）：iPhone 16（iOS 27.2），装的是提交 ae0c0fd 的构建（卸载重装过，没有以前留下的连接信息）；服务器
  是局域网里一台 amd64 主机（Ubuntu 24.10），用公开的一行命令安装，核心镜像由 main 的 ea888d5 构建，四个容器都健康。App
  扫局域网状态页上的首次配对码，一次配上：核心日志里配对（`kind=First`）后 5 秒隧道连上，再过 1 秒以主人身份登录 Matrix。
  手机在 5G 下，“家”页显示“已连接 · 中继 · 619 ms”，聊天照常（ADR-0005）；这期间服务器的主中继（iroh 的 home relay）在
  aps1 和 usw1 之间来回切（#16）。真机上没测“添加设备”和“重新配对”，Android 真机没测。

## 验证结果（M1，核心接入 Tuwunel）

Tuwunel 成为核心管理的组件，上面的接口按设计工作。验证版本：Tuwunel v1.9.4。可复跑：`deploy/dev/core/smoke.sh`
（“已配对设备经隧道登录 Matrix”一步）；管理账号的几种情况在假 Tuwunel 上测（`cargo test -p wonton-core matrix`）。

- 部署：Tuwunel 只在核心的私有网络里（`deploy/dev/core/compose.yaml`），App 和 Bot 都经核心网关访问它。核心的
  配置是 `WONTON_MATRIX_URL`、`WONTON_MATRIX_SERVER_NAME`（正式安装见“补充：Matrix 的服务器名”）、`WONTON_MATRIX_JWT_KEY`（Ed25519 私钥，
  PEM）和 `WONTON_MATRIX_REGISTRATION_SECRET`，要么都不配，要么配齐。主人的用户名（`WONTON_OWNER_NAME`）就是主人的
  Matrix 用户名，规则见 ADR-0022。配置了却在 90 秒内连不上 Tuwunel，或者
  管理账号用不了，核心启动失败，不降级运行。
- 管理账号 `@wonton-core:<server_name>`：首次启动用共享密钥注册（`admin: true`，随机密码不保存），访问令牌存在
  `<数据目录>/core/matrix-admin-token`（0600）。重启时 `whoami` 通过就沿用；令牌失效，或文件丢了而账号还在时，
  用核心签的 JWT 登录同一个账号换新令牌。每次启动都调一次管理接口，确认它是服务器管理员：实测普通账号的
  令牌调 `GET /_synapse/admin/v2/users/{user}/devices` 得到 403 `M_FORBIDDEN`，管理账号拿到设备列表。
- 经隧道（隧道客户端示例扮演手机）：登录令牌的 `homeserver_url` 是 `http://matrix.wonton.localhost:<端口>`，
  `user_id` 是 `@owner:wonton.localhost`；用它经 `matrix.<基础域名>` 登录，拿到访问令牌、刷新令牌和设备 ID，
  `whoami` 是主人。同一个令牌有效期内能再登录一次（Tuwunel 不查 `jti`，同 M0-2）。
- 局域网的浏览器带着核心会话也拿不到登录令牌（401），进 `matrix.<基础域名>` 得到 403 `M_FORBIDDEN`。
  `/_synapse/admin/`、`/_matrix/federation/` 得到 404；`/_matrix/client/../../_synapse/admin/v1/register`
  （`curl --path-as-is`）得到 400；会话 ID 里带 `%2F` 的 `room_keys` 请求原样转给 Tuwunel。
- 开发环境的公钥经环境变量传给 Tuwunel（`DOCKER_HOST` 指向远端时挂不了本机文件），轮换签发密钥要重建 Tuwunel
  容器。按 M0-2 的结论，正式安装时公钥写在核心生成的配置文件里，才能热加载轮换。

## 验证结果（M1，Bot 账号）

声明了 `matrix.bot` 的插件由核心代 Bot 登录、建主房间，规则见插件规范的“Matrix Bot”。验证版本：Tuwunel v1.9.4、
matrix-sdk 0.19.1。可复跑：`deploy/dev/core/smoke.sh`（“回声 Bot”一步）；分配、沿用、回滚和清理的各种情况在
假 Tuwunel 上测（`cargo test -p wonton-core bot`），假 Tuwunel 的行为按下面的实测写。

- Tuwunel 的管理接口（一次性容器里逐个调过）：
  - 吊销设备（`DELETE /_synapse/admin/v2/users/{user}/devices/{device}`）后，令牌立即得到 401 `M_UNKNOWN_TOKEN`；
    设备本来不在也回 200。再用 JWT 登录还是同一个账号，仍在原来的房间里。
  - 停用账号后令牌失效，JWT 登录得到 403 `M_USER_DEACTIVATED`，账号退出所有房间、资料清空；再停用一次也回 200，
    不存在的用户回 404 `M_NOT_FOUND`。停用的用户名不能再用，所以删除数据后重装换成 `<名字>-2`。
  - 查账号在不在：`GET /_synapse/admin/v2/users/{user}` 对已有的账号（包括停用的、没有设备的）回 200 和
    `deactivated` 等字段，不存在回 404 `M_NOT_FOUND`；`/_synapse/admin/v1/whois` 对不存在的用户也回 200，不能用。
    JWT 登录对已有的账号是直接登录进去，所以分配 Bot 用户名时先查，已有的一律跳过，只有插件记录在案的 Bot 能沿用。
- 主房间：`createRoom` 用 `trusted_private_chat`，`initial_state` 里带 `m.room.encryption`，建出来就是加密房间。
  Tuwunel 默认建房间版本 12：建房的人权限最高，写进 power_levels 的 `users` 就回 400 `M_INVALID_PARAM`；这个预设把建房时
  邀请的人也算作建房的人（`additional_creators`），建房的人踢不动这样的人（403）。所以建房时不邀请，用
  `power_level_content_override` 只给主人 100，房间版本写明 12：版本 11 里这个 `users` 整个换掉默认值，Bot 只剩 0，
  建房失败。成员状态（`/state/m.room.member/{用户}`）对从没进过的人回 404 `M_NOT_FOUND`，注册没注册都一样；邀请后
  是 `invite`，踢掉后是 `leave`。Bot 退出后查成员回 403 `M_FORBIDDEN`。由 Bot 建，插件装好后才交给主人：
  核心用 JWT 临时登录主人的一台设备，Bot 邀请主人，主人加入房间、写入自己的 `m.direct`（没写过时读出来是 404
  `M_NOT_FOUND`），再退出登录，这台设备随之删除。主人在 App 里直接看到和 Bot 的私聊，不用接受邀请。
- 端到端（`smoke.sh`）：回声 Bot 只凭注入的令牌恢复会话（`whoami` 取设备 ID），经 `matrix.wonton.internal` 同步，
  安装时就过了健康检查。主人的一台新设备经隧道和手机的本地代理登录，在主房间里发加密消息，一两秒内收到 Bot 的
  加密回声：matrix-sdk 默认把房间密钥发给房间里的所有设备，不要求先验证。
- 卸载后 Bot 的令牌立即 401；不删数据重装是同一个账号，主人的 `m.direct` 里仍只有原来的主房间；删除数据后账号
  停用、退出主房间。没申请 `matrix.bot` 的插件（whoami）从自己的网络里访问 `matrix.wonton.internal` 得到 403
  `permission_denied`。
- matrix-sdk 的 HTTP 客户端启动时要加载系统根证书，镜像里没有 `ca-certificates` 就直接退出，即使只用 http。
  用 matrix-sdk 的 Bot 插件，镜像里要装上它。
- 第二个 Bot：Hermes Agent v0.22.5 按同样的规则装上，用户名是 `hermes`，只凭注入的令牌连上 Matrix
  （`MATRIX_E2EE_MODE=required`），在主房间里和主人的新设备加密对话两轮（模型的部分见 ADR-0008）。核心没有为它
  开任何特例。

## 验证结果（M1，网关代登录）

插件规范 v0.1 的 `sso: proxy` 已实现：应用不支持 OIDC、但认反向代理传来的用户名或一个固定密钥时，网关在转发给插件的
请求上代加清单声明的请求头或 Cookie，主人打开网页不用再登录一次。实现见 `crates/wonton-core/src/plugin/proxy_auth.rs`
（值模板和清单校验）和 `crates/wonton-core/src/gateway/proxy_sso.rs`（转发前代加、转发后清理）。可复跑：
`cargo test -p wonton-core proxy_sso`、`cargo test -p wonton-core gateway::tunnel_tests::paired_devices_open_proxy`（经真实
隧道），以及 `deploy/dev/core/smoke.sh` 的“模型设置”一步（magpie 的网页，见 ADR-0008）。

- 什么时候加：和其他插件网页一样先认出主人（局域网浏览器看这个子域名的插件会话，经隧道的已配对设备看设备身份），
  只认本插件页面发起的请求和导航（ADR-0011 的“浏览器里的隔离”）。没认出主人的请求不转发：打开页面跳到核心登录，
  其余 401。凭据只加在这个插件自己的子域名上，别的插件的页面和会话都用不上。
- 伪造不了：转发前删掉客户端发来的同名请求头和 Cookie（名字不分大小写），再加上声明的值。
- 拿不走：转发后去掉设置这些 Cookie 的 `Set-Cookie`（名字不分大小写，包括没写名字、浏览器读成同名的），以及值里带着
  代加密钥原文的响应头。响应体不改，插件不能把凭据写进页面。
- `{secret:…}` 只引用插件自己的、由核心生成且至少 16 字节的密钥；不合规的清单在预览和安装时就被拒绝。
- 冒烟实测（magpie 0.1.1327）：局域网浏览器登录过核心后，经一次会话交接打开“模型设置”；已配对设备经隧道直接打开，
  没有跳转（Android WebView 和 iOS WKWebView 两种代理方式）。没登录时打开页面跳到核心登录，只带网页密钥的 Cookie
  得到 401；别的插件网页发来的修改得到 401，没到 magpie。网页密钥不在任何响应头、响应体和 Cookie 罐里。

## 补充：Matrix 的服务器名（ADR-0051 修订）

Matrix 的服务器名必须在 Tuwunel 第一次启动前确定：每个用户 ID、房间和事件都带着它，装好后不能改。
默认使用 [ADR-0051](0051-friends-and-plugin-sharing.md#服务器名和入口默认用我们送的名字主人定) 的家名，安装输入见
[安装说明](../../deploy/install/README.md#服务器名和证书)。

- **默认送的名字。** `wonton-core init` 先向配置的家名服务注册，取得 `<家>.<域名>`，准备证书和入口，再写 Tuwunel 的配置；安装脚本最后才启动 Tuwunel。
  没配服务或邀请码、注册或签证书失败就停止，不退回随机私用名。家名和注册凭据保存在 `entry/settings.json`，Matrix 的服务器名记在
  `secrets/matrix-server-name`，核心经 `WONTON_MATRIX_SERVER_NAME_FILE` 读。重装和升级复用已记下的身份，不重新注册。
- **自己的域名（高级）。** 第一次安装时设 `WONTON_MATRIX_SERVER_NAME`（可同时设 `WONTON_MATRIX_NAME_MODE=own`）就用自己的 DNS 名，
  主人负责服务发现、入口和证书。有名字的服务器开放联邦，严格验证证书；只有配对过的目的地走 iroh，其他 Matrix 服务器由 Tuwunel 直接访问。
- **不要名字（高级）。** 必须显式设 `WONTON_MATRIX_NAME_MODE=none`，不能同时给自己的域名。此时才生成 120 位随机数的小写 base32（24 个字符）加
  `.wonton.internal`。它不含主人信息，也不是隧道 endpoint ID；`.internal` 是留给私用的顶级域。此模式保留 ADR-0050 的封闭联邦、自签证书和 TLS 例外，
  只与同类已配对云吞通信，和有名字的服务器不通。安装时警告：以后不能改名；要接入 Matrix 生态必须清空聊天数据、所有人换新账号。
- **之后不变。** 再执行 `init`（重装、升级）都用记下的服务器名，要求的名字和记录不一致就拒绝；有名字的服务器也不能改成不要名字。
  已记录的私用名继续沿用，不自动注册家名。换名只能重装 Matrix 数据，不写身份迁移（绿地期）。
- **和基础域名分开。** 基础域名只是 App 经隧道访问核心和插件网页用的主机名（`matrix.<基础域名>`、`app-*.<基础域名>`），各台相同也不要紧，
  默认仍是 `wonton.localhost`。App 经隧道找核心，配对过的服务器按交换的 endpoint ID 连接，不靠公网 DNS。
- **和第三方 App 共用家名。** 家名同时作为 Matrix 身份和 ADR-0021 的入口域名，安装时就注册；不等到打开第三方 App 入口才注册。
  名字属于运营家名服务的一方，服务停止或名字停用会使外部 Matrix 服务器无法联系这台；换运营方的域名等于换服务器名，账号不能随之搬走。
- **核心旧版本。** 旧版本的升级器照 `init <安装目录> <基础域名>` 调新镜像，参数格式保留，`init` 不再用基础域名。新装的服务器手动退回
  （`WONTON_CORE_PIN`）到这个改动之前的核心时，旧的 `init` 拿基础域名当服务器名，和配置里的对不上就报错，`install.sh` 停下，不改任何东西。
  一键升级失败的回滚恢复的是备份，不执行 `init`，不受影响。
