# ADR-0089 HTTPS 先用主人自己的域名：lego DNS-01，设置里预设 deSEC，插件只认一个规范对外地址

- 状态：已接受（2026-11-12）。方向由主人 2026-11-12 定，见“主人已定”
- 补充 [ADR-0021](0021-third-party-clients.md)：证书、局域网直连、入口容器都不变；改的是“自己的域名”怎么配、插件怎么拿到自己的地址。
  公网入口怎么进来见 [ADR-0090](0090-public-access.md)

## 背景

ADR-0021 定了第三方 App 的 HTTPS：一张 Let's Encrypt 通配符证书、DNS-01、私钥只在家里，局域网直连
`https://<标签>.lan.<后缀>:8483`。默认的“送的名字”要家名服务，主人定了家名服务留在 M2。没有家名服务时，HTTPS 只剩“自己的域名”，
而这条路在代码里只能装机时用环境变量配（`WONTON_MATRIX_SERVER_NAME`、`WONTON_ACME_DNS_PROVIDER`、`WONTON_ACME_DNS_ENV_FILE`，
见 `crates/wonton-core/src/entry_settings.rs`），App 里没有入口。

卡住的插件（2026-11-12 对照前辈项目的调研，读于主线 96763a205）：

- **obsidian-livesync**：手机上的 Obsidian 只连有效证书的 HTTPS；Android 7 起 App 默认不认用户装的 CA（LiveSync #898 实测）。
  局域网入口加公共证书就够，不需要公网。
- **joplin-server**：逐个请求比对 `Host` 和 `APP_BASE_URL`，不一致就报 `Invalid origin`；经隧道、局域网、公网三个入口进来，`Host`
  各不相同，总有两个打不开。
- **n8n**（ADR-0090）：往外部服务登记回调时要一个固定的公网地址填 `WEBHOOK_URL`，这时没有请求可读，`X-Forwarded-Host` 不够。

前辈里要让原生 App 能用的，最后都落在公共 CA（Nabu Casa、Tailscale、Cloudron、plex.direct）；私有 CA 只对浏览器有用（StartOS、Umbrel）。

## 决策

### 1. 证书：主人自己的域名 + lego DNS-01

照 ADR-0021 的“证书”：一张证书覆盖 `<后缀>`、`*.<后缀>` 和 `*.lan.<后缀>`，DNS-01，lego 一次性容器只拿 CSR，私钥和 ACME 账号钥匙
只在家里，核心合成 `site.pem` 交给入口容器。这里只改“谁写 TXT”：

- **主人的 DNS 服务商，经 lego 自带的 provider。** 主人给服务商名（lego 的 provider 代码，如 `desec`、`cloudflare`、`alidns`）和凭据，
  核心写成 lego 的 dotenv 文件，只挂进 lego 容器。照 Home Assistant letsencrypt 的 `dns-lego` 通用项：**核心不认识具体服务商，
  没有按服务商分支的代码。** lego 支持约 230 家（仓库 `providers/dns` 2026-11-12 有 232 个目录）。
- **DNS-01 不用开 80、443。** 只开局域网入口时，家里不用任何入站端口；也绕开国内 80/443 的备案。
- **凭据是密钥。** 照 [ADR-0064](0064-input-secrets.md)：App 的原生表单里填，核心存密钥库（0600），不回显，Bot 和 MCP 总线拿不到；
  只在 lego 运行时挂进它的容器，用完容器就删。
- **Let's Encrypt 的订户协议由主人在 App 里自己勾选同意**，核心不替他同意（`WONTON_ACME_ACCEPT_TERMS` 改由这个勾选写入）。
- **续期、限额、失败即停**都照 ADR-0021：按 ARI 续期；失败时继续用旧证书，App 写明到期时间和 lego 的错误；过期后如实失败，不换自签名，
  不退回 `http`。

### 2. 设置里预设 deSEC，没有域名的自己注册

App 的“HTTPS 和域名”设置页列出服务商，**默认推 deSEC**，其后是 Cloudflare、阿里云、腾讯云，最后一项“其他”按 lego 代码填名字和
`键=值` 凭据。

- **预设是数据，不是代码。** 每项是 lego 代码、显示名、凭据的变量名（取自固定版本 lego 的 `providers/dns/<名>/<名>.toml`）和注册页地址，
  和 lego 镜像的版本一起维护。加一家只改数据。
- **没有域名的用户**在 [deSEC](https://desec.io/) 免费注册一个 `<名字>.dedyn.io`，建一个令牌填回来。App 写一句人话说这是什么、去哪注册，
  “详情”里放步骤。我们不代注册，不经手这个账号。
- **为什么是 deSEC**：德国的非营利组织，免费，有 API 令牌和 lego 的 `desec` provider（lego v3.7.0 起）。`dedyn.io` 在 PSL 的私有部分
  （2026-11-12 查 `public_suffix_list.dat`），各家之间的 Cookie 互相隔离，不撞 ADR-0021“防滥用和 PSL”的问题。每个账号只能有一个
  `dedyn.io` 名字，下面的子名字作记录管理（deSEC 文档 `dns/domains.rst`）。
- **令牌按最小权限建。** deSEC 的令牌能用 policy 限定可写的子名字和类型（文档 `auth/tokens.rst` 的 “Token Scoping”）。App 的“详情”教主人
  建只能写 `_acme-challenge`、`_acme-challenge.lan` 两个 TXT 的令牌。没限定的令牌也能用，App 不拒绝，但标出“这个令牌能改整个域名”。
- 不预设 DuckDNS：一个名字只有一条 TXT，同时签主名字和通配符可能冲突，能不能指向内网也没核实。

### 3. 后缀和 Matrix 服务器名

- **装机时用了自己的域名的**（`WONTON_MATRIX_SERVER_NAME`）：后缀就是服务器名，装好不能改（ADR-0051：Tuwunel 的数据属于这个名字）。
  设置页只能换 DNS 服务商和凭据。
- **装机时选了“不要名字”的**（`WONTON_MATRIX_NAME_MODE=none`）：设置页能另填一个后缀，只给入口用，不开联邦，Matrix 服务器名不变。
  后缀能改；改了以后第三方 App 都要重填，App 改前写明。这样 M1 的用户不用重装就能打开 HTTPS。
- **送的名字**（家名服务）在 M2，见 ADR-0021、ADR-0051。装机脚本的“自己的域名”同样接受上面的预设（`WONTON_ACME_DNS_PROVIDER=desec`）。

### 4. 主人自己加的 DNS 记录

lego 只写 TXT。其余记录由主人在服务商那里加，**核心只查不写**（ADR-0021“自己的域名时”）：

| 记录 | 什么时候要 | 核心怎么查 |
| --- | --- | --- |
| `*.lan.<后缀>` A → 家的局域网地址 | 打开任何局域网入口 | 打开时和地址变化时查一次，不对就是 `lan_unavailable`，写明要加的那一条 |
| `<后缀>` CAA `letsencrypt.org; validationmethods=dns-01; accounturi=<这个家的账号>` | 推荐，不强制 | 查到就显示“已限定账号”，查不到显示“建议加”，给出整条记录 |
| 公网的 A/AAAA 或 CNAME | 打开公网访问，见 ADR-0090 | 入口自检（ADR-0021） |

CAA 加上以后，DNS 令牌泄露也签不了这个域名的证书（账号不对）。CT 监控照 ADR-0021，对自己的域名同样适用。

### 5. 规范对外地址：一个变量 `WONTON_EXTERNAL_URL`

“应用只认一个 Host”和“回调要登记的公网地址”合成一个变量，同一个声明控制：

- **核心给声明了 `clients.http` 或 `webui.guests`（ADR-0090）的插件注入 `WONTON_EXTERNAL_URL`**，和其他 `WONTON_*` 一样只用于 compose
  插值（ADR-0006）。别的插件引用它就是清单不合法，装不上。
- **值取主人实际打开的那个入口：**
  1. 这个插件的公网入口开着：`https://<标签>.<后缀>`；
  2. 否则局域网入口开着：`https://<标签>.lan.<后缀>:<WONTON_CLIENT_PORT>`；
  3. 都没开：和 `WONTON_APP_URL` 相同（App 内经隧道的地址）。
- **值变了，核心只重建引用它的服务**（ADR-0077）。主人开关入口时，确认页写明“这个应用会重启”。
- **公网总开关（ADR-0090）不改这个值。** 总开关是应急闸门，关一下不该让所有应用重启、让 n8n 重新登记回调；再打开就恢复原样。
- **`clients.http.single_host: true`：只认一个地址的应用（Joplin）。** 经隧道、局域网、公网进这个插件的请求，核心都把 `Host` 改成
  `WONTON_EXTERNAL_URL` 的主机名（带端口），`X-Forwarded-Proto` 改成它的协议，原来的 `Host` 放进 `X-Forwarded-Host`。没声明的插件
  `Host` 不动，照旧从 `X-Forwarded-Host` 读请求的地址。
- 改写放在核心，不放在入口容器：经隧道的请求不经过入口容器，三条路只有核心都在。

这修订了 ADR-0021 原来的“插件按 `X-Forwarded-Host` 得到请求的地址，清单和插件都不知道域名”。

### 6. 不做私有 CA

不加“核心自建 CA、手机自己装”兜底。理由同 ADR-0021“没选的做法”，再加上 LiveSync #898：Android 装了用户 CA，Chrome 能连，App 照样
`Trust anchor not found`。

### 7. 不变的

- App 和插件网页经 iroh 隧道，隧道里只跑 http，不另加 HTTPS（ADR-0009、ADR-0063）。这篇只管第三方 App 和浏览器从入口进来的路。
- 证书只在入口容器里用。非 HTTP 的 TLS 协议要拿私钥（Cloudron 的 tls addon、HA 的 `/ssl`），违背 ADR-0021，不做。

### 8. 契约、提供方、消费方和契约测试

| | 证书 | 规范对外地址 |
| --- | --- | --- |
| 契约 | ACME（RFC 8555）DNS-01；lego CLI 的 provider 约定（`--dns <代码>` + 环境变量）；产物是证书目录里的 `site.pem`（ADR-0021） | 注入变量 `WONTON_EXTERNAL_URL` + 清单 `clients.http.single_host`；HTTP 的 `Host`、`X-Forwarded-Host`、`X-Forwarded-Proto` |
| 提供方 | deSEC（预设默认）、Cloudflare、阿里云、腾讯云，任何 lego provider；主人自己放 `site.pem`（`WONTON_ENTRY_CERTIFICATE=external`）；M2 的家名服务（acme-dns） | 核心，按入口开关算 |
| 真实消费方 | 入口容器（HAProxy）；手机上的 Obsidian（LiveSync）、Immich 官方 App | joplin-server（`APP_BASE_URL`、`single_host`）；n8n（`WEBHOOK_URL`，ADR-0090） |

**换提供方时消费方不用改**，照 `model_gateway_contract` 的样板，一组断言跑在两个提供方上：

- `entry_certificate_contract`：Pebble 当 ACME 服务器，同一组断言分别跑 lego 的 `exec` provider（测试里几十行的假 DNS 写入器，经
  `pebble-challtestsrv` 应答）和 `acme-dns` provider（已有的家名路径）。断言：`site.pem` 的路径、格式、权限一样；证书覆盖三个名字；
  私钥没进 lego 容器；入口容器收到 `USR2` 后不断已有连接；插件渲染出的 compose 字节相同。要 Docker，标 `#[ignore]`。
- `external_url_contract`（默认测试，不要 Docker）：同一个插件分别配两个 DNS 服务商，`WONTON_EXTERNAL_URL` 和三条路上看到的
  `Host`、`X-Forwarded-*` 完全相同；只换入口开关时值按第 5 节的顺序变；没声明 `single_host` 的 `Host` 不动。

## 安全上变了什么

| | 变了 | 不变 |
| --- | --- | --- |
| DNS 凭据 | 家里多存一份主人 DNS 服务商的令牌。泄露后能写 TXT；没有 CAA 时能签这个域名的证书，令牌没限定时能改整个域名 | 只在密钥库和 lego 的一次性容器里；插件、Bot、入口容器都拿不到 |
| 证书透明日志 | 公开日志里出现 `*.<后缀>`、`*.lan.<后缀>` 和后缀本身 | 具体标签不进日志（通配符），列不出装了哪些应用 |
| 公网 DNS | `*.lan.<后缀>` 公开解析成家的私有地址（ADR-0021 已接受） | 外人拿到 `192.0.2.10` 这类地址用不上 |
| 入站端口 | 只开局域网入口时没有任何公网入站 | 公网入口要单独打开，见 ADR-0090 |
| 隧道 | 不变 | App 和插件网页仍只经 iroh，只认配对设备 |

## 理由

- 公共证书是原生 App 唯一都认的形式；DNS-01 是唯一既不要入站端口、又能签通配符的验证方式。
- lego 现成支持两百多家服务商，核心只传名字和凭据，换服务商不改代码（ADR-0015）。
- deSEC 免费、非营利、令牌能限权，`dedyn.io` 已在 PSL，正好补上家名服务上线前“没有域名”的那一块。
- 一个变量同时解决“应用只认一个 Host”和“应用要往外登记自己的地址”，比两个变量少一种不一致。

## 没选的做法

- **私有 CA**：见第 6 节。
- **把家名服务里只管 DNS 的一块提前到 M1**：主人定家名服务整体留在 M2。
- **核心按服务商调 API 写 `*.lan` 的 A 记录**：要逐家写代码，违反 ADR-0015；lego 只写 TXT。局域网地址很少变，变了 App 写明要改哪条。
- **核心内嵌 ACME 客户端（instant-acme）**：自己的域名要逐家对接 DNS 服务商，正是 lego 现成的部分（ADR-0021 已否）。
- **两个变量（规范地址、公网地址）**：值的来源一样，分开只会出现两个不一致的地址。
- **入口容器改写 `Host`**：经隧道的请求不经过它，Joplin 在 App 里还是打不开。
- **公网总开关也改这个变量**：见第 5 节。

## 代价与风险

- **主人要自己有域名或去 deSEC 注册**，还要自己加 `*.lan` 的 A 记录。比“送的名字”多两步，App 把步骤写清楚，状态如实显示。
- **deSEC 是第三方免费服务**，没有 SLA；停了只影响续期，旧证书用到过期。主人可以随时换服务商，不用重装。
- **DNS 令牌在家里。** 没限定的令牌泄露的影响是整个域名，所以 App 教主人建限定的令牌，并推荐 CAA。
- **`single_host` 的插件在 App 里打开时**，应用拼的绝对链接和跳转指向规范地址；WebView 跟过去就不经隧道了，只开局域网入口时在外面打不开。
  只给声明了的插件，官方索引收录时实测。
- **入口开关一变，声明了的插件就重启。**

## 待验证

- deSEC 的 TXT 生效时间和 lego 默认的 `DESEC_PROPAGATION_TIMEOUT`（120 秒）够不够；同一名字两条 TXT（主名字和通配符）能否并存。
- deSEC 的令牌 policy 限到 `_acme-challenge` 和 `_acme-challenge.lan` 后，lego 的 `desec` provider 能否正常写入和清理。
- Joplin Server 在 `single_host` 下三条路都能登录和同步；Joplin 是 Personal Use License，上不上架另定。
- `dedyn.io` 的 `*.lan` 指向私有地址，常见路由器的 DNS 重绑定保护会不会挡（ADR-0021 的地址页自测照用）。

## 依赖

| 依赖 | 许可证 | 用途 | 方式 |
| --- | --- | --- | --- |
| lego | MIT（2026-11-12 查 go-acme/lego，最新 v5.6.0，核心固定 v5.5.2） | ACME 和 DNS-01 | 官方镜像，一次性容器，不链接 |
| deSEC | 服务；desec-stack 本身 MIT | 主人自己的 DNS | 主人的账号，我们不运营、不分发 |

核心不增加链接进二进制的依赖。

## 主人已定（2026-11-12）

- 先做“主人自己的域名 + lego DNS-01”；家名服务留在 M2，在那之前设置里预设 deSEC，没有域名的用户自己注册。
- 规范对外地址取主人实际打开的那个入口，和公网访问要的公网地址合成一个变量。
- 不做私有 CA。

## 关联

ADR-0006（插件变量）、ADR-0009（隧道里不加 HTTPS）、ADR-0015（小组件）、ADR-0021（第三方 App 入口）、ADR-0051（服务器名）、
ADR-0063（内部签发方）、ADR-0064（用户填写的密钥）、ADR-0077（只重建用到的服务）、ADR-0090（公网访问）。
