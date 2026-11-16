# ADR-0021 第三方原生 App 连回家：公网子域名、公共证书和专用密码

- 状态：已接受（2026-11-03）。`auth: app` 的 `setup_done` 和错误 `setup_pending` 由 [ADR-0022](0022-owner-identity.md) 补充；云端部分挪到 M2（主人 2026-11-04 定，见“范围”）；“Bot 装不了高权限插件”已被 [ADR-0033](0033-plugin-tabs.md) 取代；家名同时用作 Matrix 的服务器名、中继多转联邦，见 [ADR-0051](0051-friends-and-plugin-sharing.md)；自己的域名在 App 里配、预设 deSEC，插件的规范对外地址见 [ADR-0089](0089-https-own-domain.md)；公网入口扩成通用的公网访问，路由器转发和 Cloudflare Tunnel 提到 M1，总开关和访客链接见 [ADR-0090](0090-public-access.md)

## 背景

插件网页只经核心网关打开，只有已配对的 Wonton App 能经 iroh 隧道连回家（ADR-0004、ADR-0009）。Immich 手机 App、Bitwarden、Jellyfin 和 Subsonic 播放器、CalDAV/CardDAV、OPDS 阅读器、Git 客户端都连不进来。50 个候选插件的调研里（`research/plugin-candidates.md`），这是最大的共性缺口。那份调研的结论第 1、2、13 项核对后：

- **第 1、2 项的缺口属实。** `lan.ports` 确实不能列 `webui` 的端口（插件规范的“局域网”）。它建议的“把网页端口发布到局域网”没有采用，理由见“没选的做法”；它建议的“一个 TLS 端口按 SNI 路由”采用了，名字问题见“局域网直连”。
- **第 13 项不是缺口。** 按 `f664c6b` 的 `gateway/proxy.rs`，网关流式转发请求和响应，`Range` 原样带给插件；请求体发完后等响应头最多 120 秒（ADR-0006），响应体仍然不限时。这篇的新入口复用它。

主人 2026-11-03 定的方向：

1. 不做 VPN，只做公网子域名。
2. 第三方 App 连 `https://<插件>.<家>.<域名>`，用可读的名字。我们运营一个只按 SNI 转发、不解密的中继，放在香港，用和产品官网分开的域名。TLS 在家里终止，证书和私钥只在家里。
3. 不自建 CA，用 Let's Encrypt，手机上不装任何证书。接受“运营者理论上能代签证书”，写明；加 CAA 限定账号，部署时就做 CT 日志监控。
4. 保留局域网直连，同样不装证书，用固定名字，核心更新 A 记录。
5. HTTP 类的入口只有 `https`，不给 `http` 地址。非 TLS 协议也要支持，在中继上给每个这样的入口开一个独立的公网端口。
6. 公网入口默认关，主人在 App 里按插件打开，Bot 做不了。
7. 中继免费，按家限速、限每月流量，注册用邀请制。
8. 入口的接口开放：主人可以换成自己的域名或 Cloudflare Tunnel，产品域名不写死在代码里。
9. Immich、Jellyfin、Navidrome 每个应用一个密码，Vaultwarden 的主密码由主人自己设。
10. 云端先全部放在一台机器上。这一条只在邀请制阶段成立，见“部署和被攻破的影响”。时间安排见“范围”。

ADR-0004 否掉公网隧道的理由（流量都经过我们的服务器，带宽成本和暴露面都大）依然成立，这篇把它当作代价接受，见“代价与风险”。ADR-0004 的决定不变：Wonton App 和小程序继续走 iroh 隧道，不经这条路。

## 决策

| | 第三方 App 里填的地址 | 经过 |
| --- | --- | --- |
| 公网 HTTPS | `https://<标签>.<家>.<域名>` | 中继（按 SNI 转发）→ 连接器 → 入口容器解开 TLS → 核心网关 → 插件 |
| 局域网直连 | `https://<标签>.lan.<家>.<域名>:8483` | 局域网 → 局域网转发 → 入口容器解开 TLS → 核心网关 → 插件 |
| 公网 TCP | `<标签>.<家>.<域名>:<分到的端口>` | 中继（按端口转发）→ 连接器 → 入口容器 → 核心转字节 → 插件 |

### 组件

每个组件只做一件事，都能单独换（ADR-0015）。

**家里：**

| 组件 | 做什么 | 能拿到什么 | 网络 |
| --- | --- | --- | --- |
| 连接器（frpc，容器） | 向外连中继，把连接交给入口容器 | frp 登录令牌 | `wonton-relay`（只出网）、`wonton-edge` |
| 入口容器（HAProxy） | 终止 TLS、读 PROXY v2、按来源限速和封禁、转普通 HTTP 给核心 | 只有证书目录（只读） | `wonton-edge`、`wonton-lanfwd`、`wonton-clients` |
| 局域网转发（HAProxy，TCP 模式） | 发布 8483，按来源过滤，带 PROXY v2 把连接转给入口容器，不解 TLS | 什么密钥都没有 | `wonton-lanpub`、`wonton-lanfwd` |
| 证书续期（lego，一次性容器） | 申请和续期证书 | acme-dns 凭据、ACME 账号钥匙、核心给的 CSR，拿不到证书私钥 | 只出网 |
| 核心 | 生成上面这些的配置和证书钥匙、验证专用密码、按名字找插件；给入口容器开专用监听端口，监听地址由部署注入 `WONTON_ENTRY_LISTEN` | 全部（本来就有 Docker socket） | `core`、`wonton-clients`、各插件网络 |

**网络规则，不是可选项：**

- **面向公网的 TLS 和协议解析不进核心。** 核心挂着 Docker socket，攻破它就是宿主 root（ADR-0011）。入口容器不挂 Docker socket，以非 root 运行，去掉全部能力，根文件系统只读（实测，见[验证结果](0021-verification.md)）。它被攻破时，泄露的是证书私钥和经过它的第三方 App 流量，够得着的只有核心的专用监听端口（实测连不上局域网转发和 frpc 的管理端口）。
- **连接器绝不进 `core` 网络。** 那里有 Hydra 的 4445 管理端口和 Tuwunel。`wonton-edge` 是内部网络，只有连接器和入口容器；连接器只在 `wonton-relay` 上能出网。
- **入口容器连不上宿主。** `wonton-edge`、`wonton-lanfwd` 和 `wonton-clients` 都是内部网络，并设 `gateway_mode_ipv4` 和 `gateway_mode_ipv6` 为 `isolated`：宿主一侧没有地址，容器的路由表里只有这几个子网（实测）。关掉地址伪装的网络里，容器连得上宿主自己的地址（ADR-0019 实测），连得上 sshd、`${WONTON_PORT}`、隧道端口和 `network.lan` 插件的端口，所以入口容器不进 `wonton-lanpub`，由局域网转发发布 8483。这样解析公网 TLS 和 HTTP 的只有连不上宿主的容器。局域网转发、连接器和 lego 仍然连得上宿主，代价见“代价与风险”。
- **公网和局域网分两条路进入口容器。** 被攻破的 frpc 如果够得着局域网前端，就能带着任意来源发 PROXY v2，拿到 `X-Wonton-Entry: lan`，绕过公网开关和 `public`，按来源封禁也失效。所以局域网转发到入口容器单独用 `wonton-lanfwd`，连接器不在上面；每个前端绑一个接口，不只靠对端 IP 区分（实测连接器连不到）。
- **连接器、局域网转发、lego 都 `--cap-drop ALL`、非 root、根文件系统只读。** 没有 NET_RAW 就冒充不了别的地址。frp 还没到 1.0，frpc 和中继讲 frp 协议，又连得上宿主，要按被攻破来防（frpc 这样能跑、能 reload，实测）。
- **frpc 的管理接口（`webServer`，`frpc reload` 要用）只绑容器内的 `127.0.0.1`。** 否则被攻破的入口容器能改写 frpc 的代理，把 `localIP` 指向宿主，经中继把宿主的 sshd 和 `WONTON_PORT` 暴露出去（实测入口容器连不上）。
- **核心给入口容器开一个专用监听端口（默认 8484），只绑在 `wonton-clients` 上，只认入口容器（按它的固定别名解析，容器重建换 IP 不用改配置）。** 这个端口收普通 HTTP，信任入口容器写的 `X-Real-IP` 和 `X-Wonton-Entry`，绝不把请求当作主人；它和主网关共用转发代码（`gateway/proxy.rs`），只放行 `clients.http` 声明的路由。核心的 HTTP 解析（hyper）仍然看得到这些请求的字节，但前面已经挡掉了 TLS、PROXY 和来源限速。
- **主网关和隧道不在 `wonton-clients` 上服务。** 它们默认绑所有接口（`WONTON_LISTEN`、`tunnel.rs`），核心一加入这个网络，被攻破的入口容器就能连到状态页（没配对时上面有配对二维码）和 iroh 端口。主网关按连接的本地地址判断，落在这个子网上的直接关掉；隧道的 IPv4 UDP 套接字绑核心在 `core` 网络上的地址；`wonton-clients` 只有 IPv4。
- **`clients` 要求核心跑在容器里。** 核心跑在宿主上时（开发时，没设 `WONTON_CORE_CONTAINER`）没有 `core` 网络，`wonton-clients` 是 `isolated`，宿主一侧没有地址，上面两条绑定都做不到。这时打开入口回 `core_not_in_container`，不降级，不退回绑 `0.0.0.0`。

入口容器、局域网转发、连接器都由核心管：入口容器和连接器由核心经 Docker API 按设置建和删，网络由主 compose 渲染（核心始终在 `clients` 上，8484 的绑定和主网关的排除地址在启动时就定下来）。镜像 digest 跟核心发布走；有入口设置（家名或自管域名）就建这些容器，开关（按插件打开公网入口）还没接到设置里（ADR-0090 落地时接）。证书续期由核心每天起一次一次性容器。

**入口容器选 [HAProxy](https://www.haproxy.org/)（官方镜像，GPL-2.0，独立容器，不改不链接）。** 比较（HAProxy 实测，其余读文档）：

| | TLS + 文件证书热加载 | PROXY v2 只认指定对端 | 按来源限速（IPv6 /64） | 按响应状态计失败并封禁 | 普通 TCP | 要插件或自己编译 |
| --- | --- | --- | --- | --- | --- | --- |
| **HAProxy** | 有（`USR2` 热加载，实测不断连接） | 有（`tcp-request connection expect-proxy`，实测） | 有（stick-table + `ipmask`，实测） | 有（`http-response sc-inc-gpc`，实测） | 有 | 不要 |
| [Caddy](https://github.com/caddyserver/caddy)（Apache-2.0） | 要调用 reload | 有（listener wrapper） | 要 [caddy-ratelimit](https://github.com/mholt/caddy-ratelimit) | 没有 | 要 caddy-l4 | **要自己编译** |
| [Traefik](https://github.com/traefik/traefik)（MIT） | 有 | 有（`trustedIPs`） | 有（`rateLimit`） | 要 [fail2ban 插件](https://github.com/tomMoulard/fail2ban) | 有 | 这一项要 |
| [nginx](https://github.com/nginx/nginx)（BSD-2-Clause） | 要 reload | 有 | 有（`limit_req`，/64 要 `map`） | 要 Lua 或 NJS | `stream` | 这一项要 |
| [Envoy](https://github.com/envoyproxy/envoy)（Apache-2.0） | 有 | 有 | 要外部限速服务 | 要扩展 | 有 | 配置面太大 |

HAProxy 不做 ACME，证书还是 lego。核心从清单生成它的配置，所以**写进配置的值（标签、端口）都必须先过严格的字符集校验**，否则恶意清单就是配置注入。

### 地址和名字

- **`<家>`** 是注册时随机发的 8 位小写 base32，如 `k3f9qzab`。它会进证书透明日志，所以不能带主人的任何信息，也不是 iroh 的 endpoint ID。**永不复用**，见“迁移和回退”。
- **`<标签>`** 取可读的名字：插件 ID 的最后一段，重名时依次 `immich-2`，装的时候定下，升级和重启后不变。不能是 `lan`。
- **可读名字的代价（主人选择接受）：** 能探测的人（只要知道 `<家>`，而 CT 日志能列出所有的家）试一下就知道这家开没开 Immich；运营商、Wi-Fi、中继看到 SNI 就知道在用什么应用。证书仍用通配符，具体标签不进 CT 日志，也就不会被批量列出。详见“外人能不能列出装了哪些插件”。

### 入口的开关

每个声明了 `clients` 的插件在 App 里有开关：**局域网入口**、**公网入口**（HTTPS），以及每个 `clients.tcp` 一个。**全部默认关，只有主人能开（App 经隧道），Bot 和 MCP 总线没有这些工具。** 公网入口还受设置里的总开关管（ADR-0090）。 `credentials` 模式下，第一次 `issue` 成功之前不能打开；`app` 模式下，`setup_done` 通过之前不能打开（ADR-0022）。

**`network.clients` 和 `network.lan`（ADR-0019）为什么权限等级不同。** `network.lan` 在安装时就把端口开到局域网，所以算高权限。`network.clients` 只是声明“有这个能力”，安装后什么都没暴露，要主人再开一次开关才有效；打开开关这一步单独醒目确认，相当于高权限确认。所以 `network.clients` 不算高权限，“帮我装 Immich”时卡片上不用醒目提示。局域网入口也要开关，而不是装好就开：不然局域网里任何设备装完就能碰到应用的登录页，这和 `network.lan` 的暴露是同一类。这个权限要列进规范的权限表。

### 中继

**用 [frp](https://github.com/fatedier/frp)（Apache-2.0）的 `type = "https"` 和 `type = "tcp"`。** 比较（2026-11-03 读各仓库默认分支，frp 实测）：

| 方案 | 许可证 | SNI 转发不解密 | 真实来源 IP | 按入口限速 | 名字归属校验 | 家到中继 | 结论 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| frp | Apache-2.0 | 有 | PROXY v2 | 有，服务端模式 | server plugin | TCP、KCP、QUIC、WebSocket，多路复用 | **选它** |
| [sish](https://github.com/antoniomika/sish) | MIT | 有（`sni-proxy`） | PROXY protocol | 配置里没找到 | 只有鉴权钩子（`authentication-key-request-url`） | SSH | 备选。家里只要 SSH 客户端，但缺限速和名字归属，大流量全走 SSH |
| [rathole](https://github.com/rathole-org/rathole) | Apache-2.0 | 无，按端口分服务 | 无 | 无 | 无 | TCP、Noise、TLS | 撑不起多个家的 HTTPS |
| cloudflared | Apache-2.0 | 边缘解密 | HTTP 头 | Cloudflare 决定 | Cloudflare 决定 | QUIC、HTTP/2 | 服务端不开源。可以作为主人换的入口 |
| [SniTun](https://github.com/NabuCasa/snitun)（[Nabu Casa 在用](https://support.nabucasa.com/hc/en-us/articles/25619268678557-Remote-access-Deep-dive)） | GPL-3.0 | 有 | 没核实 | 没核实 | 自己的 Session Master 发令牌 | 自有多路复用 | 最接近的先例。家里要带 Python，用的人少 |
| bore、chisel | MIT | 无 | 无 | 无 | 无 | TCP、SSH | 只转端口 |
| iroh | Apache-2.0 | 公网一侧没有 | | | | QUIC | 见下 |

**iroh 能不能承载。** 中继和核心之间可以走 iroh：`wonton-tunnel` 已经是“一条 TCP 连接一条流”，身份就是 endpoint 的钥匙，QUIC 也没有 TCP 套 TCP 的问题。但 iroh 只解决家到中继这一段，公网一侧的 SNI 路由、名字归属、PROXY protocol、限速都要我们自己写，等于重写 frps；主人自己的 VPS 上也只能跑我们的私有程序。“接口开放”和“不造轮子”都指向 frp。

**frps 的配置：**

- **端口。** 控制和 HTTPS 共用 TCP 443（`bindPort = vhostHTTPSPort = 443`，`transport.tls.force = true`），只开放这一个固定端口，家里的防火墙最容易放行。共用时 frpc 必须设 `transport.tls.disableCustomTLSFirstByte = false`（实测，不设连不上）：控制连接第一个字节是 `0x17` 而不是标准 TLS 的 `0x16`，比标准 TLS 容易被识别。国内实测被干扰时改成独立的控制端口。普通 TCP 用 `allowPorts` 的端口池。换 QUIC 时加 `quicBindPort = 443`（UDP）。
- **frpc 要校验 frps 的证书**（`transport.tls.serverName`、`trustedCaFile`）。frp 默认不校验，路上的人能拿到登录令牌。
- **关掉的功能。** 不开 80，不开 SSH 网关；`webServer` 只绑 `127.0.0.1`，配密码，只为读 Prometheus 指标。frp 2026 年的两个公告（[SSH 网关的 DoS](https://github.com/fatedier/frp/security/advisories/GHSA-26gq-p25f-99cp)、[routeByHTTPUser 的绕过](https://github.com/fatedier/frp/security/advisories/GHSA-pq96-pwvg-vrr9)）都不在这个范围里。
- **心跳。** frps 必须显式设 `transport.heartbeatTimeout = 90`：开着 tcpMux 时默认是 -1，不校验心跳。这样不发心跳的 frpc 到时间就被断开（实测）。核心生成的 frpc 配置也必须显式设 `transport.heartbeatInterval = 30`、`transport.heartbeatTimeout = 90`：frpc 开着 tcpMux 时这两个默认也是 -1（`pkg/config/v1/client.go`，v0.71.1），不设的话每 90 秒所有控制连接都被断开，`Ping` 钩子也不会触发。下面说的“一个心跳”就是这 30 秒。
- **frpc 和中继之间**先用 TCP + TLS，能穿过几乎所有网络。一条 TCP 上多路复用，丢包时所有连接一起慢；QUIC 是一行配置的事，在国内家宽上量过再定。

**frps 的插件钩子（实现在家名服务）：**

frp 的 `user` 是 frpc 自己填的，所以每个钩子都从 `user.metas` 取令牌（`Login` 从 `metas`），和 `user` 这个家当前的哈希比对，不一致或者令牌被轮换、吊销了就拒绝。这样 `user` 就和令牌绑定了，一个家不能用别家的 `user` 逃掉配额或者把流量算到别家头上，`<家>` 也就是 `user`，指标和配额都按它算。

| 钩子 | 做什么 |
| --- | --- |
| `Login` | 比对令牌；这个家停用时拒绝 |
| `NewProxy` | 比对令牌。`https` 只接受 `customDomains` 里**一个具体名字**，形如 `<标签>.<家>.<域名>`，属于这个家，标签不是 `lan`。不接受通配符和 `subdomain`。`tcp` 只接受分给这个家的 `remote_port`。改写 `bandwidth_limit`，模式 `server`，frpc 的配置不可信 |
| `NewUserConn` | 比对令牌；这个家停用、超流量、超并发时拒绝 |
| `Ping` | 比对令牌；这个家停用、超流量时拒绝。frpc 收到后断开控制连接，已有的连接也断（实测） |

- **没开公网入口的插件，流量停在中继。** 主人打开开关时，核心写 frpc 的配置，再 `docker exec` 执行 `frpc reload`（实测），向 frps 登记这个具体名字；关掉时撤销。开关在服务端就生效了，插件的端口没人碰得到。
- **停用、超配额和令牌轮换：配合的 frpc 一个心跳内断开，否则 90 秒内。** 新连接由 `NewUserConn` 立即拒绝（实测），流量当时就到不了它。已有连接随下一次 `Ping` 断开（实测）。不配合的 frpc（比如攻击者手里用旧令牌的）：`Ping` 被拒后 frps 只回一个带错误的 `Pong`，不刷新 `lastPing`，要等 `heartbeatTimeout` 才断开控制连接和已有的连接（实测不发心跳的 frpc），抢去的名字最多再占 90 秒。
- **配额的数据来源。** 中继主机上一个小进程每 30 秒读本机的 `/metrics`（`frp_server_traffic_in/out`、`frp_server_connection_counts`，按 `<user>.<proxy>` 分开，实测），算增量（frps 重启会清零）报给家名服务。
- **限额（主人选的保守档）：每个入口 5 MB/s，每家最多 5 个入口，每月 100 GB，并发连接 100。**
  - frp 的限速按 proxy 算，不按家，所以“按家限速”落地成每入口上限乘入口数，一家最多 25 MB/s。限速写进 `bandwidth_limit = "5MB"`，模式 `server`。
  - 入口数算开着的公网入口，HTTPS 和 TCP 都算；局域网入口不经中继，入口自检的探测标签也不算。第 6 个由 `NewProxy` 拒绝，App 显示 `quota_exceeded`。
  - 月流量是这个家所有入口 `traffic_in` 和 `traffic_out` 的和，经过中继的每个字节算一次，按 UTC 的自然月清零。超了以后这个家的公网入口都停到下个月初，App 写明恢复时间；局域网入口和 Wonton App 不受影响。
- **按家的并发连接上限**用 `connection_counts` 求和判断，滞后最多一个上报周期。

### 普通 TCP 转发（`clients.tcp`）

给非 TLS 协议（Git over SSH）和自己带 TLS 的协议（Syncthing）用。**每个入口在中继上一个独立的公网端口，没有 SNI。**

- **分配。** 主人打开开关时，核心用 endpoint 签名向家名服务申请，从端口池随机分一个（不按顺序，不能猜出相邻的家）。分到后固定，关掉开关端口还留着，这样第三方 App 里的配置下次还能用。
- **开关。** 打开时核心在 frps 上登记 proxy，加上入口容器的 TCP 前端，监听自己的转发端口。关掉时三样都撤掉，并断开已有的连接（字节都经核心转，由核心断）。只撤 frps 上的 proxy 不够：`NewProxy` 只查端口归不归这个家，被攻破的 frpc 能重新登记留着的端口；路由器转发模式也没有 frps。
- **配额。** 端口池 20000–29999，每家最多持有 3 个，超了回 `port_quota_exceeded`，App 如实显示。
- **回收。** 主人在 App 里选“释放”、卸载插件、注销家时释放。释放的端口隔离 180 天才能再分给别的家：否则旧家里的 Git 客户端可能把凭据发给新家。
- **校验。** `NewProxy` 查分配表，端口不是这个家的就拒绝（实测）。frps 的 `allowPorts` 限定端口池，`maxPortsPerClient` 是第二道。
- **路径。** frpc 把连接交给入口容器的一个 TCP 前端（`expect-proxy`，只认连接器的地址），入口容器按来源限新连接速率，再转给核心在 `wonton-clients` 上的端口，核心只转字节给插件。入口容器不进插件网络（ADR-0011，只有核心进）。插件看到的来源是核心，应用自己按 IP 封禁不起作用。
- **中继侧的防滥用。** 主机防火墙（nftables）对端口池按来源限新连接速率；每个端口一个带宽上限；流量算进家的月度配额；停用和超配额同样走 `NewUserConn` 和 `Ping`。
- **审计。** 每条连接：来源、端口、字节数、时长。
- **只有 TCP，只有公网。** UDP 不做（Syncthing 的 QUIC 用不了）。局域网里的非 HTTP 协议继续用 `lan.ports`（ADR-0019）。
- **清单规则。** `clients.tcp` 不能是 `webui` 或 `mcp` 的端口：`sso: proxy` 信任网关加的请求头，TCP 入口原样转发，伪造得了。官方索引收录时审核这个端口默认必须要验证。

### 家名服务和 DNS

自研的只有两块：**注册和管理接口**、**frps 插件**。其余都用现成的。

**DNS 怎么选。** 要解决两件事：DNS-01 的 TXT，和局域网地址的 A 记录。

| 方案 | TXT | 局域网 A 记录 | 权限范围 | 结论 |
| --- | --- | --- | --- | --- |
| [acme-dns](https://github.com/acme-dns/acme-dns)（MIT），lego 的 `acme-dns` provider | 能，每个账号只能写自己的（实测，别的账号写回 401）；保留最新两个值 | 不能，只有 TXT | 主 zone 只要一条固定 CNAME，之后不用写主 zone | **TXT 用它** |
| PowerDNS 的 RFC 2136 + 按 zone 的 TSIG，lego 的 `rfc2136` provider | 能 | 能，**但不能校验地址** | `lua-dnsupdate-policy-script` 只看得到名字和类型（[`UpdatePolicyQuery`](https://github.com/PowerDNS/pdns/blob/master/pdns/lua-auth4.hh) 没有记录内容），令牌能把 `*.lan` 指向任何公网 IP，等于一个能签证书的免费动态域名。主 DNS 还要对公网开更新接口 | 不用 |
| 主 zone 的 API 密钥放家里 | 能 | 能 | 整个 zone | 不用 |
| `iroh-dns-server` | 名字必须是 z32 公钥，会把 endpoint ID 写进 CT；没有通配符（`node_zone_handler.rs`） | | | 不用 |

所以：**TXT 走 acme-dns，局域网 A 记录走家名服务的一个接口（只认 endpoint 签名，只接受 RFC 1918 地址）。** 权威 DNS 用 [PowerDNS Authoritative](https://github.com/PowerDNS/pdns)（GPL-2.0，我们运行的独立进程，不分发）；API 密钥只给家名服务。拆开部署后它作隐藏主服务器，不对公网开放；单机时它经 dnsdist 回答公网查询，见“部署和被攻破的影响”。

**记录：**

| 名字 | 类型 | 值 | 谁能改 |
| --- | --- | --- | --- |
| `*.<家>.<域名>` | A/AAAA | 中继 | 家名服务，注册时写 |
| `*.lan.<家>.<域名>` | A | 家的局域网地址，TTL 60 秒 | 家名服务，核心用 endpoint 签名请求 |
| `_acme-challenge.<家>.<域名>`、`_acme-challenge.lan.<家>.<域名>` | CNAME | 这个家的两个 acme-dns 子域名，TTL 60 秒 | 家名服务，注册时写，轮换 acme-dns 凭据时改，注销时删 |
| `<家>.<域名>` | CAA | `letsencrypt.org; validationmethods=dns-01; accounturi=<这个家的账号>` | 家名服务，核心用 endpoint 签名请求，有冷却期 |

DNS 对任何标签都回答同样的结果（通配符），查不出哪个标签存在。

**三种凭据，分开放：**

| 凭据 | 给谁 | 只能做什么 | 泄露的后果 |
| --- | --- | --- | --- |
| endpoint 私钥 | 只在核心 | 管理接口：CAA、局域网地址、端口、轮换吊销令牌、注销。也是隧道身份 | 整个家的身份都丢了，先于这篇就成立 |
| frp 登录令牌 | frpc | 以这个家登录 frps，登记这个家的名字和端口 | **能劫持路由**：把这个家的流量（包括没开的标签）接到自己那里，看得到 SNI 和来源 IP。没有有效证书时手机上是证书错误，看不到内容 |
| acme-dns 凭据（两组） | lego | 写这个家的 `_acme-challenge` TXT | **没有 CAA 时能签这个家的证书**。加上 frp 令牌就是完整的中间人。CAA 生效后签不了，因为账号不对 |

令牌泄露的后果和 CAA 的账号绑定是一起设计的：两个令牌同时泄露，只要 CAA 在，攻击者还是拿不到证书。入口容器被攻破时证书私钥会泄露，这时要换证书，见“迁移和回退”。

**接口。** 除了注册，都要 endpoint 私钥签名（带时间戳和随机数，防重放），和推送中继同一个防滥用模型（ADR-0012）。没有用户账号。

| 接口 | 做什么 |
| --- | --- |
| `POST /homes` | 注册：邀请码 + endpoint ID + 签名。发 `<家>`、frp 令牌、acme-dns 凭据，写 DNS 记录 |
| `GET /homes/{家}` | 这个家的状态：记录、待生效的 CAA 变更、配额和用量、是否停用。核心每小时查一次，有变化就通知主人 |
| `PUT /homes/{家}/caa` | 设置 `accounturi`。第一次立即生效；之后冷却 24 小时，到期前旧值仍有效，可以撤销 |
| `PUT /homes/{家}/lan` | 设置局域网地址，只接受 RFC 1918 |
| `POST /homes/{家}/ports`、`DELETE /homes/{家}/ports/{端口}` | 申请、释放 TCP 端口 |
| `POST /homes/{家}/tokens/{frp 或 acme}` | 轮换（发新值。frp 令牌的旧值立即失效，acme-dns 的旧账号在 CNAME 的 TTL 过后失效） |
| `DELETE /homes/{家}/tokens/{frp 或 acme}` | 吊销，不发新值 |
| `DELETE /homes/{家}` | 注销 |

**轮换 acme-dns 凭据就是换账号。** acme-dns 不能给已有账号换密钥，也不能删账号。轮换时家名服务注册两个新账号，改指两条 CNAME，把新凭据发给核心。旧账号留着不删：CNAME 改走以后，它的 TXT 不再影响这个家。吊销就是删掉这两条 CNAME，下次要签发前先轮换。

**轮换 frp 令牌要重建连接器。** 令牌在 frpc 全局的 `metadatas` 里，而 `frpc reload` [不改全局参数](https://github.com/fatedier/frp#hot-reloading-frpc-configuration)。只 reload 的话，frpc 带着旧令牌重连，被 `Login` 拒绝，这个家的公网入口都断掉。所以核心写好新配置后重建连接器容器，进行中的公网连接会断开（实测重建后恢复）。

**第一次的顺序（不能乱）。** 主人在 App 里填邀请码时开始（`POST /_wonton/clients/home`），没注册时打开入口回 `not_registered`：

1. 注册，拿到 `<家>` 和两种令牌。
2. lego 注册 ACME 账号（钥匙留在家里），得到账号 URI。
3. `PUT caa`。核心查权威 DNS，确认 CAA 生效。
4. lego 签发证书，写进证书目录。
5. 开关才可用。

步骤 1 到 3 之间这个家没有 CAA，也没有证书和路由，泄露的令牌能签一张证书，但核心的 CT 监控看得见。

**丢了 ACME 账号钥匙。** 钥匙和证书都在核心的数据里，跟备份走，正常从备份恢复。只丢了钥匙时：lego 注册新账号，`PUT caa` 走冷却期；期间旧证书继续用；冷却期到了再签。如果证书也过期了，HTTPS 如实失败，App 写明什么时候恢复。连 endpoint 私钥也丢了，就是一个新的家，第三方 App 要重填。

### 部署和被攻破的影响

**我们的服务被攻破是要认真对待的威胁**：PowerDNS 的 API 密钥能改整个 zone。

**云端先全部放在一台机器上（主人定），放在香港，只在邀请制阶段成立。** 域名、注册商和 NS 的名字部署时定。

**代价：这一台被攻破（拿到宿主 root），所有家都能被代签证书、被中间人攻击。** 攻击者能改任何家的路由和 CAA，再写 TXT 签新证书，CT 监控只能事后告警，主人只能用自己的域名躲开。这一台挂了，所有家的第三方 App 在外都连不上；权威 DNS 也只有这一台，局域网名字在缓存过期（TTL 60 秒）后也解析不了。局域网直连本来就依赖权威 DNS 和家里的外网。Wonton App 不受影响。

**单机上做进程和网络隔离。** 防的是单个服务被攻破，挡不住宿主 root。所有容器都 `--cap-drop ALL`，所有内部网络都设 `gateway_mode_ipv4` 和 `gateway_mode_ipv6` 为 `isolated`：

| 服务 | 怎么跑 | 公网开放 | 这个服务被攻破时 |
| --- | --- | --- | --- |
| frps | 容器，host 网络（保住 IPv6 的真实来源，也不用发布上万个端口），非 root，根文件系统只读，监听高端口，主机的 nftables 把 443 重定向过来 | 443，端口池 | 能断流量、看 SNI 和来源 IP。没有证书，看不到内容；连不到内部网络，碰不到 DNS（实测）。它和每个家的 frpc 讲 frp 协议，frpc 又连得上它所在的宿主 |
| 指标上报 | 同上，host 网络 | 无 | 能谎报用量 |
| dnsdist | 容器，非 root，根文件系统只读，连公网和内部网络 `dns`，按名字分发 53（实测），并按来源限速 | 53 | **和家名服务同级**：它是唯一对公网回答 53 的进程，Let's Encrypt 查 CAA 和 `_acme-challenge` 都由它回答。能隐去 CAA、伪造 TXT，给任何家签证书，CAA 不起作用，只有 CT 监控事后发现 |
| PowerDNS | 容器。DNS 在内部网络 `dns`，API 绑在另一个只有它和家名服务的内部网络 `dns-api`，`webserver-allow-from` 只允许家名服务的固定地址（实测） | 无 | 能改所有记录，和家名服务被攻破一样 |
| acme-dns | 容器，只在内部网络 `dns` | 无 | 能写所有家的 TXT，但 CAA 挡住了签发；连不到 PowerDNS 的 API（实测） |
| 家名服务 | 容器，非 root，连公网、`dns`（调 acme-dns）和 `dns-api`，持有 PowerDNS 的 API 密钥 | 8443（只接受签名和邀请码，另外只转发 acme-dns 的 `/update`，`/register` 不对外）。frps 插件的端口只绑 `127.0.0.1` | **最严重**：能改任何家的路由和 CAA，能代签证书 |

- **PowerDNS 的 API 不绑宿主回环，也不和 DNS 放在同一个网络。** frps 用 host 网络，和宿主共用回环，普通内部网络在宿主一侧也有网关地址，它都够得着（实测）。dnsdist 和 acme-dns 直接解析公网来的报文，默认的 NET_RAW 能力可以做 ARP 欺骗，同一个网络上明文的 `X-API-Key` 就保不住。
- **frps 插件的端口和管理接口分开。** 插件端口只回答“放不放行”，写不了 DNS。
- **dnsdist 是因为只有一个公网 IP。** PowerDNS 和 acme-dns 都要回答公网 53，一个答主 zone，一个答 `auth.<域名>`（实测）。拆开后不再需要它分发。
- **443 归 frps，所以家名服务用 8443。** 拆开后可以回到 443。
- **这台机器上不跑别的服务。** Docker socket 只有 root 能碰。
- **家名服务的数据库和 zone 每天加密备份到别处。** 这台机器丢了，不能丢 `<家>` 的墓碑表和端口的分配表，否则永不复用的规则就破了。

**公开注册之前必须拆开**（前置条件见“防滥用和 PSL”），至少：中继和持有 DNS 密钥的服务不同机；权威 NS 两台以上，放在不同的主机或服务商，和中继分开，公网 53 不再只由 dnsdist 这一个进程回答。完整拆开时：

| 主机 | 跑什么 | 公网开放 | 被攻破时 |
| --- | --- | --- | --- |
| 中继 | frps、指标上报 | 443，端口池 | 能断流量、看 SNI 和来源 IP。没有证书，看不到内容；碰不到 DNS。能经 frp 协议攻击每个家的 frpc |
| 家名服务 | 家名服务、PowerDNS 隐藏主服务器 | 只有家名服务的 HTTPS（只接受签名和邀请码）。PowerDNS 的 API 只绑本机，AXFR 只给辅助 NS | **最严重**：能改任何家的路由和 CAA，能代签证书。只有 CT 日志事后看得到，主人可以换自己的域名躲开这一条 |
| acme-dns | acme-dns | 53（只有 `auth.<域名>`），API | 能写所有家的 TXT，但 CAA 挡住了签发 |
| 权威 NS，至少两台 | PowerDNS 或 NSD，辅助 | 53 | 能答错 DNS，可以把人引到错的 IP，CAA 也能被答错。不同主机和服务商，单台被攻破时 Let's Encrypt 的多视角验证不一定过 |

- **frps 和家名服务不同机。** frps 调插件走私有链路（WireGuard）。frps 的主机上没有 DNS 权限。
- **权威 NS 和中继分开。** 中继被打挂时，名字还能解析。

不管部署成几台：

- **CAA 的变更只接受 endpoint 签名。** 家名服务被攻破时这条防不住，防的是令牌泄露和接口被滥用。
- **令牌只存哈希。** frpc 经 `metadatas` 把 frp 令牌带给 `Login` 钩子（实测），家名服务比对哈希。
- **日志保留。** 中继和家名服务的连接日志保留 7 天（排查和滥用处理够用），不外传。月度用量按家保留 13 个月。没有内容，也不记录 SNI 之外的请求信息。

### 证书

**一张 Let's Encrypt 的通配符证书，覆盖 `*.<家>.<域名>` 和 `*.lan.<家>.<域名>`，用 DNS-01。** lego + acme-dns 的整条路径在 Pebble 上实测过（见[验证结果](0021-verification.md)）。

- **为什么 DNS-01。** 通配符只能用 DNS-01。HTTP-01 和 TLS-ALPN-01 要逐个名字申请，每个插件的名字都会进证书透明日志，HTTP-01 还要开 80。
- **私钥和账号的钥匙都在家里**，0600，插件的容器访问不到，也没有导出接口。中继、家名服务都拿不到。
- **手机上不用装任何东西。** 这也绕开了 Android 7 起 App 不认用户 CA、Dart 的 `HttpClient` 只读系统证书目录的问题。
- **证书目录是接口。** 核心生成钥匙和 CSR，`lego run --csr` 只拿到 CSR，写出证书链；核心把私钥和证书链合成固定文件名 `site.pem`，向入口容器发 `USR2`（实测不断已有连接）。主人用 certbot 或自己的证书也行。
- **续期按 [ARI](https://letsencrypt.org/docs/rate-limits/) 走**（lego 默认）。证书的有效期会变短：Let's Encrypt 的默认证书 [2027-02-10 起 64 天，2028-02-16 起 45 天](https://letsencrypt.org/upcoming-features/)，每天检查、按 ARI 决定，不用改。
- **失败即停。** 续期失败时继续用旧证书，App 显示到期时间和 lego 的错误；过期后连接如实失败，不换自签名，不降级到 `http`。

**Let's Encrypt 的[限额](https://letsencrypt.org/docs/rate-limits/)（2026-11-03）：**

| 限额 | 数值 | 怎么办 |
| --- | --- | --- |
| 每个注册域名的新证书 | 7 天 50 张，全局，可以申请提额 | 邀请制下够用。扩大前申请提额；进了 PSL 后按家算 |
| 每个账号的新订单 | 3 小时 300 个 | 每个家一个账号，碰不到 |
| 完全相同名字集合的证书 | 7 天 5 张，不能提额 | 证书跟核心数据一起备份。碰到时 App 写明什么时候能重试 |
| 每个名字的验证失败 | 每小时 5 次，不能提额 | lego 失败后退避；测试用 staging |
| ARI 续期 | 不受以上限制 | |

**dns-persist-01** 能让家只写一次记录，但 Let's Encrypt [在草案的账号替换问题解决前不部署](https://community.letsencrypt.org/t/dns-persist-01-deployment-status-and-timeline/246468)（2026-06 的公告，2026-11-03 没查到上线的消息）。不依赖它，上线后再评估。

### CT 日志监控

我们控制 DNS，所以**理论上能给任何家另签一张证书再做中间人**，“不解密”只对被动的中继成立。这和 Nabu Casa、[Tailscale](https://tailscale.com/kb/1153/enabling-https) 是同一个信任模型，主人接受并写明。缓解：

- CAA 的 `accounturi`（RFC 8657，[Let's Encrypt 的文档](https://letsencrypt.org/docs/caa/)写了账号 URI 的形式，实际效果见[待验证](0021-verification.md#待验证)）。
- **核心定期查证书透明日志，发现不是自己申请的证书就告警。** 用第三方的 [SSLMate CT Search API](https://sslmate.com/ct_search_api/) 的免费额度（够不够见[待验证](0021-verification.md#待验证)），每 6 小时查一次 `<家>.<域名>` 和子域名。比对公钥，不比对证书指纹：Let's Encrypt 记的是预证书，指纹和最终证书不同。用它返回的 `pubkey_sha256`，和核心记下的钥匙比对。钥匙由核心生成，下单前就记下了公钥，所以 lego 中途失败也不会误报。必须是第三方，因为防的就是我们自己。
- **监控不可用时如实显示**“监控连不上”，不显示为安全。出现陌生证书时：App 醒目告警，写审计，给出 CAA 和注销的操作。这是事后发现，挡不住签发。
- 不信任我们的主人用自己的域名，这个问题就没有了。

### 换入口

核心对入口只有这几个接口，不认识 Let's Encrypt 或任何具体域名。主人在 App 的设置里选入口和 DNS 服务商（ADR-0089、ADR-0090），下表的环境变量是装机时的写法：

| 接口 | 内容 |
| --- | --- |
| `WONTON_CLIENT_DOMAIN` | 地址的后缀，如 `<家>.<域名>`。名字就是 `<标签>.<后缀>` 和 `<标签>.lan.<后缀>` |
| `WONTON_CLIENT_SERVICE` | 家名服务的地址。默认值只写在 `deploy/install/compose.yaml` 里（`${WONTON_CLIENT_SERVICE:-…}`），不进代码。要改就写进 `.env`。设了 `WONTON_CLIENT_DOMAIN` 就不用它 |
| 入口容器的 `wonton-edge` | 连接器把 TLS 连接送到这里。真实来源由 `WONTON_CLIENT_TRUST` 决定：`proxy-v2`（默认）、`header:<名字>` |
| 局域网端口（`WONTON_CLIENT_PORT`，默认 8483） | 局域网转发发布，里面是 TLS |
| 证书目录 | `site.pem` |

`WONTON_CLIENT_DOMAIN`、`SERVICE`、`PORT`、`TRUST` 都在 `compose.yaml` 里核心的 `environment` 下逐个写上，`.env` 不会自动进容器。

| 主人想要 | 怎么做 | 私钥只在家里 | 说明 |
| --- | --- | --- | --- |
| 默认 | frpc 连我们的中继，lego 走 acme-dns | 是 | 运营者理论上能代签 |
| 自己的域名，自己的 VPS | VPS 上跑原版 frps，lego 用 DNS 服务商的 API（lego 支持 200 多家，密钥存在家里） | 是 | 不依赖我们的任何服务 |
| 自己的域名，路由器转发 443 | 把连接器换成和局域网转发一样的 TCP 转发容器：它接替连接器的角色，用连接器在 `wonton-edge` 上的固定地址，带 PROXY v2，不做来源过滤；发布在一个关掉地址伪装的桥接网络上，只发布 IPv4 并指定 `HostIp` | 是 | 要有公网 IP。TCP 入口要自己转发。接公网连接的这个容器连得上宿主，它只转字节。要发布 IPv6 时先关掉 Docker 的 `userland-proxy`，否则 IPv6 客户端都显示为网桥网关，共用一个限速和封禁的桶 |
| 自己的域名，Cloudflare Tunnel | 把连接器换成 cloudflared，`originRequest` 打开 [`matchSNItoHost`](https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/configure-tunnels/cloudflared-parameters/origin-parameters/)，`WONTON_CLIENT_TRUST=header:CF-Connecting-IP` | **否** | 支持，三条代价见下 |

**自己的域名时，`*.lan.<后缀>` 的 A 记录由主人提供。** lego 的 provider 只写 TXT，核心不写这条记录。打开局域网入口和地址变化时，核心查一次这个名字，解析结果不是选中的地址就回 `lan_unavailable`（开着的入口标成这个状态），写明要加什么记录。核心随地址检测定时重查（打开地址页时当场查），解析对了就恢复。

**默认值怎么到已经装好的主机上。** `compose.yaml` 由 `wonton-core init` 写，安装脚本和一键升级都执行新镜像的 `init`（`deploy/install/README.md`），所以升级后就有默认值，`.env` 不用多一个键。`.env` 只放主人改过的：`install.sh` 按固定的键重写 `.env`，要改成保留 `WONTON_CLIENT_*`，否则重跑安装脚本修复时，主人自己的域名会静默变回默认中继。

**Cloudflare Tunnel 的代价，App 选它时写明：**

1. **在边缘解开 TLS。** 手机看到的是 Cloudflare 的证书，Cloudflare 看得到内容，包括密码（[第三方分析](https://ssdnodes.com/learn/cloudflare-tunnel-no-open-ports)，这一点没有查到官方原文）。
2. **免费计划的请求体[上限 100 MB](https://developers.cloudflare.com/workers/platform/limits)**，Immich 传大视频会失败。
3. **[国内不稳定](https://developers.cloudflare.com/china-network/)。**

另外，普通 TCP 经 Cloudflare 要客户端也跑 cloudflared，所以这种入口下 `clients.tcp` 不可用。

### 局域网直连

**用真实的域名指向局域网地址，配公共证书，和 Plex 的 `plex.direct` 同一个思路，名字固定。** `https://<标签>.lan.<家>.<域名>:8483` 解析到家的 RFC 1918 地址，核心更新 A 记录。没选 Plex 的“IP 写进名字”：DHCP 换地址（ADR-0019：主机从 `.14` 变成 `.10`）时第三方 App 都要重填。

**端口只绑检测到的局域网地址（启动器声明了宣告地址时绑 `0.0.0.0`，见 ADR-0019），不按来源过滤当边界。** 静态 compose 的 `ports` 会监听 `0.0.0.0` 和 `::`，经 docker-proxy 进来的 IPv6 连接来源是网桥网关，正好在 RFC 1918 里（ADR-0019 实测），所以单靠来源过滤，公网 IPv6 能直连局域网入口，还绕过公网入口的开关。

- **按 ADR-0019 的办法，发布时填 `HostIp`，只发布 IPv4，不发布到 `0.0.0.0` 或 `::`。** 这个端口不写在核心的 compose 里，局域网转发归核心管，核心填好地址再创建。发布在一个关掉地址伪装的桥接网络 `wonton-lanpub` 上，局域网转发只绑它在这个网络上的地址，不能经它出网。
- **地址变化时核心重建局域网转发。** 只断开局域网上正在用的连接，入口容器和公网连接不动。
- **来源过滤是第二道，在局域网转发上：** 只放行选中地址所在的网段（检测结果带上前缀长度），**网桥网关地址不算**（实测被拒绝）。别的 VLAN 和访客网段连不上，走公网地址。这也挡住被攻破的连接器：它能经路由连到局域网转发，但来源不在这个网段（实测）。
- **PROXY v2 按前端只认一个对端。** 公网前端绑在 `wonton-edge` 上，只认连接器；局域网前端绑在 `wonton-lanfwd` 上，只认局域网转发。校验对端地址后才 `expect-proxy`，其余连接不解析 PROXY 头，直接断开（实测）。发布出去的 8483 不解析 PROXY。
- **要有测试：** 从公网 IPv6 和 Docker 网关连 8483 都被拒绝；伪造 PROXY 头的连接被拒绝；从连接器连局域网前端或局域网转发被拒绝。

**地址（和 ADR-0019 的关系）：**

- **检测的触发**见 ADR-0019。
- **选了多个地址时**，DNS 和端口都只用一个：按 ADR-0019 的结果（启动器声明了宣告地址时看 `announce_addresses`，`addresses` 这时是通配地址），默认路由的地址优先，否则按设置里的顺序，取第一个 RFC 1918 的 IPv4。
- **选中的地址不是 RFC 1918 时**（100.64/10、fc00::/7，或 `WONTON_LAN_ADDRESSES` 指定的这类地址）拒绝打开局域网入口，地址页写明原因（错误类别 `lan_unavailable`）。公网入口不受影响。
- **地址集合一致。** 家名服务 `PUT lan` 接受的和能打开局域网入口的都是 RFC 1918 地址（宣告地址同样只收私有地址），8483 放行这个地址所在的网段。

- **`*.lan` 的公网解析会泄露家的私有地址。** 只有 `192.0.2.10` 这种信息，外人用不上。
- **iOS 第三方 App** 第一次连局域网地址时，系统会问“本地网络”权限，拒绝就连不上（[TN3179](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy)）。
- **第三方 App 里只能填一个地址的，主人二选一。** 公网地址在家也能用，只是绕一圈中继。Immich 有[按 Wi-Fi 自动切换地址](https://github.com/immich-app/immich/pull/14437)的设置（要定位权限，[有过切换失败的报告](https://github.com/immich-app/immich/issues/21930)），能两个都填。

**DNS 重绑定保护。** 路由器或 DNS 可能丢弃“公网名字解析到内网地址”的回答：OpenWrt 的 dnsmasq [默认开着](https://openwrt.org/docs/guide-user/base-system/dhcp)，Unbound 有 `private-address`。Plex 的 `plex.direct` [也撞这个问题](https://words.filippo.io/how-plex-is-doing-https-for-all-its-users/)，它的 [Preferred 模式](https://support.plex.tv/articles/206225077-how-to-use-secure-server-connections/)会退回不安全连接。我们取它的“必须安全”模式：连不上就是连不上。

**Wonton App 在“地址”页面自己测一次，** 在手机上用系统解析器查局域网名字，再连上去验证证书，分清原因：

| 现象 | App 显示 | 主人能做的 |
| --- | --- | --- |
| 解析为空、NXDOMAIN 或 `0.0.0.0` | 路由器或 DNS 挡了内网地址 | 把 `<家>.<域名>` 加白名单（dnsmasq `rebind-domain-ok=/…/`，OpenWrt `list rebind_domain`，Unbound `private-domain`），或者用公网地址 |
| 解析对，连不上 | 不在家、访客 Wi-Fi、无线隔离、或地址刚变 | 换网络；iOS 上检查本地网络权限 |
| 解析到别的地址 | 地址不对，可能是 DNS 被改写 | 用公网地址 |
| 证书不对 | 如实显示，不继续 | 查证书状态 |
| 都通过 | 能用 | |

不给 `http`，不给 IP 地址，不自动改用别的路径。第三方 App 自己的错误提示我们管不了，所以 App 把这个结果写在地址旁边。

### 清单怎么声明

新字段 `clients`，新权限 `network.clients`，规范版本加一（ADR-0022 用了 `"0.3"`，`clients` 不和它一起发布时就是 `"0.4"`）。

```yaml
x-wonton:
  spec: "0.3"
  permissions: [network.clients]
  clients:
    http:                         # 一个插件最多一个
      service: forgejo
      port: 3000
      auth:
        credentials:              # 见“专用密码”
          issue: [sh, /wonton/token.sh, issue, "{name}"]
          generates: true
          ends_sessions: true
          verify: [sh, /wonton/token.sh, verify, "{name}"]
          revoke: [sh, /wonton/token.sh, revoke, "{name}"]
      public:                     # 只作用于公网入口
        deny: [/admin, /user/sign_up]
      auth_failures:              # 见“公网暴露面”
        - { path: /user/login, status: [401, 403] }
    tcp:
      - { service: forgejo, port: 22, name: ssh }
```

- `clients` 和 `network.clients` 要一起写，只写一个时清单不合法。`clients.http`、`clients.tcp` 至少有一项。
- `clients.http` 不能是 `mcp` 的端口，可以是 `webui` 的端口（Immich 的网页和 API 是同一个端口）：转发前删掉 `webui.proxy` 里声明的请求头和 Cookie，`sso: proxy` 伪造不了。删除时按规范化后的名字比较（`_` 换成 `-`，忽略大小写），`gateway` 的模板头也一样，入口容器也用 `option http-restrict-req-hdr-names delete` 删掉名字带 `_` 的请求头（实测）：CGI、WSGI、FastCGI、PHP 和很多 Python 应用把 `X_Remote_User` 和 `X-Remote-User` 映射成同一个变量。这个入口不看设备身份，也没有会话，绝不当作主人。
- 插件按 `X-Forwarded-Host` 得到请求的地址。要固定地址的（往外登记回调、只认一个 `Host`）用核心注入的 `WONTON_EXTERNAL_URL` 和 `single_host`，见 ADR-0089 第 5 节。
- **`public.deny`、`public.allow`（二选一）：公网路径限制，由核心在公网入口上执行。** 按整段前缀匹配，不分大小写：`/share` 只匹配 `/share` 和 `/share/...`。局域网入口不受影响。比如 Immich 的 `/api/auth/admin-sign-up`、Vaultwarden 的 `/admin` 和 `/identity/accounts/register`、Jellyfin 的 `/Startup`（都读过各自的代码；写插件时核实完整列表）。
- **路径怎么匹配（`public` 和 `auth_failures` 一样）。** 应用解析路径的方式各不相同（Kestrel 保留 `%2F` 再去点段，不合并斜杠；Express、chi 按字面路由），所以声明了其中任何一个的入口：
  - 沿用 ADR-0009 的规则并收紧：不解码、解码一次、解码两次的结果，都按 `/` 和 `\` 切段，每段再去掉 `;` 起的部分，只要有 `.` 或 `..` 段，就回 400 `invalid_input`。这样 `%2e%2e`、`..;`，以及要解码后才切得开的 `..%2f`，都到不了应用。规范的客户端发请求前会去掉点段。非法的百分号序列按原样保留，不回 400，也不丢掉这个候选。
  - 没有点段以后，其余的差别（`%2F` 和 `\` 怎么切段、大小写、`;`）按候选算：不解码、解码一次、解码两次，每个都把 `\` 换成 `/`，去掉每段 `;` 起的部分，合并 `//`，转成小写。`deny` 任何一个候选命中就拒绝；`allow` 所有候选都要在列表里，每个候选再算一遍不去掉 `;` 的，按字面处理 `;` 的应用看到的是 `share;x`；`auth_failures` 任何一个候选命中就算。
  - 转发的始终是原始路径，所以 Forgejo 的 `/branches/feature%2Fx`、WebDAV 文件名里的 `%` 和 `;` 都不受影响。解码三次才还原的写法不在防护范围。
- **规范要求：声明 `network.clients` 的插件默认关掉自助注册。** 只能由主人经 App 内的网页建第一个账号。官方索引收录时审核。

| | `lan.ports`（ADR-0019） | `clients`（这篇） |
| --- | --- | --- |
| 解决什么 | 局域网设备用任意协议直连 | 第三方 App 在家在外连 |
| 走哪 | Docker 发布端口，不经核心 | 全部经入口容器和核心，插件容器不发布端口 |
| TLS | 没有 | `http` 入口有 |
| 谁能连 | 局域网里任何设备 | 主人打开后，局域网或公网上任何人，要过应用或网关的验证 |
| 权限 | `network.lan`，高权限，装好就生效 | `network.clients`，不算高权限，主人开开关才生效 |

### 专用密码

`clients.http.auth` 写明由谁验证。三种：

**1. `gateway`：网关验证。** 应用认反向代理传来的用户名，或者根本不需要自己登录（Radicale 的 `http_x_remote_user`）。

```yaml
auth:
  gateway:
    headers:
      X-Remote-User: "{user}"
```

- 网关只认 HTTP 的 `Basic`（用户名是主人的用户名）和 `Bearer`，比对通过后删掉 `Authorization`，按模板加请求头，模板同 `webui.proxy.headers`。没带或不对时回 401 和 `WWW-Authenticate: Basic`，转不到插件。
- 每个客户端一个密码，吊销下一个请求就生效，已经升级的连接（WebSocket）立即断开。
- 只做这两种标准方案。Subsonic 的 `u`、`t`、`s` 这类协议私有的写法不进核心（ADR-0015），插件自己用下一种。

**2. `credentials`：应用验证，核心发密码。** 和模型网关的凭据（插件规范的“凭据的签发与吊销”）同一套约定：核心在插件的容器里执行声明的命令，看退出码。

```yaml
auth:
  credentials:
    issue: [sh, /wonton/password.sh, set, "{name}"]
    verify: [sh, /wonton/password.sh, check, "{name}"]  # 可选
    revoke: [sh, /wonton/password.sh, revoke, "{name}"]  # per_client: true 必须写
    rotate: [sh, /wonton/password.sh, rotate, "{name}"]  # per_client: false 必须写
    ends_sessions: true         # 必须写
    service: sidecar            # 默认是 clients.http.service
    username: "{email}"         # 显示给主人填的用户名，默认 {user}
    per_client: true            # 默认 true
    generates: false            # 默认 false
```

- **`issue` 是覆盖写：** 把值设成标准输入给的值，不结束已有的登录。这和模型网关的 `issue`（已有时不改动）不同，规范分开写。
- **值由谁生成。** 默认核心生成，走 `issue` 的标准输入。`generates: true` 时由应用生成（Forgejo 的访问令牌、Nextcloud 的应用密码都不能指定值），`issue` 不读标准输入，把值写到标准输出的第一行（上限 4 KiB），其余输出不解析。这是相对模型网关约定（“不解析输出”）的扩展，规范要写明。
- **对账。** 插件每次启动后核心对一次账：
  - 有 `verify`（标准输入给值，退出码 0 表示有效）：只验证，无效时再 `issue`。
  - 没有 `verify`：重新 `issue` 同一个值，所以 `issue` 必须可以重复执行，且**不能结束已有的登录**。
  - `generates: true` 必须写 `verify`：重新 `issue` 会换值。`verify` 失败时重新生成，App 提示主人这个值变了。
- `revoke` 退出码 0 就表示旧密码失效了。`ends_sessions: true` 时也要结束它已有的登录。
- `rotate` 把值换成标准输入给的新值（`generates: true` 时写到标准输出）。`ends_sessions: true` 时也结束全部登录，比如 Immich 改密码时带 `invalidateSessions`。
- `ends_sessions` 必须写，不设默认值。写 `false` 的，同时在 `about` 里写明；吊销时 App 提示主人到应用里退出登录，不说“所有客户端要重新登录”。
- **`per_client: false`。** 应用里每个用户只能有一个密码。这时整个插件一个密码，装好时生成。写 `issue`、`rotate`（和可选的 `verify`），不写 `revoke`；吊销就是 `rotate`。
- 命令超时 30 秒。`revoke` 和 `rotate` 失败时，App 如实显示失败和命令的标准错误输出。
- **吊销不能丢。** 脚本可能已经改了密码，只是后面一步出错或超时，所以失败和超时都算结果未知。执行 `rotate` 之前，核心先把新值记成“待换”并落盘（`generates: true` 时只记“待换”，新值以成功时的输出为准）；`revoke` 同样先记“待吊销”。退出码 0 才转正。插件没在运行时也先记下。没完成的隔一段时间重试，插件启动时先重跑，再对账，所以这两个命令都必须能重复执行；对账绝不 `issue` 已被待换值取代的旧值；标着待吊销的条目，对账不 `issue`、不 `verify`、不重新生成，只重试 `revoke`。吊销的接口如实报“待完成”（202），不报已吊销。

**3. `app`：应用自己的账号。** 核心不发密码，App 只给地址，写明“用这个应用自己的账号”。`clients.tcp` 都按这个算。`clients.http` 的 `app` 必须写 `setup_done`，首次注册没完成时入口打不开（ADR-0022 的“和公网入口的关系”）。

| 应用 | 方式 | 说明 |
| --- | --- | --- |
| Immich 手机 App | `credentials`，`per_client: false`，**每个应用一个密码** | 手机 App 只有邮箱加密码（`auth.service.dart`）或 OAuth；`mobile/lib` 里搜不到用 API key 登录的入口 |
| Jellyfin 客户端 | `credentials`，`per_client: false`，**每个应用一个密码** | 客户端用用户名和密码登录 |
| Navidrome（Subsonic 客户端） | `credentials`，`per_client: false`，**每个应用一个密码** | 反向代理的用户名头对 Subsonic [也生效](https://www.navidrome.org/docs/usage/integration/authentication/)，但客户端发的是 Subsonic 自己的令牌，网关验不了 |
| Vaultwarden（Bitwarden） | `app`，**主密码由主人自己设** | 主密码是解开密码库的钥匙，[开了 SSO 也还要它](https://github.com/dani-garcia/vaultwarden/discussions/6678)。核心替主人定，核心就能解开密码库 |
| Radicale（CalDAV、CardDAV） | `gateway`，每个客户端一个 | 认 `X-Remote-User`（[文档](https://radicale.org/v3.html)） |
| Forgejo（Git over HTTPS） | `credentials`，`generates: true`，每个客户端一个 | `admin user generate-access-token`，值由应用生成 |
| Nextcloud | `credentials`，`generates: true`，每个客户端一个 | `occ user:add-app-password`，值由应用生成 |
| Forgejo（Git over SSH）、Syncthing | `clients.tcp` | 靠公钥、设备 ID，协议自己验证 |

每个插件具体怎么接，写插件时核实。

**在 App 里**

- 每个插件一页：地址、开关、用户名、密码，能查看、复制、吊销，`per_client` 的能新建（给客户端起个名字）。复制时标成敏感内容（iOS 只留在本机，Android 用 `EXTRA_IS_SENSITIVE`）。
- **“查看前过 Face ID 或指纹”只在客户端检查。** 被攻破的设备绕得过，所以核心不把它当作安全边界：**每次取值都记审计，具体到设备。**
- 明文存进插件密钥那套存储（同一把密钥库，0600，App 能查看的那套），不另起文件，因为要能查看。值是 128 位随机数的小写 base32，和 ADR-0020 的编号一样长。
- **吊销设备时**，这台设备看过或建过的专用密码一起吊销；**这台设备看过或建过的共享密码（`per_client: false`）默认 `rotate`**（Immich、Jellyfin、Navidrome 都是这样，手机丢了是最常见的吊销原因）。结束不结束已有的登录，按上面的 `ends_sessions`，吊销设备的响应列出哪些应用的登录没结束。主人要保留就明确选“不换”。没被这台设备看过的不动。
- 卸载插件时全部删除，保留数据时除外。
- Bot 和插件都拿不到别人的专用密码，MCP 总线没有相关工具。

### 接口

都在核心地址上，经应用内隧道，只给已配对的设备，类型在 `wonton-protocol` 的 `clients` 模块。理由同 ADR-0020 的鉴权：隧道身份随 WebView 的请求自动带上，所以都要求请求头 `X-Wonton-Clients: 1`（跨站要预检，核心不回 CORS 头）。

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/_wonton/clients` | 每个插件的地址、开关、用户名、密码列表（不含值）、验证方式 |
| POST | `/_wonton/clients/{id}/passwords` | 新建，请求体 `{"label": …}` |
| GET、DELETE | `/_wonton/clients/{id}/passwords/{密码 id}` | 取值（记审计）；吊销（`per_client: false` 时 `rotate`），没完成时回 202 |
| POST | `/_wonton/clients/home` | 注册这个家，请求体 `{"invite": …}`。主人在 App 的入口页填邀请码时调用，只有主人能调，后续步骤的进度看 `status`。设了 `WONTON_CLIENT_DOMAIN` 时不用 |
| PUT | `/_wonton/clients/{id}/entries/{lan、public 或 tcp 名字}` | 开或关这个入口。`credentials` 第一次 `issue` 成功前回 409，没注册时回 `not_registered` |
| DELETE | `/_wonton/clients/{id}/ports/{tcp 名字}` | 释放 TCP 端口 |
| GET | `/_wonton/clients/status` | 注册进度、证书、入口自检、CT 监控、配额和用量、最近的封禁、待生效的 CAA 变更、待完成的吊销 |
| DELETE | `/_wonton/clients/bans/{来源}` | 解除封禁（入口容器的运行时接口，只在 `wonton-clients` 上可达） |
| POST | `/_wonton/clients/home/{rotate-tokens 或 deregister}` | 轮换令牌；注销这个家 |

公网访问自己的接口（domain、ingress、guests、`status` 的扩展）在 [ADR-0090](0090-public-access.md) 的“接口”，不在这里重复。

吊销设备的接口加一个参数 `keep_shared_passwords`，默认 false，响应列出待完成的吊销和登录没结束的应用。

错误用 ADR-0017 的格式：`unauthenticated`（401）、`plugin_unavailable`（503，包括 `setup_done` 查不出结果，附上标准错误输出，ADR-0022）、`credentials_failed`（502，附上命令的标准错误输出）、`invalid_input`（400）、`conflict`（409）、`lan_unavailable`（422）、`port_quota_exceeded`（409）、`home_suspended`（403，写明原因）、`quota_exceeded`（403）、`invite_invalid`（403，邀请码不存在、用过或过期）、`not_registered`（409）、`setup_pending`（409，应用的首次注册没完成，ADR-0022）、`core_not_in_container`（422）。

**入口的状态分开显示，不合并成“连不上”。** 核心定期从家里按公网地址连回自己（一个专门的探测标签，经 DNS、中继、连接器、入口容器再回到核心，验证证书），这样不用认识连接器是什么。分清：证书快过期或已过期、DNS 不对、中继或连接器不通、这个家被停用、超配额、插件没在运行、应用的首次注册状态不确定（ADR-0022）。这个自检只说明从家里绕出去是通的，不代表外面所有网络都通，App 这样写。

### 公网暴露面

ADR-0011 写的是“所有访问经核心网关鉴权”。这篇开了一个明确的例外：`credentials`、`app` 模式和 `clients.tcp` 下，**未认证的请求能到插件**，因为验证本来就是应用自己做的。`gateway` 模式下，未认证的请求停在网关。

**未认证的请求一路上遇到什么：**

| 层 | 做什么 |
| --- | --- |
| 中继的主机 | 防火墙按来源限新连接速率（nftables），防止打满一个家 |
| 中继 | 只转登记过的名字，其余回 `unrecognized_name`（实测）。按入口限带宽，按家限并发和月流量 |
| 入口容器，握手 | 只接受连接器；每个前端一个 `crt-list`：公网前端只放打开了公网入口的 `<标签>.<后缀>` 和入口自检的探测标签，局域网前端只放打开了局域网入口的 `<标签>.lan.<后缀>`，不在列表回 `unrecognized_name`（实测）。共用一个列表的话，路由器转发模式下公网客户端、中继模式下被攻破的 frpc 用 lan 名字就绕开了公网开关，局域网设备用公网名字就绕开了 `public.deny` |
| 入口容器，握手后 | 读 PROXY 头，按来源（IPv4 或 IPv6 /64）限速和封禁（实测） |
| 入口容器，HTTP | 删掉名字带 `_` 的请求头（实测）。`Host` 必须和 SNI 一致，否则 421（实测）。两个前端都删掉客户端传来的 `Forwarded`、`X-Forwarded-*`、`X-Real-IP`、`X-Wonton-Entry`，写上真实来源和入口（实测） |
| 核心 | 校验对端是入口容器；`X-Wonton-Entry` 没有、有多个、和 `Host` 的形式对不上，或者这个入口的开关关着，都拒绝，关开关时断开这个入口已有的连接（TCP 入口开关关着时连端口都没有，见“普通 TCP 转发”）；`public` 路径限制；`gateway` 模式验证；删掉 `webui.proxy` 声明的请求头和 Cookie；表外的路由一律拒绝（Tuwunel、Hydra、MCP 总线、插件网页入口、核心的管理接口都不在表里） |
| 插件 | 应用自己的登录 |

**防暴力破解不能对所有插件一样：** Immich 错了回 401，Vaultwarden 回 400 `invalid_grant`，Subsonic 回 200 加错误码。所以由清单声明，或者明确交给应用：

- **`auth_failures` 声明哪些响应算验证失败**（路径和状态码）。核心按“路径怎么匹配”判断，命中就在响应上加内部头 `X-Wonton-Auth-Failure`，入口容器据此计数并删掉它（实测）；插件自己带这个头时核心先删掉。不看响应体。**声明才计数**，匿名访问也可能回 401（[Immich 的共享相册就撞过](https://github.com/immich-app/immich/issues/3358)），不能默认全算。
- **`gateway` 模式只计带了 `Authorization` 但校验失败的 401，核心同样加内部头。** 没带时网关回的 401 是正常的 Basic 质询，DAV、Git 客户端第一次请求都会收到。`gateway` 入口不得声明 `auth_failures`，清单校验直接报错，不静默忽略：声明了却不生效的字段是清单陷阱。
- **看不出失败的，交给应用，并定为插件必配：** Vaultwarden 配 `LOGIN_RATELIMIT_*` 和 `IP_HEADER=X-Real-IP`（`IP_HEADER_TRUSTED_PROXIES` 默认的 `local` 覆盖内部网络）。Navidrome 的 Subsonic 接口回 200，看不出来，但它自己按来源和用户名限失败次数（`server/subsonic/middlewares.go`，默认分支），网页登录有 `AuthRequestLimit`。
- **官方索引收录时要求二选一：** 声明了 `auth_failures`，或者在 `about` 写明应用自己怎么限。Immich [没有登录限速](https://github.com/immich-app/immich/discussions/21165)（2025-08 的讨论），所以 Immich 插件声明 `/api/auth/login` 的 401。Jellyfin 的 `LoginAttemptsBeforeLockout` 是按用户锁的，外人能把主人锁在外面，插件不要打开它。
- **计数：** 按来源（IPv4 单个地址，IPv6 /64，实测同一个 /64 共用一个计数），5 分钟内失败 10 次，这个来源全部回 429，封 15 分钟（入口容器执行，机制实测，数值见[待验证](0021-verification.md#待验证)）。另外按入口一个全局上限：所有来源合计 5 分钟内失败 50 次，封 15 分钟，防止换很多 /64 慢慢试（核心执行）。**全局上限只拦 `auth_failures` 声明的路径**，已登录的客户端不受影响；否则外人能把整个入口打成 429。**`gateway` 入口不设全局上限：** 所有请求都带 `Authorization`，入口容器分不清对错，一拦连主人的日历同步也拦了；网关的密码都是 128 位随机数，猜不中，按来源封就够了。取 50 的理由：共享密码换值后，几台设备一起失败，每台到 10 次就被单独封了，碰不到它；换来源的攻击者被压到每 20 分钟 50 次，一天约 3600 次，猜不中 128 位的专用密码，主要保护主人自己设的密码。
- **运营商级 NAT。** 同一个出口的人会被一起限，主人自己也可能撞上。只封验证失败多的来源，有时限，主人能在 App 里解除。

**审计。** 写进 `core/audit.jsonl`：

- 完成握手、命中标签的连接：来源、插件、入口、字节数、开始和结束时间。
- **没命中的不逐条记：** 按来源每小时聚合计数，每小时最多留 1000 个来源，其余合并成“其他”。
- 验证失败按来源聚合；封禁和解除；开关；专用密码的新建、取值（具体到设备）、吊销；CT 监控的告警；CAA 和令牌的变更。
- 不记 HTTP 请求内容。入口容器的日志视为不可信数据，核心读 Docker 日志后解析。容器日志按大小轮转（10 MiB，3 份），审计文件 64 MiB 轮转，留 4 份，外人打不满磁盘。stick-table 也有固定上限。

**外人能不能列出装了哪些插件：**

| 渠道 | 能看到什么 | 怎么处理 |
| --- | --- | --- |
| 证书透明日志（公开） | `*.<家>.<域名>`、`*.lan.<家>.<域名>`，**没有插件名** | 用通配符。`<家>` 是随机的。所以 CT 能列出**有哪些家**，列不出谁家装了什么 |
| DNS | 通配符对任何标签都回答同样的结果 | 不开 zone transfer，不做 NSEC 枚举 |
| 中继 | **知道 `<家>` 的人试 `immich.<家>.<域名>`，开了的握手继续，没开的回 `unrecognized_name`** | 可读名字的代价，主人接受。只暴露“开了公网入口”的，装了没开的看不出来。主机按来源限速 |
| 路上的人（运营商、Wi-Fi） | SNI 是明文，看得到完整名字，包括应用名 | 第三方 App 不支持 ECH。可读名字的代价 |
| 我们的中继 | 同上，加上流量大小和来源 IP | 不解密；日志 7 天 |
| 端口扫描 | **TCP 端口池上开着的端口和它的横幅**（如 SSH 版本），不用知道 `<家>` | 普通 TCP 的代价，见下 |

**官方索引。** 声明了 `network.clients` 的插件，收录时要审核默认配置：默认账号、未认证能到的接口、已知的未认证漏洞、`public` 路径限制、自助注册默认关、防暴力破解。来源不是官方索引的插件，打开开关时明确写“这个应用会暴露到公网”。

### 防滥用和 PSL

任何注册者都能在我们的域名下放带正规证书、内容我们看不到的网站，可能被用来钓鱼，整个域名被安全浏览拉黑。

**PSL 没生效之前，所有家同站，可以互相写 Cookie：** 一个家的页面能给 `<域名>` 写 Cookie，发到别的家。第三方原生 App 一般不受影响，但插件的网页在浏览器里打开时受影响。**定为规则：把 `<域名>` 加进 [PSL 的私有部分](https://github.com/publicsuffix/list/wiki/Guidelines)、并且在主流浏览器里生效之前，只允许邀请制注册。** PSL 明确不接受只为绕过限额的申请，但把子域名发给“互不信任的各方”正是它写明的用途；要求域名注册期还剩两年以上，由志愿者审核。

- **注册门槛：** 邀请码由家名服务的管理命令生成，一码一用，有过期时间（生成时指定），管理员手动发。注册按来源限速。
- **举报和下线：** 官网和证书页写明举报邮箱。下线一个家是一条命令，**同时停 DNS 和停 frps 路由**（`Login`、`NewUserConn`、`Ping` 都拒绝），生效时间见“中继”。App 显示这个家被停用和原因。
- **安全浏览监控：** 家名服务用 Web Risk 的 [Update API](https://cloud.google.com/web-risk/docs/update-api)，`threatLists.computeDiff` 下载本地库（免费），查登记过的名字时先在本地比对哈希前缀，命中才把前缀发给 `hashes.search`（按量计费）。不用 Lookup API：它每月 10 万次以内免费，但把完整的 URI 发给 Google，等于把所有家的可读标签成批交给第三方。[Safe Browsing API](https://developers.google.com/safe-browsing/v4/usage-limits) 限非商业用途，不用。域名本身加进 Search Console 收告警。
- **入口专用一个和产品官网无关的域名**（部署时定，注册商也是），一个家出事不影响官网。

**先只在邀请制范围内（主人和朋友）运行。公开注册的前置条件，缺一不可：**

1. `<域名>` 进了 PSL，并且在主流浏览器里生效。
2. 部署拆开，见“部署和被攻破的影响”。
3. 先定运营主体，再咨询律师（替别人转发内容不可见的流量）。

### 迁移和回退

- **核心回退到这篇之前的版本。** 旧核心没有 8484，也不认识入口容器。按 ADR-0025 的“核心管理的其他组件”处理：入口容器和连接器带 `wonton.managed-by=core` 和 `wonton.core.component=entry|connector`，支持它的镜像在 `wonton.core.components` 里声明；回滚或手动退回到不支持的版本时删掉这两个容器（容器由核心建，删了就是删，下次升级回来时按设置重建）。连接器和入口容器一起消失，8483 也关了，中继上的名字随 frpc 断开而消失。第三方 App 如实连不上。手动退回用 ADR-0025 的命令。再升级回来时数据都在，开关恢复。旧核心遇到规范 `0.3` 的已装插件怎么办，见[待验证](0021-verification.md#待验证)。
- **从我们的中继换成自己的域名。** 主人设 `WONTON_CLIENT_DOMAIN`，核心按新名字申请证书、生成配置，App 显示新地址，第三方 App 要重填。旧入口：签名 `DELETE /homes/{家}`，删掉路由、局域网记录、CAA 和两条 CNAME，吊销令牌，TCP 端口进隔离。旧证书和私钥从证书目录删除。旧名字如实解析不了，不做跳转。CT 日志里的历史记录留着。换回来要新的邀请码，是一个新的家。
- **卸载。** 卸载脚本发注销；发不出去时，家名服务发现这个家 180 天没有心跳和签名请求，就按同样的流程停用和清理。
- **`<家>` 永不复用。** 注销后名字进墓碑表。否则旧家的第三方 App 会把专用密码发给新主人。40 位随机数不缺名字。TCP 端口没有这个余地（端口池有限），所以有 180 天隔离，SSH 的主机密钥也能挡一层，其他协议是残余风险。
- **入口容器被攻破或私钥泄露。** App 里一键：吊销旧证书（lego `revoke`）、签新证书、轮换 frp 令牌。私钥换了以后旧名字不变。

### 范围

M1 做局域网直连和主人自己的域名（在 App 里配，ADR-0089），公网入口用主人自己的路由器转发、自己的 frps 或 Cloudflare Tunnel（ADR-0090），不依赖我们的云服务，验收见 [GOAL](../GOAL.md) 的 M1 第 9 项。公网中继、家名服务、DNS、证书服务和 CT 监控在 M2 部署，和推送、中继节点一起（主人 2026-11-04 定）。

以后：家人（专用密码按人分，`gateway` 的 `{user}` 跟着变）；第三方 App 的 OIDC（Immich 手机 App 支持 OAuth，能解决“每客户端一个”，要改核心的登录流程，另写 ADR）；多地区中继；付费（主人说以后再说）。

## 理由

- **第三方 App 只会填一个地址，公网 HTTPS 是所有 App 都支持的唯一形式。** 不用装证书，不占系统的 VPN，不和 Clash 冲突，iOS 和 Android 一样。
- **家里没有公网地址，所以要中继；TLS 在家里终止，所以中继只看 SNI。** 中继和 DNS 都是可替换的组件，核心只认 TLS、PROXY v2 和证书目录。
- **能用现成的都用现成的。** 中继 frp，入口 HAProxy，证书 lego，TXT acme-dns，DNS PowerDNS。自研的只有注册和 frps 插件，因为这一块本来就是我们自己的策略。
- **公网解析放在没有特权的容器里，不放在核心。** 攻破入口不等于攻破宿主。
- **令牌按能力拆开，重要的改动只认 endpoint 私钥。** 泄露一个令牌的影响是能说清楚的。

## 没选的做法

- **系统级 VPN（iOS 的 packet tunnel、Android 的 `VpnService`，含 `wonton/vpn/0` 和核心自建 CA）。** 主人否掉了。系统同一时间只允许一个 VPN（[Android 文档](https://developer.android.com/develop/connectivity/vpn)：“启动新的会自动停掉旧的”；[iOS 同样](https://developer.apple.com/forums/thread/675507)），和国内用户常用的 Clash 冲突，这也是 ADR-0004 当初不用 VPN 的理由。国内上架要 [MIIT 牌照，只发给中国公司](https://developer.apple.com/forums/thread/105600)（[App Store 指南 5.4](https://developer.apple.com/app-store/review/guidelines/#vpn-apps)）。
- **核心自建 CA，引导主人装进手机。** iOS 要五六步，Android 有长期的“网络可能受到监控”；Android 7 起 App 默认不认用户 CA，Dart 的 `HttpClient` 只读系统目录，很多 App 根本用不了。
- **主人自己装 Tailscale，或者核心带 Headscale。** 还是系统 VPN，还多一个账号和控制面（ADR-0009）。
- **TLS 和协议解析放在核心里（rustls）。** 核心挂着 Docker socket，攻破它就是宿主 root，违反 ADR-0011 和 ADR-0015。
- **入口容器留在 `wonton-lanpub` 上，宿主加一条防火墙规则挡住它连宿主。** 不用多一个局域网转发，但和 ADR-0019 的“核心不改宿主的防火墙”冲突，主人自己的防火墙配置也可能和它打架。
- **一个家令牌管所有事。** 泄露后既能劫持路由又能签证书。
- **frpc 登记 `*.<家>.<域名>` 通配符。** 开关只能在家里执行，没开的插件流量也到家。
- **中继用 iroh。** 见“中继”。公网一侧等于重写 frps。
- **中继解开 TLS 再转发（Caddy、Traefik、Pangolin 这类）。** 私钥要放在我们这边，违背方向 2。
- **PowerDNS 的 RFC 2136 + 按 zone 的 TSIG。** 见“家名服务和 DNS”。写 A 记录时校验不了内容。
- **`lan.ports` 放开 `webui` 端口（调研报告的建议）。** 没有 TLS，密码在 Wi-Fi 上明文；局域网设备绕过网关，`sso: proxy` 的前提不成立；Bitwarden 这类强制 HTTPS 的 App 还是用不了。这条限制已被 ADR-0023 取消：要不要接受这个风险由主人决定，第三方 App 的推荐做法仍是这篇的 `clients`。
- **局域网里用 mDNS（`.local`）。** ADR-0019 已经否掉，也拿不到公共证书。
- **公共的通配 DNS（sslip.io、nip.io）。** 依赖第三方，也拿不到我们名下的通配符证书。
- **IP 写进名字（Plex）。** 主人选了固定名字。
- **一条 DNS 记录同时给局域网和中继两个地址。** 客户端不一定会换地址重试；在别人家的局域网里，同一个 IP 可能是别的设备，连上后 TLS 失败也不会再试。
- **路由器上做分离解析（同一个名字在家解析到局域网）。** 我们控制不了主人的路由器，端口也对不上（公网 443，局域网 8483）。有路由器的主人可以自己做。
- **随机的插件标签。** 外人和中继看不出应用，但地址又长又丑，主人选了可读的名字。
- **每个插件一张证书，HTTP-01 或 TLS-ALPN-01。** 见“证书”。插件名都进 CT 日志。
- **核心内嵌 ACME（`instant-acme`，Apache-2.0）。** 少一个容器，但自己的域名要逐家对接 DNS 服务商，这是 lego 现成的 200 多家。
- **把 `<家>` 取成 endpoint ID。** 不用另外发名字，但 endpoint ID 进了 CT 日志，也太长。
- **CT 监控由我们的服务做。** 防的就是我们自己，要第三方。
- **一开始就按完整的方式拆成五台。** 主人定了一台：邀请制阶段只有主人和朋友，主机的钱和运维先不花。公开注册前必须拆。
- **给 acme-dns 第二个公网 IP，不用 dnsdist。** 少一个组件，但依赖主机商能加 IP，dnsdist 还能顺便按来源限速。
- **核心替主人定 Vaultwarden 的主密码。** 核心就能解开密码库。
- **网关替 Immich 这类应用拦截登录请求。** 核心要认识每个应用的登录接口，违反 ADR-0015。清单声明的路径和状态码是另一回事，核心不认识应用。
- **入口容器读响应体判断失败（Subsonic）。** 要缓冲响应，还要理解应用协议。交给应用。
- **第三方 App 也走 iroh。** 它们不认识 iroh，这就是这篇要解决的问题。
- **专用密码只存哈希。** 要能查看。

## 代价与风险

- **带宽是最大的代价。** 第三方 App 在外的流量都经中继，没有 P2P。备份 200 GB 的相册，中继进出各 200 GB；连在家用公网地址也一样。这正是 ADR-0004 否掉公网隧道的理由，这次为了第三方 App 接受。靠限速和月流量控制，免费的代价我们出。每月 100 GB 放不下 200 GB 的相册，5 MB/s 跑满 100 GB 也要将近 6 小时，所以大相册第一次备份要在家走局域网入口。所有家共用一台主机的带宽，没有按主机的公平分配，几家同时跑满时互相影响。
- **普通 TCP 的攻击面更大。** 每个入口一个公网端口，**端口扫描能直接找到**，不用知道 `<家>`，也不用猜名字；**没有 SNI 区分**，握手前什么都挡不了，只能按来源限速；协议自己的验证是全部防线；插件看不到真实来源，应用自己的封禁（如 SSH 的 fail2ban）不起作用；横幅还可能暴露版本。所以默认关，只有主人能开，App 写明这一点。
- **可读名字暴露应用。** 运营商和 Wi-Fi 看得到 SNI，中继看得到，知道 `<家>` 的人能探测（主人接受）。
- **速度取决于家的上行和到中继的路。** 手机和家都在国内而中继在香港时，流量出境再回来。国内跑得怎么样要实测。
- **多了云服务，而且是单点。** 域名，加一台主机上的中继、家名服务、acme-dns 和权威 DNS。ADR-0012 的云服务多一类。这一台被攻破或挂掉的后果见“部署和被攻破的影响”。
- **合规。** 大陆节点的域名要备案（ADR-0004）。替别人转发内容不可见的流量，律师的意见是公开注册的前置条件，邀请制阶段没有这份意见。境外域名和 SNI 在国内可能被挡。
- **插件的未认证漏洞直接在公网上。** 可读名字挡不住扫描，知道 `<家>` 就能试。靠默认关、路径限制、官方索引审核、及时升级。公网入口随时能关。
- **控制连接和 HTTPS 共用 443 时第一个字节不标准**，见“中继”。
- **frp 还在 0.x，又是公网服务。** 一个 frps 崩溃影响所有家（v0.71.0 修过[客户端能触发的服务端崩溃](https://github.com/fatedier/frp/releases/tag/v0.71.0)）。限制功能面，及时升级，frpc 的版本跟核心发布走。
- **宿主的 Docker 配了代理时**（`~/.docker/config.json`，国内很常见），环境变量注入容器，frpc 经代理连 frps，登录失败，只报 `session shutdown`（实测）。连接器要显式清掉这些变量。
- **入口容器被攻破时**证书私钥和经过的明文（包括专用密码）都泄露。它连不上宿主，但核心的 HTTP 解析还在攻击面里。
- **局域网转发、连接器和 lego 连得上宿主。** 局域网转发在关掉地址伪装的网络里（实测），连接器和 lego 在普通桥接上，都连得上宿主的 sshd、`${WONTON_PORT}`、隧道端口和 `network.lan` 插件的端口，受宿主防火墙约束。核心不改宿主的防火墙（ADR-0019），所以留着这个缺口。三者都不解析公网来的 TLS 和 HTTP：局域网转发只收局域网来的连接，只转字节；连接器转公网的字节，还和中继讲 frp 协议，被攻破就能把宿主的端口经中继暴露出去；lego 只出网。路由器转发模式下，接公网连接的 TCP 转发容器也连得上宿主。
- **重建局域网转发会断开局域网上的第三方连接。** 地址变化时发生，很少。
- **第三方 App 看到错误时，我们帮不了。** 它们只会说“连不上”或“证书错误”。所以 Wonton App 要把入口的状态写清楚。
- **DNS 重绑定保护挡住时，局域网地址用不了。** 要主人改路由器，或者走公网地址。
- **Clash 这类代理。** 公网地址和 Clash 没有冲突。局域网名字要规则放行，否则可能被送去代理（fake-IP 尤其如此），要实测。
- **全局失败上限能被外人触发。** 主人在这段时间也登录不了新客户端，已登录的不受影响。
- **`<家>` 变了，所有第三方 App 都要重填。** 核心数据丢失又没有备份时会发生。
- **多三个常驻容器**：入口容器和局域网转发（同一个 HAProxy 镜像，60 MB。内存在开发机上是入口容器约 27 MiB，在 OrbStack 上两个都约 80 MiB）、frpc（镜像 39 MB，内存约 12 MiB），加上每天一次的 lego。

## 验证

验证结果和待验证的项目在 [0021-verification.md](0021-verification.md)。

小程序经隧道不另加 HTTPS，见 ADR-0009。

## 依赖

| 依赖 | 许可证 | 用途 | 方式 |
| --- | --- | --- | --- |
| HAProxy | GPL-2.0 | 入口容器 | 官方镜像，独立容器，不修改 |
| frp（frps、frpc） | Apache-2.0 | 中继和连接器 | 官方镜像，独立容器 |
| lego | MIT | ACME | 官方镜像，一次性容器 |
| acme-dns | MIT | DNS-01 的 TXT | 我们运行的独立进程，不分发 |
| PowerDNS Authoritative | GPL-2.0 | 权威 DNS | 我们运行的独立进程，不分发 |
| [dnsdist](https://github.com/PowerDNS/pdns/tree/master/pdns/dnsdistdist) | GPL-2.0 | 单机上分发公网 53 | 我们运行的独立进程，不分发 |

核心不增加链接进二进制的依赖（TLS 和 PROXY 都在入口容器里）。

## 关联

ADR-0004（隧道）、ADR-0005（厚核心）、ADR-0009（身份和配对）、ADR-0011（安全模型）、ADR-0012（云服务）、ADR-0015（小组件）、ADR-0016（Bot 能装什么）、ADR-0019（局域网）、ADR-0020（文件服务）。
