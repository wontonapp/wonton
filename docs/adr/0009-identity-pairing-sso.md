# ADR-0009 身份、配对与单点登录

- 状态：已接受（2026-10-28），M0 选型已定（见验证结果）

## 背景

旧方案的账号和设备绑定由端脑云 User Center 负责。新产品不接任何云账号。

## 决策

- **服务器即账号**：安装后，局域网网页和终端显示配对二维码（服务器 endpoint ID、当前中继、一次性令牌）。
  App 扫码即成为主人，不需要注册。
- **设备身份**：每个 App 安装生成密钥对；核心记录已配对设备，主人可以随时吊销。隧道只接受已配对设备。
- **Matrix 登录**：核心为配对设备和 Bot 签发 Tuwunel 登录（Tuwunel 支持由运营方签发 JWT 登录）。
- **家人**：主人可以邀请家人（M2），每人一个 Matrix 用户和私人空间；一个 App 可以连多台服务器。
- **小程序单点登录**：所有插件网页都经核心网关鉴权；支持 OIDC 的应用由核心提供 OIDC 单点登录，
  不支持的保留应用自身的登录。
- **找回**：主人丢失手机时，在局域网网页上重新配对；能接触到局域网即视为可信。

## 理由

不依赖任何云账号，也就没有账号数据要保护；配对体验和“扫码加好友”一样简单。

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
   加 `redirect_paths`），把签发方和各端点地址注入插件；卸载时删除客户端。
3. 应用把浏览器带到 `auth.<base>`（网关转发给 Hydra 的公开端点）→ Hydra 跳到核心的登录端点 →
   核心按会话接受登录（`subject` 是用户 ID）→ 授权确认端点只发放客户端可申请的 scope，并按 scope 附上
   声明（`name`、`preferred_username`、`email`、`groups`）→ 回到应用。Hydra 不记住会话，每次授权都
   回到核心检查。实测从未登录到进入 Grafana 首页共 12 次跳转。
4. 插件后端在自己的网络里经别名 `auth.<base>` 访问同一个签发方地址（令牌、用户信息端点），浏览器和
   插件看到的签发方一致。Hydra 只在核心的私有网络里，插件访问不到它的管理端口（冒烟脚本验证）。

**遗留**

- App 内 WebView 经隧道打开：见下一节。WebView 的 HTTP 代理设为客户端核心的本地代理，`Host` 原样保留，
  插件地址和签发方地址在手机上和服务器上一致。
- M0 的网关和 Hydra 只跑 http（Hydra 用 `--dev`），局域网和隧道内的 HTTPS 方案在 M1 定。
- 核心会话在内存里，核心重启后要重新登录。
- 家人账号（M2）：`subject` 按用户区分，`groups` 给出家庭角色。

## 验证结果（#21，网关接受隧道身份）

核心内置隧道入口（`crates/wonton-core/src/tunnel.rs`）：iroh endpoint 和隧道服务端跑在核心进程里，每条隧道
流直接交给网关的 HTTP 服务，QUIC 握手认证过的设备 endpoint ID 放进请求扩展（`TunnelPeer`）。局域网的 TCP
请求里没有这个扩展，请求头也伪造不了它。可复跑：`cargo test -p wonton-core gateway::tunnel_tests`（真实
隧道，只走回环地址），以及 `deploy/dev/core/smoke.sh` 的“已配对设备经隧道打开插件”一步。

- **设备登记**：这组验证里主人用令牌经管理 API 登记设备（`dev.sh allow <endpoint ID>`），扫码配对见 #16。
  登记表存在 `core/devices.json`，所有设备都属于主人。吊销（`dev.sh revoke`）立即断开该设备的隧道连接；
  网关对每个请求再查一次登记表，吊销和断开之间漏进来的请求也不放行。
- **放行范围**：插件网页、核心首页、OIDC 登录和授权确认端点接受隧道身份。经隧道的已配对设备不需要核心
  会话，也不需要插件会话交接；Hydra 回到核心的登录端点时，核心直接按设备身份接受登录。冒烟实测：
  - 未登记的设备在隧道层就被拒（本地代理回 403 `unauthorized`），请求到不了网关。
  - 登记后打开 Grafana，经过 9 次跳转（只有 OIDC）直接进入首页，用户是 `owner`；浏览器登录要 12 次。
  - 全程网关不发 Cookie。吊销后的下一个请求立即被拒。
- **管理 API 不接受隧道身份**：隧道身份和 Cookie 一样随请求自动带上，App 里任何插件网页都能借它向核心
  发跨站请求（例如用 `<form>` 提交安装插件）。所以插件管理和设备管理 API 仍只认 `Authorization: Bearer`。
- **前提**：网关信任隧道身份，前提是设备上只有本 App 用得了它的本地代理。本地代理监听回环地址，同机其他
  App 也能连上（ADR-0004 的结论），所以设备一侧的本地代理必须校验调用方，见 ADR-0004 的验证结果（#21）。
- **App 内打开**：iOS 模拟器的 WKWebView 和 Android 模拟器的 WebView 经本地代理打开
  `http://app-grafana.wonton.localhost:18480/`，都直接进入 Grafana，用户是 `owner`，没有看到任何登录页。
  做法和遇到的问题见 ADR-0004 的验证结果（#21）。


## 接口：App 登录和使用 Matrix（M1）

App（客户端核心）经隧道登录和使用 Tuwunel，不需要知道 Tuwunel 在哪里。两个入口都只给经隧道接入的已配对设备
（网关看隧道身份，局域网请求不行），管理 API 的规则不变，仍只认主人令牌。实现见
`crates/wonton-core/src/gateway/matrix.rs` 和 `crates/wonton-core/src/matrix/`。

**1. 取登录令牌：`POST <核心地址>/_wonton/matrix/login-token`**

- 没有请求体，鉴权只看隧道身份。核心域名上的跨站修改拦截对它生效，响应也不带 CORS 头：插件网页能让 WebView
  带着隧道身份发请求，但拿不到令牌。
- 响应 200（JSON）：

  | 字段 | 说明 |
  | --- | --- |
  | `homeserver_url` | Matrix 客户端接口的地址：`<scheme>://matrix.<基础域名>[:端口]`，经隧道访问，`Host` 原样保留 |
  | `server_name` | Tuwunel 的 `server_name`，即用户 ID 冒号后的部分 |
  | `user_id` | 设备所属的用户，如 `@owner:wonton.localhost`。M1 的已配对设备都属于主人 |
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

**配对码**：二维码里是一个地址 `wonton://pair?v=0&id=…&relay=…&addr=…&domain=…&core=…&secret=…`。

| 参数 | 说明 |
| --- | --- |
| `v` | 格式版本，不兼容地变化时加一。App 认不出的版本提示更新 App，不当成坏码 |
| `id` | 服务端的 endpoint ID |
| `relay`、`addr` | 中继地址、直连地址，各自可以有多个，合起来至少一个：n0 的地址发现在国内靠不住（ADR-0004） |
| `domain` | 基础域名。本地代理只转发它和它的子域名 |
| `core` | 核心自己的地址，经隧道访问，主机名就是基础域名 |
| `secret` | 一次性的配对密钥：128 位随机数，32 位十六进制小写 |

认不出的参数忽略，同一个版本里可以往码里加可选的字段。

**配对**

1. 还没登记的设备用 ALPN `wonton/pair/0` 连上服务端（和隧道的 `wonton/tunnel/0` 在同一个 endpoint 上），开一条双向流，
   写入请求后结束发送：`{"type": "pair", "secret": "…", "device_name": "…"}`。`device_name` 给主人在设备列表里认设备用。
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

**码的有效期**：一次性，用过就作废，10 分钟后过期。码只在核心的内存里，核心重启后全部作废。

**状态页**：局域网浏览器用 IP 地址打开核心的端口（`http://<服务器 IP>:<端口>/`），看到运行状态和可用的配对码。只回应
`Host` 是 IP 地址的请求：别的网站把自己的域名解析到局域网 IP（DNS 重绑定）后，从浏览器里读不到配对码。

**开发和冒烟**：主人令牌 `POST /_wonton/api/pairing-codes` 发一个码，回应 `{"code": "wonton://pair?…", "expires_in": 600}`。

## 验证结果（M1，核心接入 Tuwunel）

Tuwunel 成为核心管理的组件，上面的接口按设计工作。验证版本：Tuwunel v1.9.4。可复跑：`deploy/dev/core/smoke.sh`
（“已配对设备经隧道登录 Matrix”一步）；管理账号的几种情况在假 Tuwunel 上测（`cargo test -p wonton-core matrix`）。

- 部署：Tuwunel 只在核心的私有网络里（`deploy/dev/core/compose.yaml`），App 和 Bot 都经核心网关访问它。核心的
  配置是 `WONTON_MATRIX_URL`、`WONTON_MATRIX_SERVER_NAME`（默认基础域名）、`WONTON_MATRIX_JWT_KEY`（Ed25519 私钥，
  PEM）和 `WONTON_MATRIX_REGISTRATION_SECRET`，要么都不配，要么配齐。主人的用户名（`WONTON_OWNER_NAME`）就是主人的
  Matrix 用户名，要是合法的 localpart，也不能是管理账号的 `wonton-core`。配置了却在 90 秒内连不上 Tuwunel，或者
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
- 主房间：`createRoom` 用 `trusted_private_chat`、`is_direct`，`initial_state` 里带 `m.room.encryption`，建出来就是
  加密私聊。由 Bot 建、邀请主人；核心用 JWT 临时登录主人的一台设备加入房间、写入主人的 `m.direct`（没写过时读出来
  是 404 `M_NOT_FOUND`），再退出登录，这台设备随之删除。主人在 App 里直接看到和 Bot 的私聊，不用接受邀请。
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
