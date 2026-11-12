# ADR-0009 身份、配对与单点登录

- 状态：已接受（2026-10-28），M0 选型已定（见验证结果）；加设备和找回的做法 2026-10-29 改定（见决策和理由）；小程序单点登录的默认做法和主人用户名的规则由 ADR-0022 修订；一个 App 连多台服务器的做法由 ADR-0042 定；家人的账号、设备和配对码由 [ADR-0045](0045-family-members.md) 定；Matrix 的服务器名 2026-11-06 改成每台唯一（见“补充：Matrix 的服务器名”），默认改用 ADR-0021 的家名、对整个 Matrix 开放联邦见 [ADR-0051](0051-friends-and-plugin-sharing.md)；插件后端访问签发方的地址由 ADR-0063 取代

## 背景

旧方案的账号和设备绑定由端脑云 User Center 负责。新产品不接任何云账号。

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

**配对**

1. 还没登记的设备用 ALPN `wonton/pair/0` 连上服务端（和隧道的 `wonton/tunnel/0` 在同一个 endpoint 上），开一条双向流，
   写入请求后结束发送：`{"type": "pair", "secret": "…", "device_name": "…"}`。`device_name` 是主人在设备列表里认设备用的名字，
   App 配对时填，去掉首尾空白后为空就报 `InvalidInput`，不登记一台没有名字的设备。配对成功后它随连接信息存在设备本机，
   Matrix 登录和推送登记拿它当设备显示名，确认弹窗里看到的也是它。
2. 服务端回一个 JSON 后结束：`{"result": "paired"}`，或者 `{"result": "rejected", "reason": "…", "message": "…"}`。请求和
   回应都不超过 4096 字节，超过就断开。
3. 登记的是 QUIC 握手认证过的设备 endpoint ID，请求里没有身份字段。成功后设备改用隧道的 ALPN 连接，之后和已登记的设备
   一样。

| `reason` | 含义 | App 的 `PairingFailure` |
| --- | --- | --- |
| `unknown_code` | 服务端没发过这个码 | `Rejected` |
| `used` | 码已经用过 | `Rejected` |
| `expired` | 码过期了 | `Expired` |
| `closed` | 这种码不再能用来配对，例如已经有配对设备时的首次配对码 | `Closed` |
| `bad_request` | 请求格式不对或太长 | `Protocol` |
| `server_error` | 服务端自己出错了，例如设备登记表写不进去 | `Upstream` |
| 认不出的值 | 比 App 新的服务端给的原因 | `Rejected`，显示 `message` |

连不上服务端是 `TransportUnavailable`；服务端不接受 `wonton/pair/0`（版本不配）是 `Protocol`。扫到的文字不是配对码时，
`PairHome` 直接返回 `InvalidInput`。

配对沿用隧道的 30 秒限时，包含本机 endpoint 的准备、建连和读取回应。失败立即结束“配对中”，
不等 QUIC endpoint 清理完成才发布结果，也不自动重试。两端按错误类别显示失败页，替换扫码和粘贴表单，
提供重试和返回；返回只收起失败，不把它当作配对成功。试过的直连地址、中继、耗时、限时和底层原因
由客户端核心按 App 语言写进默认收起的详情，不包含配对密钥。

**码的有效期**：一次性，用过就作废，10 分钟后过期。码只在核心的内存里，核心重启后全部作废。

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

## 补充：Matrix 的服务器名（2026-11-06）

原来每台的 Matrix 服务器名默认都是基础域名 `wonton.localhost`。服务器名装好后改不了：每个用户 ID、房间和事件都带着它。以后两台家里的服务器
互相联邦（M3，ADR-0045 的“跨机”），名字一样的两台分不开，只能重装、丢掉聊天记录。所以主人定了现在就改成每台唯一，越晚改要重装的服务器越多。

- **怎么生成。** 第一次安装时 `wonton-core init` 生成 120 位随机数的小写 base32（24 个字符）加 `.wonton.internal`，如
  `k3f9qzabm2x7c4d5e6f7g2h3.wonton.internal`，记在 `secrets/matrix-server-name`，核心经 `WONTON_MATRIX_SERVER_NAME_FILE` 读。
  - 不带主人的任何信息。
  - 不是隧道的 endpoint ID：两者各管各的，换隧道密钥不牵连 Matrix。
  - `.internal` 是 ICANN 留给私用的顶级域，不会被公网注册，不会和别人的真实域名撞上。
  - 用户在界面上看不到它。用户 ID 变长，只是多占几十个字节。
- **之后不变。** 记下以后再执行 `init`（重装、升级）都用记下的，安装时要的和记下的不一致就拒绝。记录之前装的服务器有 Tuwunel 的配置、
  没有这个文件：接着用配置里原来的名字并记下。这样的服务器要换名字，只能重装 Matrix 的数据，不写迁移（绿地期）。
- **自己的域名。** 第一次安装时设了 `WONTON_MATRIX_SERVER_NAME` 就用它。只有要和公网的 Matrix 服务器联邦才需要，要求主人能在这个域名上提供
  Matrix 的服务发现。核心和 App 不靠它找服务器：App 经隧道连核心（ADR-0004），以后两台家里的服务器之间也走 iroh，按配对时交换的 endpoint ID 连，
  不查 DNS。
- **和基础域名分开。** 基础域名只是 App 经隧道访问核心和插件网页用的主机名（`matrix.<基础域名>`、`app-*.<基础域名>`），各台相同也不要紧，
  默认仍是 `wonton.localhost`。开发环境（`deploy/dev/`）不设服务器名，用基础域名。
- **和第三方 App 入口的家名（ADR-0021）分开。** `<家>` 是打开公网入口时向家名服务注册才有的，会进证书透明日志，主人可以注销、换成自己的域名；
  服务器名在装的时候就要有，以后也不能变。拿 `<家>` 当服务器名，就把公网入口和 Matrix 的身份绑在一起了：没开公网入口的家没有名字，换家名就换了所有账号。
  反过来服务器名也不进家名服务和证书。
- **核心旧版本。** 旧版本的升级器照 `init <安装目录> <基础域名>` 调新镜像，参数格式保留，`init` 不再用基础域名。新装的服务器手动退回
  （`WONTON_CORE_PIN`）到这个改动之前的核心时，旧的 `init` 拿基础域名当服务器名，和配置里的对不上就报错，`install.sh` 停下，不改任何东西。
  一键升级失败的回滚恢复的是备份，不执行 `init`，不受影响。
