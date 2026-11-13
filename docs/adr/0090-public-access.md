# ADR-0090 公网访问：自己的域名或 Cloudflare Tunnel 当入口，默认关，一键全关，回调按路径放行，访客链接

- 状态：已接受（2026-11-12）。方向由主人 2026-11-12 定，见“主人已定”
- 补充 [ADR-0021](0021-third-party-clients.md)：把它的公网入口从“第三方原生 App”扩成通用的公网访问；把它“换入口”里的两种做法
  提到 M1。补充 [ADR-0011](0011-security-model.md) 的“网络暴露”：访客链接是一种匿名访问。HTTPS 和规范对外地址见 [ADR-0089](0089-https-own-domain.md)

## 背景

ADR-0021 的公网入口（`clients.http`、`network.clients`、`public.allow/deny`、按插件的开关）默认经我们的中继和家名服务，这两样在 M2。
没有它们，M1 里所有要被外面访问的插件都卡着（2026-11-12 对照前辈项目的调研，读于主线 96763a205）：

- **n8n、astrbot**：GitHub、企业微信、QQ 这类外部服务要往家里发回调。
- **halo**：公开博客。ADR-0051 的好友分享要对方有 Matrix 账号和 App，解决不了陌生人读博客。
- **scribble-rs、razzia**：聚会时把房间链接发给朋友，对方扫码就进。

前辈的形状一致：打包者按路径声明，主人按应用开关（YunoHost、Umbrel、Cloudron）；临时访问用带有效期、能收回的链接（Sandstorm、Nextcloud）。
Home Assistant 的教训：cloudhook 关了 Remote UI 还开着、单个关掉会自己重开、没有一键全关。

## 决策

### 1. 入口的提供方：主人自己的，默认关

“公网访问”在 App 设置里，**默认关**。主人选一种入口，用他自己的域名（ADR-0089）和他自己的账号：

| 入口 | 主人要准备 | 连接器（ADR-0021 的组件） | TLS 在哪终止 |
| --- | --- | --- | --- |
| 路由器转发 | 公网 IP，路由器把一个端口转到主机；`*.<后缀>` 的 A/AAAA 指向家 | TCP 转发容器（同局域网转发，ADR-0021“路由器转发 443”） | 家里 |
| 自己的服务器 | 一台有公网 IP 的服务器，跑原版 frps；地址、令牌、CA | frpc | 家里 |
| Cloudflare Tunnel | 域名托管在 Cloudflare；在 Cloudflare 后台建一条隧道，把令牌贴进 App | cloudflared | **Cloudflare 的边缘** |
| 我们的中继（M2） | 家名服务的邀请码 | frpc | 家里 |

- **同一时间只用一种。** 换入口时旧连接器先撤掉，再建新的。
- **不捆绑、可配置。** 我们的中继只是表里的一行，地址由配置给（`WONTON_CLIENT_SERVICE`，ADR-0021、ADR-0048 的做法），M1 不依赖它。
- **Cloudflare Tunnel 用远程管理的隧道令牌**，只能跑这一条隧道，不用 `cloudflared tunnel login` 那张整个账号的证书。主人在 Cloudflare
  后台加一条公开主机名 `*.<后缀>`，服务填入口容器的公网前端，打开 “Match SNI to Host”；App 一步一步写清楚。令牌是密钥（ADR-0064）。
- **哪个标签放行不靠 Cloudflare 的配置。** 不管 Cloudflare 那边写了什么，入口容器的 `crt-list` 只放打开了公网入口的标签，核心再查一次开关
  （ADR-0021“公网暴露面”）。Cloudflare 那边配多了，也只能到家门口。
- **Cloudflare Tunnel 的代价，选它时写明**（ADR-0021 已列）：Cloudflare 看得到全部内容，包括密码和访客链接；免费计划的请求体上限 100 MB；
  国内不稳定；`clients.tcp` 不可用。另外 Universal SSL 只覆盖一级子域名，后缀要用 Cloudflare 上的主域名本身，否则 `<标签>.<后缀>`
  没有边缘证书（见“待验证”）。所以没有域名、只有 `dedyn.io` 名字的主人用不了 Cloudflare Tunnel，用路由器转发或自己的服务器。
- 这一跳仍是入口自己的 TLS（`matchSNItoHost`，ADR-0021），入口容器只有 TLS 前端，按 SNI 拦标签的那道防线不拆。
  和“隧道里不加 HTTPS”（ADR-0009）说的不是一回事：那条说的是 App 和核心之间的 iroh 隧道，这篇不碰。

### 2. 一键关掉所有公网入口

设置页最上面一个总开关“公网访问”，关掉就是**关掉所有公网入口**（主人已定：Matrix 联邦一起关）。开关文案写明“关着时，别的服务器上的
好友联系不到你”，状态如实显示：

- 核心删掉连接器容器（frpc、cloudflared 或 TCP 转发），清空入口容器公网前端的 `crt-list`，并在 8484 上拒绝所有 `X-Wonton-Entry: public`，
  断开已有的公网连接。三道都做，任何一道没做成都如实显示“没关干净”和原因，不显示为已关（吸取 HA cloudhook 的教训）。
- 包括 ADR-0051 公网入口上的 Matrix 联邦和 `.well-known`：关着时别的 Matrix 服务器上的人联系不到这台，重新打开后对方重试才收到消息。
  经 iroh 配对过的云吞、局域网入口、App 都不受影响。
- **各插件的开关和访客链接都留着，只是不生效**，App 里标成“公网访问已关”。总开关打开前，确认页列出会重新打开的应用、路径和访客链接。
  访客链接的有效期照走，不因总开关停而顺延。
- 总开关不改 `WONTON_EXTERNAL_URL`（ADR-0089 第 5 节），应用不重启。
- 只有主人在 App 里能开关（ADR-0021：入口的开关 Bot 和 MCP 总线没有工具）。

### 3. 每个应用的公网入口

照 ADR-0021 的开关和 `public.deny`、`public.allow`，补两条：

- **确认页写一句人话：“打开后，陌生人和机器人都能访问这个应用。”** 声明了 `public` 的，列出路径原文。来源不是官方索引的，再写“这个应用没有审核”。
- **详情页列出当前公开的东西**：哪些应用开着公网入口、各放行或挡掉了哪些路径、哪些访客链接还有效。

### 4. 回调：`public.allow` 按路径放行

照 Umbrel 给 n8n 的做法，不另造组件：

- 插件用 `clients.http` + `auth: app` + `public.allow` 列出回调路径，公网上只有这几条能到应用；其余路径只在局域网入口和 App 里能用。
  n8n 的例子：

  ```yaml
  clients:
    http:
      service: n8n
      port: 5678
      auth:
        app:
          setup_done: [sh, /wonton/setup-done.sh]
      public:
        allow: [/webhook, /webhook-test, /form, /form-test, /form-waiting]
  ```

  `WEBHOOK_URL: ${WONTON_EXTERNAL_URL}`（ADR-0089）。
- **核心不验签名，不按服务商写逻辑**（ADR-0015）。回调的安全靠应用自己：n8n 的回调路径默认带 UUID，企业微信、QQ 自带签名。
  确认页写明：“这几条路径任何人都能访问，靠应用自己的密钥或签名保护。”
- `public.allow` 不绕过 `auth`：`gateway` 模式下放行的路径仍要过网关的密码。要收外部回调的插件用 `app`。
- 地址用可读标签，不另发随机的回调域名。

### 5. 访客链接

给“发个链接就能进”的应用：房间、白板、小游戏。照 Sandstorm 的分享链接和 Nextcloud 的公开链接。

**插件声明 `webui.guests: true`**，表示应用自己能接待匿名用户（自带房间码或口令）。没声明的插件不给建访客链接，App 和 Bot 都不行。声明了的插件
有入口的开关（局域网、公网），和 `clients.http` 一样默认关；只声明 `webui.guests`、没有 `clients.http` 时，入口上只有拿着有效访客 Cookie
的请求能到应用，其余回一个写着“链接已失效或不对”的页面。

**怎么用：**

- 主人在 App 里给这个应用建链接，或让 Bot 经确认卡片建（主人已定）：开到外网先弹卡片，卡片写明应用、有效期和“拿到链接的人都能进”，
  主人点了才建（核心自己的敏感工具，ADR-0024 第 1 步）；收回和总开关仍只在 App 里。链接：起个名字，选有效期，**默认 4 小时，最长 7 天**，
  可以带一个落地路径（比如房间）。
  地址是 `<WONTON_EXTERNAL_URL>/_wonton/guest/<令牌>`：公网入口开着就是公网地址；只开了局域网入口时是局域网地址，App 写明“只有连着家里
  Wi-Fi 的人能打开”；都没开时建不了，回 409 `entry_closed`。
- 令牌 128 位随机数，小写 base32。存在核心的密钥库，主人能再复制。
- 打开链接：核心校验令牌，换成只对这个主机名有效的 Cookie（`__Host-` 前缀，`Secure`、`HttpOnly`、`SameSite=Lax`，过期时间同链接），
  302 到落地路径，这个响应带 `Referrer-Policy: no-referrer`，令牌不留在地址栏后续的请求里。`/_wonton/` 在插件主机名上本来就归核心
  （ADR-0080），应用收不到这个路径。
- **Cookie 只在链接声明的路径内有效。** 后续请求带着有效 Cookie、路径又在链接 `path` 的前缀内（链接没写 `path` 按 `/`，即整个网页），
  核心视同已认证放行，仍受该入口 `public` 规则的约束（ADR-0021）；路径外的请求按该入口的正常认证走（`gateway` 的密码、应用自己的登录）。
  只声明 `webui.guests` 的入口没有正常认证可言，路径外一律当没带 Cookie，回同一个“链接已失效或不对”的页面。这样一枚泄露的 Cookie
  只给到分享的那条路径，不是整个入口。
- **访客是匿名的。** 请求转给应用的网页服务（`webui`），删掉身份头、`webui.proxy` 声明的头和 `wonton_` 开头的 Cookie，不代登录，
  不算主人也不算家人（同 ADR-0063 的匿名转发）。访客到不了 MCP 总线、核心的接口和别的插件。
- **随时收回。** 收回后下一个请求就失效，已经升级的 WebSocket 立即断开（复用 ADR-0051 收回分享的守卫）。过期同样处理。
- 一个应用可以有多条链接，各自起名、单独收回，最多 20 条有效的。
- 错的令牌按验证失败计数（核心加 `X-Wonton-Auth-Failure`，ADR-0021），同一来源试多了就被封。
- 审计：建、收回、每次换 Cookie（来源、链接名），请求只按链接聚合计数，不记内容。
- 默认不加密码，靠应用自己的房间码。

**它不是 ADR-0051 否掉的“访客通道 `wonton/guest/0`”。** 那个是给好友另造一套身份；访客链接没有身份，不进 `friends.json`，不能聊天，
不能用 Bot，只能用这一个应用的网页，到期就没了。

### 6. 公开博客

Halo 这类就是 `clients.http` + `auth: app` + `public.deny` 挡掉后台和注册（具体路径写插件时读 Halo 源码核实），走第 1 节主人自己的入口。
**经我们中继的公开博客留到 M2 再定**：会碰到 ADR-0021“防滥用和 PSL”的邀请制、律师意见和每月 100 GB。

### 7. 接口

都在核心地址上、经隧道、只给已配对设备，类型在 `wonton-protocol` 的 `clients` 模块，鉴权同 ADR-0021 的 `X-Wonton-Clients: 1`。

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET、PUT | `/_wonton/clients/domain` | 后缀、DNS 服务商和凭据（只写不读）、订户协议的勾选（ADR-0089） |
| GET、PUT | `/_wonton/clients/ingress` | 入口的种类和它的设置（只写不读的令牌）；总开关 |
| GET、POST | `/_wonton/clients/{id}/guests` | 列出、新建访客链接，请求体 `{"label", "ttl_seconds", "path"}`，`ttl_seconds` 超过 7 天回 400 `invalid_input` |
| GET、DELETE | `/_wonton/clients/{id}/guests/{链接 id}` | 取地址（记审计）；收回 |
| GET | `/_wonton/clients/status` | ADR-0021 已有，加上：入口种类、总开关和三道闸门各自的状态、当前公开的应用、路径和访客链接 |

新错误类别：`entry_closed`（409，入口没开）、`ingress_unavailable`（503，连接器起不来或连不上，附详情）、`public_access_off`（409，总开关关着）。
其余用 ADR-0021 的。

### 8. 契约、提供方、消费方和契约测试

| | 公网入口（连接器） | 访客链接 |
| --- | --- | --- |
| 契约 | 连接器把公网连接送进 `wonton-edge` 上入口容器的公网前端：TLS（SNI，RFC 6066），真实来源二选一：PROXY v2，或一个具名请求头（`WONTON_CLIENT_TRUST`，ADR-0021）。插件看到的请求形状固定：`Host`/`X-Forwarded-Host`、`X-Forwarded-Proto: https`、`X-Real-IP`、`WONTON_EXTERNAL_URL` | 一个 URL：`<WONTON_EXTERNAL_URL>/_wonton/guest/<令牌>`，换成主机名范围的 HTTP Cookie（RFC 6265 的 `__Host-`）；应用收到的是不带身份的普通请求 |
| 提供方 | TCP 转发（路由器转发）、frpc（自己的 frps；M2 我们的中继）、cloudflared（Cloudflare Tunnel） | 核心 |
| 真实消费方 | n8n（回调）、halo（公开博客）、Immich 官方 App（ADR-0021） | scribble-rs、razzia |

**换提供方时消费方不用改**，照 `model_gateway_contract` 的样板：

- `public_entry_contract`（默认测试，不要 Docker）：直接对核心的 8484 监听，用测试里的两个假连接器各跑一遍同一组断言，一个按
  `proxy-v2` 的信任方式送真实来源，一个按 `header:CF-Connecting-IP`。断言：插件收到的 `Host`、`X-Forwarded-*`、`X-Real-IP` 相同；
  `public.allow` 外的路径都被挡；伪造的 `X-Wonton-Entry`、`X-Wonton-*`、带 `_` 的头都被删；总开关关着时两种都拒绝；访客 Cookie 在两种下行为相同；
  收回后下一个请求失效、WebSocket 断开。
- 同一组断言再跑一遍真的 HAProxy 加 TCP 转发和 frpc 两个连接器，要 Docker，标 `#[ignore]`。
- **真的 Cloudflare 跑不进 CI**（要账号和外网）。CI 里用假连接器发和 cloudflared 一样的请求头；真实的 Cloudflare Tunnel 写进里程碑的
  端到端冒烟，在真实主机上手动跑。

## 安全上变了什么

| | 打开后变了 | 仍然受保护 |
| --- | --- | --- |
| 默认 | 不变：公网访问默认关，没打开就没有连接器、没有公网前端 | |
| 开着公网入口的应用 | 互联网上任何人和机器人都能到这个应用（`public.deny` 挡掉的、`public.allow` 之外的路径除外），未认证的请求能到应用自己的登录（ADR-0021 已接受）。应用的未认证漏洞直接在公网上 | 没声明 `clients` 或 `webui.guests` 的插件开不了；核心的管理接口、MCP 总线、Hydra、Tuwunel（联邦端点除外）、插件网页入口都不在路由表里 |
| 回调路径 | 任何人都能往这几条路径发请求，唯一的保护是应用自己的密钥或签名 | 其余路径在公网上到不了 |
| 访客链接 | 拿到链接的人在有效期内、在链接声明的路径内是这个应用的匿名用户；链接被转发，转到谁手里谁就能进 | 没有身份、不能聊天、不能用 Bot，只能到这个应用的网页；随时收回，最长 7 天 |
| 路由器转发 | 家的公网 IP 写进公网 DNS，路由器开一个端口，扫描器会来 | TLS 和协议解析在没有 Docker socket、连不上宿主的入口容器里（ADR-0021） |
| Cloudflare Tunnel | Cloudflare 看得到经过的全部内容，包括密码、访客令牌、回调内容 | 只有主人选了它才这样；家里不开任何入站端口 |
| 名字 | 可读标签在 SNI 里，路上的人看得到用的是什么应用（ADR-0021 已接受） | 证书是通配符，CT 日志列不出装了哪些应用 |
| 不变 | | App 和插件网页只经 iroh 隧道、只认配对设备；Docker socket 只在核心；Bot 和 MCP 总线开不了入口（访客链接 Bot 经确认卡片建，见第 5 节）；局域网入口不受总开关影响 |

## 理由

- 都挂在 ADR-0021 已有的公网入口上，只补三个小声明（`public.allow` 的用法、`webui.guests`、一个对外地址变量），不另造三套机制。
- 入口的提供方只是“把 TLS 连接送到 `wonton-edge`”的连接器，换哪个核心和插件都不改（ADR-0015）。主人自己的域名和账号，不依赖我们的云。
- 按标签放行的决定始终在家里做，所以 Cloudflare 那边配错、frps 被攻破，都打不开主人没开的应用。
- 总开关三道闸门如实报状态，是 HA cloudhook 那个问题的反面。

## 没选的做法

- **等 M2 的中继**：主人定现在就做。
- **Tailscale Funnel、ngrok**：前者要每台客户端装 Tailscale（系统 VPN，ADR-0021 已否）才能管，Funnel 本身不带鉴权；后者在边缘按服务商验签，
  是 ADR-0015 禁止的按提供方写死。
- **本地管理的 Cloudflare 隧道**（`cloudflared tunnel login`）：核心能自己写路由，但那张 `cert.pem` 管整个账号。
- **核心验回调签名**：要认识每个服务商。
- **另发随机的回调域名**：多一个组件，n8n 的路径本来就带 UUID。
- **访客链接带进 ADR-0051 的好友体系**：访客不该有账号，陌生人也不会装 App。
- **访客链接默认带密码**：聚会扫码的场景用不上，应用自己有房间码；以后有需要再加。
- **总开关关掉时也删掉各插件的开关**：重新打开要一个个再开，主人更不愿意关。

## 代价与风险

- **公网上的应用会被扫、被试密码。** 靠默认关、路径限制、ADR-0021 的按来源限速和封禁、官方索引审核、及时升级。总开关随时能关。
- **路由器转发要公网 IP**，家宽的地址会变，`*.<后缀>` 的记录要主人自己更新（路由器的 DDNS 或 ddclient）；变了以后入口自检报“DNS 不对”。
- **Cloudflare Tunnel 下 Cloudflare 能看到内容。** 选它时写明，不推荐给 Vaultwarden 这类。
- **访客链接会被转发。** 有效期默认短，详情页能看到每条链接最近用过没有，随时收回。
- **总开关包括联邦（已定）**：关着期间外面的 Matrix 服务器发来的消息要等打开后对方重试才到。
- **每多一种连接器就多一个要跟着核心发布的镜像**（cloudflared 约每月一版），digest 跟核心发布走（ADR-0021）。

## 待验证

- Cloudflare 远程管理的隧道能否用通配符公开主机名 `*.<后缀>`，后台能否打开 “Match SNI to Host”；Universal SSL 对二级子域名不签证书。
- cloudflared 的令牌能否经文件传入（`--token-file`）而不是环境变量；以非 root、只读根文件系统、`cap_drop: ALL` 能否运行；远程诊断能否关掉。
- 桌面上的隐藏虚拟机（ADR-0062）里，路由器转发的端口怎么从 Mac、Windows 的局域网地址进到虚拟机。
- 访客 Cookie 的 `SameSite=Lax` 在 scribble-rs 的 WebSocket 上能否带上。

## 依赖

| 依赖 | 许可证 | 用途 | 方式 |
| --- | --- | --- | --- |
| cloudflared | Apache-2.0（2026-11-12 查 cloudflare/cloudflared，最新 2026.10.7） | Cloudflare Tunnel 的连接器 | 官方镜像，独立容器，不链接 |
| frp（frpc） | Apache-2.0 | 自己的 frps | 已有（ADR-0021） |
| HAProxy | GPL-2.0 | 入口容器、TCP 转发 | 已有，官方镜像，独立容器，不修改不链接 |

核心不增加链接进二进制的依赖，没有 GPL/AGPL 代码链接进我们的二进制。

## 主人已定（2026-11-12）

- 现在就做“自己的域名或 Cloudflare Tunnel 当入口”，默认关，用户用自己的账号。
- 加“一键关掉所有公网入口”总开关。
- 总开关连 Matrix 联邦一起关；开关文案写明“关着时，别的服务器上的好友联系不到你”，状态如实显示。
- `public.allow` 按路径放行回调。
- 访客链接默认 4 小时、最长 7 天、随时收回。
- Bot 经确认卡片建访客链接：开到外网先弹卡片，收回和总开关只在 App 里。
- 公开博客走我们的中继，M2 再说。

## 关联

ADR-0004（隧道）、ADR-0009（隧道里不加 HTTPS）、ADR-0011（安全模型）、ADR-0015（小组件）、ADR-0021（第三方 App 入口）、ADR-0048（中继可配置）、
ADR-0051（好友和联邦）、ADR-0063（匿名转发）、ADR-0064（用户填写的密钥）、ADR-0080（`/_wonton/` 路径）、ADR-0089（HTTPS 和规范对外地址）。
