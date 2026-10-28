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
`cargo test -p wonton-matrix --test tuwunel login`；签发逻辑见
`crates/wonton-matrix/tests/tuwunel/support/issuer.rs`，Tuwunel 配置见 `deploy/dev/matrix/tuwunel.toml`。

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

- App 内 WebView 经隧道打开（依赖 #2、#6）还没验证。设备经隧道接入后，网关直接用隧道认证的身份，
  插件子域名不再需要会话交接；客户端本地代理要原样保留 `Host`（例如把 WebView 的 HTTP 代理设为本地
  代理），这样插件地址和签发方地址在手机上也保持一致。
- M0 的网关和 Hydra 只跑 http（Hydra 用 `--dev`），局域网和隧道内的 HTTPS 方案在 M1 定。
- 核心会话在内存里，核心重启后要重新登录。
- 家人账号（M2）：`subject` 按用户区分，`groups` 给出家庭角色。
