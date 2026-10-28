# ADR-0009 身份、配对与单点登录

- 状态：已接受（2026-10-28），部分 M0 选型

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
