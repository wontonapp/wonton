# wonton-tunnel

iroh 1.x 应用内隧道，服务端与客户端共用。设计和验证结论见
[ADR-0004](../../docs/adr/0004-iroh-tunnel.md)。

- **服务端**：`TunnelServer`（独立运行）或 `TunnelProtocol`（挂到核心已有的 iroh `Router` 上）。
  只接受 `Allowlist` 里的设备 endpoint ID，未配对的设备在握手后立即被关闭连接；吊销设备会断开它现有的连接。
  每条隧道流有两种去向：
  - 转发到本机的上游 HTTP 服务（`TunnelServer::spawn`、`TunnelProtocol::new`），示例和开发工具用这种。
  - 交给嵌入方（`TunnelServer::spawn_embedded`、`TunnelProtocol::embedded`）：嵌入方从 `TunnelListener`
    取出 `PeerStream`，它实现 tokio 的 `AsyncRead` / `AsyncWrite`，`remote_id()` 是 QUIC 握手认证过的设备
    endpoint ID。核心用它在进程内服务网关，并把设备身份交给网关。正常结束要 `shutdown`；没有 shutdown 就
    丢弃时复位流，客户端不会把被截断的数据当成正常结束。
- **客户端**：`TunnelClient` 让所有请求复用一条连接，断开后下一次使用时自动重连，建连的时限和退避都在它这里（ADR-0032），
  `retry_now` 跳过等待；不设在线超时门禁；
  用 `watch` 报告 `TunnelStatus`，只保留最新状态。App 不可见时调用 `set_suspendable(true)`（保持连接也在内），回前台检查后关掉：
  双时钟检测执行断档，在首个新流之前开始共享检查，不等壳的前台事件、不重放请求。判据见 ADR-0032。App 回前台也调用 `revalidate`：服务端可能已经
  放弃了这条连接而手机收不到关闭帧。快直连限时 1.5 秒，无慢证据的中继 3 秒，经主动 probe 证实的慢路径 8 秒；
  没回应就换新连接，不等 30 秒空闲超时。中继 3 秒按 ADR-0004 历史数据取值，未在真实中继复测；判据与取舍见 ADR-0032。
  平时定时调用 `probe`：心跳这些小包还能往返时 QUIC 算连接活着，请求却可能
  过不去，`probe` 限时做一次真实的往返，没有回应就断开，状态和路径随之更新（ADR-0004 的“卡住的连接”）。
  `LocalProxy` 在回环地址上监听，把本机 HTTP 客户端的连接经隧道转发
  过去，用法见下一节。

## 本地代理的两种用法

本地代理监听回环地址，同一台设备上的其他 App 也连得上；服务端只认设备的配对身份，经代理来的请求都算
这台设备的。所以 `LocalProxy::bind` 要明确选一种 `ProxyAccess`：

- **`Credential { credential, domain }`（App 用）**：HTTP 正向代理，只接受带本次会话凭据的连接，只转发家里
  服务器基础域名（`domain`）和它的子域名。凭据由 `ProxyCredential::generate()` 每次启动随机生成（用户名
  `wonton`，密码 256 位随机数），只交给本 App 的 WebView 和 HTTP 客户端。代理只解析每条连接的第一个请求头，
  之后原样转发，`Host` 不变：
  - 没带凭据或凭据不对：回 407 和 `Proxy-Authenticate: Basic`，不建隧道连接。
  - 目标不在基础域名下（`CONNECT` 看请求行，绝对形式看 URL，源站形式看 `Host`）：回 403，不建隧道连接，
    外部资源不会被送回家。
  - `CONNECT host:port`（iOS 的 WKWebView 只能用这种代理）：回 200，之后的字节原样进隧道。
  - 绝对形式的请求行（`GET http://host:port/path`，Android WebView 和一般 HTTP 客户端访问 http 地址时这样发）：
    改成源站形式，`Host` 取自 URL，去掉 `Proxy-Authorization` 和 `Proxy-Connection`，加 `Connection: close`，
    让服务端回完这个请求就关闭连接。每条连接只有一个请求，凭据不会随后续请求进入隧道。协议升级的请求
    保留原来的 `Connection`。
- **`Open`（开发工具和测试用）**：不校验调用方，每条连接原样转发，把代理当成服务端访问即可（把 base URL
  换成 `http://<代理地址>`）。本机任何进程都能借用这台设备的配对身份。

服务端的下载方向真正 EOF 后，本地代理最多再等 2 秒让上传收尾，随后取消上传并结束发送方向，
不永久等客户端关掉写端，也不因迟到的上传错误复位已完整收到的响应。这不限制响应体传输时长：
没有下载 EOF 的 keep-alive、WebSocket 和双向长连接仍然保持；客户端先结束上传时，下载也能继续。

## 线协议（ALPN `wonton/tunnel/0`）

做法参考 dumbpipe：一条 QUIC 连接上每条双向流对应一条 TCP 连接，流里原样转发字节。

1. 客户端开流后先写 1 字节请求类型：`1` 转发、`2` ping。转发请求紧接着发送 TCP 字节，不等服务端回应。
2. 服务端先回 1 字节状态：`0` 成功（转发请求随后是上游的原样字节）、`1` 连不上上游（交给嵌入方时是嵌入方
   已停止接收）、`2` 不支持的请求类型。
3. 服务端拒绝未配对或已吊销的设备时，用错误码 `403` 关闭整条连接。
4. 转发中途出错时，两端用错误码 `1` 复位流，对端不会把被截断的数据当成正常结束。

## 本地代理的错误响应

隧道出错、上游还没返回任何数据时，代理回一个 HTTP 错误，`x-wonton-tunnel-error` 头标明原因：

| 原因 | 状态码 | `x-wonton-tunnel-error` |
| --- | --- | --- |
| 服务端拒绝本设备（未配对或已吊销） | 403 | `unauthorized` |
| 连不上服务端、连接中断或超时 | 502 | `transport-unavailable` |
| 隧道是通的，服务端连不上上游 | 503 | `upstream-unavailable` |
| 两端协议不匹配 | 502 | `protocol` |
| 客户端已关闭 | 502 | `closed` |
| 正向代理：没带会话凭据或凭据不对 | 407 | `proxy-unauthorized` |
| 正向代理：目标不在家里服务器的域名下 | 403 | `foreign-host` |
| 正向代理：请求头不合法或超过 64 KiB | 400 / 431 | `bad-request` |

上游已经开始返回数据后再出错，只能断开连接。`CONNECT` 已经回了 200 之后的隧道错误，作为里面第一个请求的
响应回给客户端。

## 联邦转发（ALPN `wonton/federation/0`）

配对过的两台服务器的 Matrix 联邦流量经 iroh 互转（ADR-0050）。挂在和隧道、配对同一个 endpoint 上：
对外只有一个 UDP 端口、一个 endpoint ID。线协议和隧道的一样（1 字节请求类型、1 字节状态、出错复位流），
白名单分开：**服务器名单**只放行联邦的 ALPN，**设备名单**只放行隧道的 ALPN——配对过的手机进不了联邦，
配对过的服务器也进不了 App 的隧道，互相都以 403 关闭。

- **入站**：`FederationProtocol`（`TunnelServer::spawn_embedded_with_pairing_and` 挂到核心的 router 上）。
  名单认过的连接，每条双向流原样接到本机 Tuwunel 的联邦端口（8448），不解 TLS。对方从名单里被去掉时，
  它的现有连接随即被关闭。
- **出站**：`FederationForwarder::bind` 在 Tuwunel 的网络上监听（Tuwunel 的 `/etc/hosts` 把每个配对过的
  服务器名指向这个地址）。每条连接先读 TLS ClientHello 取 SNI（rustls 的 Acceptor，只解析、不回握手、
  不终结 TLS），SNI 就是对方的 Matrix 服务器名；按嵌入方提供的查找函数（服务器名 → 配对时记下的
  endpoint ID 和地址）选路，把已读的 ClientHello 和后续字节原样转过去。查找函数返回 `None`（名字不在
  配对名单里）或开头不是带 SNI 的 ClientHello：关闭连接，分别记一次 `unknown_server` /
  `invalid_client_hello`。
- **每台对方服务器一条共享连接**（`TunnelClient` 的形状：复用、退避、探活）。断了下次用时按退避重连；
  定时 ping 探活，周期和超时由 `bind` 的参数给。名字从查找函数里消失（对方被删除）时，连接和探活一起停。
- **状态**：`watch_status` 给出每台对方服务器的 `PeerStatus`。`Connected` 是最近一次探活或数据往返成功；
  `Offline { since, last_error }` 是连不上（传输问题，不是对方的态度）；`RemovedByPeer` 是握手后被对方用
  403 关闭（对方名单里没有本机），对方的名单恢复后自动回到 `Connected`。Tuwunel 分不清“对方离线”和
  “对方拒绝”，嵌入方按这里的状态给 App 出不同的提示（ADR-0050 的“状态”）。

## 服务器配对（ALPN `wonton/peer/0`）

两台服务器的主人在 App 里配对服务器（ADR-0050）：一方出一张一次性码（`wonton://peer?…`，带 endpoint ID、
直连和中继地址、Matrix 服务器名、密钥），另一方扫码后经这条 ALPN 连过来交换身份，出码方的主人在
“Wonton”私聊里点同意，两边才记成已配对。和设备配对（[`pair`]）、联邦挂在同一个 endpoint 上。

- **码**：`PeerPairingCode`，格式和取地址方式照设备配对码；一次性，过期作废。
- **消息**：一条双向流一个 JSON 消息加一条回应（`PeerMessage` / `PeerReply`）。扫码方发 `scan`
  （密钥、自己的服务器名、主人的用户 ID 和显示名、能被回连的地址），出码方回“等主人同意”附自己的
  身份，或按原因拒绝（`PeerRejectReason`）。
- **结果**：主人点了同意或拒绝后，出码方用扫码请求里的地址连回扫码方发 `decision`。扫码方只接受
  自己扫过的那个 endpoint ID（[`PeerPairHandler::expecting_decision`] 说了算），别的来源一律回错。
- **删除通知**：`removed` 消息，尽力而为；对方离线时，它以后在联邦握手被拒时知道。
- **嵌入方**：实现 [`PeerPairHandler`]（验码、记配对、等主人回答），用
  [`spawn_embedded_with_pairing_and`](crate::TunnelServer::spawn_embedded_with_pairing_and) 把这条 ALPN
  和其他协议一起挂上。行为测试在 `tests/tunnel/peer_pair.rs`。

## 示例与实地测试

```bash
# 服务端：把隧道流量转发到本机 8080
cargo run -p wonton-tunnel --example tunnel-server -- --upstream 127.0.0.1:8080 --allow <客户端 ID>
# 客户端：打印本机 ID；起本地代理；测直连/中继比例和延迟
cargo run -p wonton-tunnel --example tunnel-client -- id
cargo run -p wonton-tunnel --example tunnel-client -- proxy --server <服务端 ID>
# 像 App 一样：带会话凭据的正向代理，只转发 --domain（默认 wonton.localhost）下的地址，启动时打印凭据
# （curl -x http://127.0.0.1:8081 -U wonton:<密码>）
cargo run -p wonton-tunnel --example tunnel-client -- proxy --server <服务端 ID> --auth
cargo run -p wonton-tunnel --example tunnel-client -- probe --server <服务端 ID> --relay <中继 URL>
```

`scripts/field-test.sh` 把服务端部署到一台 SSH 主机（进程到时自动退出），在本机跑测量，再清理，
用来在真实网络（例如手机热点）下复测 ADR-0004 的验证结果。

行为测试在 `tests/tunnel/`（转发到 TCP 上游、交给嵌入方、App 用的本地代理各一组），只走本机回环地址，
不依赖外网。
