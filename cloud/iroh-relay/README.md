# iroh 中继（relay.808cbd.com）
本目录里的 `relay.808cbd.com`/`omarchy.online` 是**真实部署域名**，换域名要改 `install.sh` 的 `DOMAIN` 与 nginx `server_name`；占位示例是 `192.0.2.10`。

给 Wonton 隧道用的转发站：两台设备打洞失败时，端到端加密的 QUIC 流量经它中转，中继看不到内容，也不存数据。
只转发、不存账号和业务数据（[ADR-0012](../../docs/adr/0012-updates-release-cloud.md)）。
公共 n0 中继限速严重（2026-11-06 实测晚高峰约 20 KB/s），所以自建（[ADR-0048](../../docs/adr/0048-relay-table.md)）。

上游是 [iroh](https://github.com/n0-computer/iroh) 的官方 `iroh-relay`，固定版本、官方 release 二进制、不改编。

**内测期只给主人自己的设备用**，不对外公开、不开放注册：准入是 `shared_token`，没带令牌的连接被拒；
令牌随配对码/连接信息下发，只发给自己的设备。

## 端口

| 端口 | 协议 | 谁听 | 用途 |
| --- | --- | --- | --- |
| 443 | TCP | nginx | 中继协议（WebSocket upgrade，nginx 终结 TLS 后反代回环） |
| 80 | TCP | nginx | ACME http-01 和跳转 443 |
| 7842 | UDP | iroh-relay | QUIC 地址发现（QAD），客户端直连，不经过 nginx |
| 8742 | TCP | iroh-relay | 只绑 127.0.0.1，给 nginx 反代 |
| 19090 | TCP | iroh-relay | 只绑 127.0.0.1，Prometheus 指标 |

## 架构

```
客户端 ──TLS:443──▶ nginx（终结 TLS，按 WebSocket Upgrade 反代）──▶ 127.0.0.1:8742 iroh-relay
客户端 ──QUIC/UDP:7842──────────────▶ iroh-relay（QUIC 地址发现，自己的证书）
```

nginx 终结 TCP 443 的 TLS，再按 `Upgrade` 反代到中继；中继自己用 `--dev` 只跑 HTTP。
这样不用把现有 443 的 http 块挪成 stream，`808cbd.com`、`*.808cbd.com`、`omarchy.online`、
`push.808cbd.com` 的 server 块都不受影响。QUIC 的 QAD 走 UDP 7842，和 nginx 无关，所以中继仍要读一份
`relay.808cbd.com` 的证书——由 certbot 的 deploy hook 从 `/etc/letsencrypt/live/relay.808cbd.com/`
复制到 `/etc/wonton-iroh-relay/tls/`（0640 root:wonton-iroh-relay）。

中继以 `wonton-iroh-relay` 用户运行，只读根、无 capability、端口都 > 1024（见 `wonton-iroh-relay.service`）。

### 为什么是 nginx 终结 TLS，而不是 stream 按 SNI 透传

另一种做法（a）是 nginx `stream` + `ssl_preread`，按 SNI 把 `relay.808cbd.com` 原样透传给中继，让中继自己终结 TLS。
优点是中继独占证书、nginx 不碰明文；代价是这台机的 443 已经被 nginx 的 `http` 块占着，要改成 `stream` 监听 443
就必须把 `808cbd.com`、`omarchy.online`、`push.808cbd.com` 所有现有 server 块的 `listen 443` 挪到别的端口，
再给 `http` 加 PROXY protocol 恢复客户端 IP——动的是主人两个站和推送中继的配置，回归面大。

我们选（b）：nginx 照旧占用 443 并终结 TLS，新增一个精确 `server_name` 块，按 WebSocket Upgrade 反代到中继的回环 HTTP 端口。
现有 server 块一个都不动；证书由 certbot 单独签 `relay.808cbd.com` 这一张子域证书。
核实过上游 iroh-relay 1.4.1 的 relay 协议就是标准 RFC 6455 WebSocket upgrade（`src/server/http_server.rs` 校验
`Upgrade: websocket`），nginx 用 `proxy_http_version 1.1` + `Upgrade`/`Connection` 能代理；`test/relay-e2e.sh`
就是用这份 nginx 配置实跑 101、30 秒吞吐和错令牌拒绝。

### 为什么不用 Docker 镜像

ADR-0048 写的是用上游官方镜像；这台目标机没有 Docker，而旁边的推送中继也是 systemd。所以改成上游官方
**release 二进制**（同一个 release、sha256 校验、不改编），效果等价，且不引入 Docker 依赖。

## 部署

目标机是 Ubuntu 26.04，不用 Docker。把整个 `cloud/iroh-relay/` 目录拷到目标机（例如 `/root/iroh-relay/`），然后：

```bash
bash install.sh --email ops@example.com   # 装服务、vhost、签证书
```

- 首次必须先有 DNS：`relay.808cbd.com` A/AAAA 指到本机、安全组放行 443/tcp、80/tcp、7842/udp，否则 http-01 签不过。
- 没有 certbot 先 `apt install certbot python3-certbot-nginx`。
- certbot 会被主人的 `*.808cbd.com` 块骗到、把证书装到 `808cbd.com` 块上；`install.sh` 装完会换回主人的证书路径（同 `cloud/push-relay`）。
- `--no-cert` 只装服务不签证书（之后自己签）；`--offline-binary /tmp/iroh-relay` 用离线二进制。
- 可以重复跑：已有的 `config.toml`、`relay.env`、证书都不覆盖。

装机脚本会：建系统用户 → 装 `/opt/wonton-iroh-relay/iroh-relay`（校验 sha256 和版本）→ 装配置和 0600 令牌 →
装 systemd 单元并启用 → 签证书 → 装 vhost → 起服务 → 健康检查。全程幂等。

## 配置和令牌

- 配置文件 `/etc/wonton-iroh-relay/config.toml`（0640 root:wonton-iroh-relay），模板见 `config.toml.example`。
- 准入令牌在 `/etc/wonton-iroh-relay/relay.env`（**0600**），systemd 经 `EnvironmentFile` 注入
  `IROH_RELAY_ACCESS_TOKEN`，上游用它整个覆盖配置里的 `shared_token` 列表。配置里留空列表，
  令牌缺失时中继拒绝启动（失败即停）。
- **`https_bind_addr` / `quic_bind_addr` 必须写在 `[tls]` 节**（上游 `TlsConfig`）：上游对顶层未知键不报错，
  放顶层会被静默忽略、QAD 只剩回环监听；e2e 和 `install.sh` 都会断言 UDP 7842 绑在 `0.0.0.0`。
- **默认是拒绝所有人，不是 everyone**：`shared_token` 留空列表、又没有 `IROH_RELAY_ACCESS_TOKEN`（或令牌为空）时，
  进程直接拒绝启动（失败即停）；要放开必须显式改 `access = "everyone"`。反过来，整段删掉 `[access]` 会回落到上游默认的
  everyone，`install.sh` 会打 `everyone-default` 提醒加回。
- 每连接入方向默认限速 256 KiB/s（`[limits.client.rx]`，1 MiB 突发）：按实测 5 Mbps 出口取，单条连接最多占约 40%，
  让一条大同步不至于饿死语音（语音双向约 1.2 Mbps）。只保住“一条”同步：两条并发 4.2 Mbps 加语音 1.2 Mbps 仍会超 5 Mbps，大家一起慢。只是公平保护，不是全局限额也不是按 endpoint 计量。
- 内测期所有自己的设备共用一个令牌：令牌随配对码/连接信息下发，泄露后别人也能用这个中继，接受（ADR-0048）；
  只发自己的设备，不公开。收费时改成按 endpoint ID 的 http 回调，核心和协议不用改。
- 轮换：`printf 'IROH_RELAY_ACCESS_TOKEN=%s\n' "$(openssl rand -hex 32)" > /etc/wonton-iroh-relay/relay.env && systemctl restart wonton-iroh-relay`。
  旧令牌立即失效，所有客户端要在下次连接前更新连接信息。
- 上游没有按 endpoint 计量和限速：metrics 是进程累计值；限速只能全服务一个值（`[limits.client.rx]`，模板里注释着）。

## 证书和续期

- 证书域名只有 `relay.808cbd.com`，certbot 自动续期。
- deploy hook `/etc/letsencrypt/renewal-hooks/deploy/wonton-iroh-relay.sh` 在每次续期后复制证书、reload nginx。
- 中继的 TLS 用 `cert_mode = "Reloading"`：每 24h 重读证书文件，续期后最迟一天内生效，不用重启。
  QAD 用旧证书到续期后 24h 也没问题（Let's Encrypt 有效期 90 天，提前 30 天续）。
- 证书文件对中继用户可读（0640 root:wonton-iroh-relay），私钥不落 `config.toml`。

## 日志和指标

- 服务日志：`journalctl -u wonton-iroh-relay`（一行一条连接/断开，不带内容）。
- nginx：`/var/log/nginx/relay.808cbd.com.access.log`。
- 指标：`curl --noproxy '*' http://127.0.0.1:19090/metrics`。主要看
  `relayserver_bytes_sent_total` / `relayserver_bytes_recv_total`、`relayserver_connects_total` /
  `relayserver_disconnects_total`、`relayserver_send_packets_dropped_total`。
- 健康检查：`curl --noproxy '*' http://127.0.0.1:8742/healthz`（200）。
- **令牌默认不进日志**：systemd 单元默认 `RUST_LOG=info`（`EnvironmentFile` 里同名变量可覆盖），journal 里 grep 不到令牌。上游 1.4.1 在 debug/trace 下
  启动时会 dump 整份配置、包含令牌，开 debug 排错时别把日志原样贴出去。
  验收：`journalctl -u wonton-iroh-relay | grep -F -- "$(cut -d= -f2 /etc/wonton-iroh-relay/relay.env)"` 应无输出。

## 验收

```bash
# 本机
systemctl is-active wonton-iroh-relay
curl --noproxy '*' -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8742/healthz   # 200
curl --noproxy '*' -o /dev/null -w '%{http_code}\n' -H 'Connection: Upgrade' \
  -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13' \
  -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
  -H 'Sec-WebSocket-Protocol: iroh-relay-v2' -H "Authorization: Bearer $(cut -d= -f2 /etc/wonton-iroh-relay/relay.env)" \
  https://relay.808cbd.com/relay   # 101

# 端到端（开发机，docker；测连接、30 秒吞吐、错令牌被拒，命令见 test/relay-e2e.sh）
cloud/iroh-relay/test/relay-e2e.sh
```

`test/relay-e2e.sh` 用同版本官方二进制 + nginx（TLS 终结）+ 上游 `transfer` 示例当一对 `--relay-only` 端点：
断言经中继连接成功、下载 ≥ 1.5 Mbps（默认 256 KiB/s 限速下实测约 2.2 Mbps）、路径是中继、中继字节计数上涨、错令牌拿不到数据。
它把 `config.toml.example` 和 `nginx/relay.808cbd.com.conf` 过一遍 sed 后在临时端口跑，同时钉住这两个文件。

## 容量：出口只有约 5 Mbps

2026-11-26 在目标机上经 SSH 粗测：出口（上行）约 5 Mbps（两次 5.24 / 4.48），入口约 56 Mbps
（可能被测试端限制）。中继转发的**每个字节都占出口**——A→B 和 B→A 各算一次，所以出口 5 Mbps 就是硬上限，
2 vCPU 在这条线上绰绰有余。

估算按每条流 r Mbps、20% QUIC 头/重传余量、双向各一次出口：单路占 `2 × r × 1.2` Mbps，
同时能撑 `C ≈ 5 / (2 × 1.2 × r)`：

| 用途 | 单路 r | 同时能撑 |
| --- | --- | --- |
| 语音（Opus + 头） | 0.5 Mbps | 约 4 路 |
| 远程桌面（日常） | 1 Mbps | 约 2 路 |
| 720p 视频 | 2 Mbps | 约 1 路 |
| 1080p 视频 | 4 Mbps（被限到约 2.1） | 1 路都紧：单连接被限到 2.1 Mbps 后按 ~2 Mbps 跑，2×2.1×1.2≈5.03 正好打满出口，靠降码率/丢帧 |

- 上表是"只跑这一种"的理论值；混跑按 `Σ(2 × r × 1.2) ≤ 5` 加，超了大家一起变差。
- 家庭日常同步（只在打洞失败时经中继）：平均占用 `M × Δ × 8 / 86400` Mbps。
  用掉一半出口（2.5 Mbps）时：Δ=200 MB/天 → 约 135 个家庭；Δ=50 MB/天 → 约 540 个家庭。
  同步是突发的，几个家庭同时推大文件就会瞬间打满，先卡的是正在通话的人。
- 月流量：5 Mbps 跑满约 1.6 TB/月；按 30–50% 利用率约 0.5–0.8 TB/月。阿里云的带宽/流量计费按实际套餐核对。
- 定位是**兜底**：文字、语音、轻量远程桌面可以；视频和批量同步别指望。小文件等打洞成功再传，或错峰。

超了怎么如实提示：

- iroh-relay 1.4.1 没有全局接纳控制/排队（`accept_conn_limit` 还是 TODO）。所有流共享出口，超了就是 QUIC
  拥塞控制分带宽：每条流都变慢、RTT 变高、丢包重传，新连接更难建。它不会拒绝新通话，只会让所有人一起变差。
- 运维：对出口设 4 Mbps（5 Mbps 的 80%）告警，表达式 `rate(relayserver_bytes_sent_total[1m]) * 8 > 4e6`，
  告警文字写"中继出口打满（拥塞，不是故障）"；`relayserver_send_packets_dropped_total` 在涨说明已经在丢。
  这是容量事实，不是中继挂了。
- 产品：连接详情里，路径是中继且 RTT/丢包升高时显示"中继拥塞"，不要显示"网络正常"；
  同步走中继时显示"经中继，较慢"，大文件建议稍后。
- 默认已经开每连接 256 KiB/s 的公平限速；想更严可调到 128 KiB/s（`bytes_per_second = 131072`）。
  这是每连接入方向限速，不是全局配额。
- 扩容：升阿里云出口带宽包或加第二台分流，2 vCPU 不用动。

## 升级

已有的 `config.toml` 不会被覆盖：从 #1058 及以前的版本升级后，手动加一段 `[limits.client.rx]`（默认 256 KiB/s）才有限速。

改 `release.env` 的版本和 sha256（和仓库 `Cargo.lock` 里的 iroh 版本一起），在开发机跑
`test/relay-e2e.sh` 过，再在目标机重跑 `install.sh`。二进制是 rename 替换，失败回滚就改回 `release.env` 重跑。

## 回滚和卸载

- 回滚：`install.sh` 重装旧版本；配置和令牌没被覆盖，不用回滚。
- 卸载：`bash uninstall.sh`（保留配置和令牌）；`bash uninstall.sh --purge` 连用户、数据、证书一起删。
- 目标机上的 nginx 只删 `/etc/nginx/sites-{available,enabled}/relay.808cbd.com` 这一个 vhost，主人其他站不动。

## 排错

- `systemctl status wonton-iroh-relay` 报 `access.shared_token must not be empty`：`relay.env` 缺令牌或为空。
- 启动报 TLS/cert：`/etc/wonton-iroh-relay/tls/` 里没有证书，先跑 certbot 或 `install.sh`。
- 公网 502：中继没起或没监听 8742，看 journal。
- WebSocket 101 但很快断：nginx 少了 `proxy_read_timeout`/`Upgrade` 头（对照 `nginx/relay.808cbd.com.conf`）。
- QAD 不生效：安全组没放 UDP 7842，或 `quic_bind_addr` 不是 `0.0.0.0:7842`。
- 401/拿不到数据：客户端令牌和 `relay.env` 不一致。

## 版本和许可

- 固定上游 `iroh-relay v1.4.1`（和仓库 `Cargo.lock` 的 iroh 1.4.1 同版本），官方 release asset，sha256 校验，见 `release.env`。
- 上游许可：MIT OR Apache-2.0。二进制原样分发，未改源码。
