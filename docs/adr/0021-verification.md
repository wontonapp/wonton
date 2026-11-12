# ADR-0021 的验证

[ADR-0021](0021-third-party-clients.md) 的验证结果和待验证的项目。

## 验证结果（2026-11-03）

除了注明的，都在 npchy（x86_64，Docker 29.7.2）上用一次性容器，用完删除。

**整条链路**：frp v0.71.1 官方镜像、`haproxy:3.2-alpine` 官方镜像（`--cap-drop ALL --read-only`，以 uid 99 运行）。三条独立的 Docker 网络，`wonton-edge`、`wonton-clients` 是内部网络。后端是一个 Python 小程序。没有经过公网。证书是自签名的 `*.k3f.example.test`、`*.lan.k3f.example.test`，客户端只信这张证书。

| 测试 | 结果 |
| --- | --- |
| curl → frps → frpc → HAProxy → 后端，登记过的名字 | 200，HTTP/2，验证的是家里的证书，中继没有解密；后端看到真实来源、`X-Wonton-Entry: public` |
| 客户端伪造 `X-Forwarded-For`、`X-Real-IP`、`Forwarded` | 都被删掉，`X-Real-IP` 是 PROXY 头的值 |
| 没登记的 `navidrome.k3f…`、`immich.lan.k3f…` 经中继 | frps 回 `unrecognized_name`，到不了家 |
| 登记时：抢别家的名字、抢别家的 TCP 端口、登记 `lan` 名字 | `NewProxy` 全部拒绝 |
| TCP 经中继（`type = "tcp"`，入口容器的 TCP 前端） | 通，到后端，后端看到的来源是入口容器。端口池里没分配的端口：连接被拒 |
| `bindPort = vhostHTTPSPort = 443` + `tls.force` | frpc 不设 `disableCustomTLSFirstByte = false` 时连不上（`session shutdown`），设了以后控制和 HTTPS 都正常 |
| `frpc reload`（`docker exec`，`webServer` 绑 `127.0.0.1`） | 新加的 `forgejo` 名字回 200。撤销名字没有单独测，见“待验证” |
| HAProxy `USR2` | 下载中的连接没断（64 MiB 完整收到），新连接正常 |
| 未信任地址连入口容器（带或不带伪造的 PROXY 头） | 连接被重置，没到 HTTP |
| `Host` 和 SNI 不一致 | 421 |
| 列表外的 SNI | `unrecognized_name` |
| 同一个 IPv6 /64 累计 6 次 401（测试阈值 5 次） | 第 7 个请求 429，同一个 /64 的别的地址也 429，另一个 /64 正常 |
| 运行时接口 | `show table` 看得到计数，`clear table` 清掉，只在 `wonton-clients` 上可达 |
| 限速（插件把 2MB 写进 `bandwidth_limit`，服务端模式） | 8 MiB 用 3.0 秒，符合 2 MB/s 加一秒的突发；frpc 的配置里没写。限速按 proxy 算，没有按家的总量 |
| 指标（`webServer` 绑 `127.0.0.1`，`enablePrometheus`） | `frp_server_traffic_out`、`frp_server_connection_counts` 按 `home1.immich` 分开 |
| 暂停的家：`NewUserConn` | 拒绝后后端没有新连接 |
| 暂停的家：`Ping` | frpc 在下一次心跳（测试配置 4–5 秒）收到错误并断开，下载中的连接也断了，`client_counts` 归 0 |
| 暂停的家：`Login` | 拒绝 |
| frpc 不发心跳，`heartbeatTimeout = 12` | 12 秒后被断开 |
| 内存 | HAProxy 27 MiB、frpc 12 MiB、frps 47 MiB（跑完这些之后） |

**局域网端口**：`-p 192.168.0.193:18483:8483`。

| 测试 | 结果 |
| --- | --- |
| npchy 上的监听 | 只有 `192.168.0.193:18483`，没有 `0.0.0.0` 和 `::` |
| 同一局域网的 Mac（192.168.0.182） | 200，`X-Real-IP: 192.168.0.182`，客户端伪造的被覆盖 |
| npchy 的 IPv6 全局地址（从 Mac） | 连不上 |
| Docker 网关（宿主连容器地址，来源 10.213.4.1） | 连接被重置 |
| 列表外的标签、`Host` 不一致 | `unrecognized_name`、421 |

**证书和 DNS**：lego v5.5.2、acme-dns（`joohoi/acme-dns`）、CoreDNS 1.12.1 做解析（主 zone 里两条 CNAME，`auth` 转给 acme-dns）、Pebble 做 CA。

| 测试 | 结果 |
| --- | --- |
| `lego run --dns acme-dns --pem -d '*.k3f…' -d '*.lan.k3f…'` | 跟随 CNAME，两个 DNS-01 都通过，发了一张两个通配符的证书，生成 `.pem` |
| 别的 acme-dns 账号写这个家的子域名 | 401 `forbidden` |

Pebble 不是 Let's Encrypt，CAA 没有测。

**单机的 DNS**：dnsdist 1.9.16（`powerdns/dnsdist-19`）、`powerdns/pdns-auth-49`、acme-dns，都是 2026-11-03 的 `latest`。PowerDNS 和 acme-dns 只在 Docker 内部网络上，dnsdist 同时连内部网络和另一个桥接网络。

| 测试 | 结果 |
| --- | --- |
| 从外侧经 dnsdist 查 `auth.example.test` 的 SOA 和子域名的 TXT | acme-dns 回答 |
| 查 `*.k3f`、`*.lan.k3f` 的 A，`k3f` 的 CAA，`_acme-challenge.k3f` 的 CNAME | PowerDNS 回答 |
| 从外侧连 PowerDNS 的 API | 连不上；内部网络上回 200 |
| 内存 | dnsdist 58 MiB、PowerDNS 48 MiB、acme-dns 5 MiB |

**隔离、请求头和令牌**：OrbStack（arm64，Docker 29.8.2）上的一次性容器，frp v0.71.1、`haproxy:3.2-alpine`（`--cap-drop ALL --read-only`，uid 99）。没有经过真实局域网和公网，“宿主”是 OrbStack 的虚拟机，服务是 host 网络上一个监听 18022 的程序。

| 测试 | 结果 |
| --- | --- |
| 入口容器（只在两个 `isolated` 内部网络上）连宿主地址和各网关地址 | 连不上，路由表只有这两个子网 |
| host 网络连入口容器的地址 | 连不上；不带 `isolated` 的内部网络连得上 |
| 局域网转发（关掉地址伪装）连宿主地址 | 连得上 |
| 同网段的客户端经局域网转发到入口容器 | 200，`X-Real-IP` 是客户端地址，伪造的被覆盖，`X-Wonton-Entry: lan` |
| 网关地址作来源连局域网转发 | 连接被重置 |
| 发 `X_Remote_User`（`http-restrict-req-hdr-names delete`） | 到不了后端 |
| 经公网发两个 `X-Wonton-Entry: lan`，经局域网发 `public` | 后端只收到一个，分别是 `public` 和 `lan` |
| 连接器的位置连局域网前端和 `wonton-lanfwd` 上的局域网转发 | 超时。经路由连局域网转发在 `wonton-lanpub` 上的地址：连上后被来源过滤重置 |
| 入口容器连 frpc 的管理端口、局域网转发 | 连不上 |
| frpc（`--cap-drop ALL --read-only`，uid 99） | 经中继通，`frpc reload` 成功 |
| 后端响应带 `X-Wonton-Auth-Failure`（测试阈值 2 次） | 第 4 次起 429，客户端看不到这个头 |
| home1 的令牌填 `user = home2` | `Login` 拒绝，frpc 登录失败 |
| 轮换 home1 的令牌（心跳间隔 3 秒） | 下一次 `Ping` 被拒，frpc 断开，用旧令牌重新登录也被拒。用新令牌重建连接器后回 200 |
| 不发心跳的 frpc（`heartbeatTimeout = 12`）的令牌被轮换 | 新连接立刻被拒；别的 frpc 抢不到同名；到时间后控制连接和下载中的连接都断开。没有测名字释放后别的 frpc 能接上 |
| PowerDNS 的 API（绑在 `dns-api`，`webserver-allow-from` 只有一个地址） | 那个地址 200；同网络的别的地址连接被重置；host 网络超时；`dns` 网络上的容器（dnsdist、acme-dns 的位置）报 `Network is unreachable` |
| dnsdist（uid 953，`--cap-drop ALL --read-only`）、acme-dns、PowerDNS（`--cap-drop ALL`） | 都在 53 上正常回答 |

**读代码或文档的**：PowerDNS 的 `UpdatePolicyQuery` 没有记录内容；frp 两端心跳的默认值、`Ping` 被拒时不刷新 `lastPing`、`frpc reload` 不改全局参数、共用端口时第一个字节的要求；Vaultwarden、Immich、Jellyfin 的路径；Navidrome 的限流；各比较表（默认分支，2026-11-03）。Let's Encrypt 的限额和有效期、PSL 的规则、dns-persist-01 的状态、SSLMate 和 Web Risk 的额度（公开资料，2026-11-03）。

## 待验证

都在实现时验证，结果写进这一节。

**中继和网络**

- 真实公网上的吞吐和延迟：国内家宽 + 香港的中继，备份一个 1 GiB 的视频。frp 的 TCP 和 QUIC 比一次。
- 控制连接的第一个字节 `0x17` 在国内家宽上会不会被干扰。
- 连接器断线重连时，第三方 App 的表现（Immich 的后台备份）。
- frps 带上百个家、上千条连接时的内存和 CPU。
- 关开关时 `frpc reload` 撤销名字，frps 上立即回 `unrecognized_name`。
- 单机部署：frps 非 root、host 网络、nftables 把 443 重定向到高端口，IPv4 和 IPv6 的真实来源都不变。
- 注册商和顶级域对两个 NS 指向同一个 IP 的要求，选域名时查。不接受就要第二个 IP。
- 家名服务经 API 建 zone 和记录后立即生效（`pdnsutil` 建的 zone 在 PowerDNS 重启前回 REFUSED，2026-11-03 实测）。
- lego 的 `acme-dns` provider 用带路径前缀的 API 地址（经家名服务转发 `/update`）。
- frpc 校验 frps 证书（`trustedCaFile`）在官方镜像里怎么配。

- 隧道的 IPv4 UDP 套接字绑核心在 `core` 网络上的地址以后，发布的端口和打洞都正常（`tunnel.rs` 绑的是 `0.0.0.0`）。核心接入插件网络以后默认路由可能不走 `core` 网络，要看出去的 UDP 能不能回来。

**局域网地址**

- DHCP 换地址后，多久恢复（检测 5 分钟 + TTL 60 秒是估算，没有实测），重建局域网转发要多久。
- 局域网转发经真实局域网：`HostIp` 发布到关掉地址伪装的 `wonton-lanpub` 上，来源是手机的真实地址，公网 IPv6 和 Docker 网关都连不上，入口容器连不上宿主的 22 和 `WONTON_PORT`。上面的局域网测试是直接发布入口容器到普通桥接。
- 有 ufw 的宿主上，`isolated` 的内部网络和局域网转发的行为一样。
- 常见路由器和国内运营商 DNS 对 `*.lan` 的回答（OpenWrt、华硕、小米、光猫），写进 App 的提示。
- 开着 Clash（VPN 模式，fake-IP）时，第三方 App 能不能连局域网名字，要什么规则。
- iOS 上第三方 App 的本地网络权限：Immich、Bitwarden、Jellyfin。

**证书和监控**

- Let's Encrypt staging 上用 `lego run --csr` 签两个通配符（Pebble 上测的是不带 `--csr`）；`--csr` 时的 ARI 续期；CAA 的 `accounturi` 拦截别的账号；lego 的账号注册和签发分开成两步。
- 重装、从备份恢复时不撞“相同名字 7 天 5 张”。
- SSLMate 匿名查询的额度（文档写的是免费账号的额度）够不够每 6 小时查一次，多少个家以后要换账号或别的来源。
- Web Risk 的 Update API 本地库的大小和更新频率，`hashes.search` 的命中量。

**防暴力破解和路径限制（实测 Vaultwarden 和 Navidrome）**

- Vaultwarden：`auth_failures` 的 400 计数，`LOGIN_RATELIMIT_*` 经 `X-Real-IP` 按真实来源生效。
- Navidrome：Subsonic 接口的失败限制在发布版本里存在，按 `X-Real-IP` 生效；`/rest` 路径上回 200 时入口容器不计数。
- 路径规则：`/x/y%2Fz/%2e%2e/%2e%2e/admin`、`/admin/x//..//..`、`/admin/..;/x`、`/x/../admin`、`/x%2f..%2fadmin`、`/x/..\admin`、`/%252e%252e/admin` 回 400；`/ADMIN`、`//admin`、`/%61dmin`、`/admin;x`、`/%2561dmin`、`/admin/` 都命中 `deny /admin`，`/admin-x` 不命中；`allow /share` 时 `/share;x` 被拒绝；Forgejo 的 `%2F` 分支名和非法的 `%zz` 能到插件（写成核心的行为测试）。
- 全局失败上限只拦声明的路径。
- 一次 DAVx5 或 iOS 日历同步不触发封禁。
- 阈值 10 和 50，固定封 15 分钟。实测的配置是失败率超过阈值时回 429，失败率降下来就解除，不是固定时长的封禁。

**第三方 App 和插件**

- Immich 官方 App 在家和在外各备份一张照片。
- Immich 的 Wi-Fi 自动切换是否可靠。
- Immich 插件的 `credentials`：不经网页建好主人的账号、设密码、改密码时带 `invalidateSessions` 结束所有登录，用的是哪个接口。Jellyfin 和 Navidrome 能不能结束登录，做不到的声明 `ends_sessions: false`。
- `:8483` 端口和长名字在各 App 的地址输入框里能不能填：Immich、Bitwarden、Jellyfin、DAVx5。
- Radicale（`gateway`）和 DAVx5、iOS 日历各连一次。
- Forgejo：`generates` 和 `verify`；Git over SSH 经 TCP 转发。
- 自己的域名 + 原版 frps；自己的域名 + cloudflared 的 `matchSNItoHost` 和 `header:CF-Connecting-IP`。

**迁移**

- 核心回退到不支持 `clients` 的版本时，`wonton-updater` 清掉项目（ADR-0025）。
