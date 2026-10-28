# wonton-tunnel

iroh 1.x 应用内隧道，服务端与客户端共用。设计和验证结论见
[ADR-0004](../../docs/adr/0004-iroh-tunnel.md)。

- **服务端**：`TunnelServer`（独立运行）或 `TunnelProtocol`（挂到核心已有的 iroh `Router` 上）。
  只接受 `Allowlist` 里的设备 endpoint ID，未配对的设备在握手后立即被关闭连接；吊销设备会断开它现有的连接。
  每条隧道流转发到本机的上游 HTTP 服务。
- **客户端**：`TunnelClient` 让所有请求复用一条连接，断开后下一次使用时自动重连，不设在线超时门禁；
  用 `watch` 报告 `TunnelStatus`，只保留最新状态。`LocalProxy` 在回环地址上监听，本机的 HTTP 客户端
  把 base URL 换成代理地址即可，相当于直接访问服务端的上游。

## 线协议（ALPN `wonton/tunnel/0`）

做法参考 dumbpipe：一条 QUIC 连接上每条双向流对应一条 TCP 连接，流里原样转发字节。

1. 客户端开流后先写 1 字节请求类型：`1` 转发、`2` ping。转发请求紧接着发送 TCP 字节，不等服务端回应。
2. 服务端先回 1 字节状态：`0` 成功（转发请求随后是上游的原样字节）、`1` 连不上上游、`2` 不支持的请求类型。
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

上游已经开始返回数据后再出错，只能断开连接。

## 示例与实地测试

```bash
# 服务端：把隧道流量转发到本机 8080
cargo run -p wonton-tunnel --example tunnel-server -- --upstream 127.0.0.1:8080 --allow <客户端 ID>
# 客户端：打印本机 ID；起本地代理；测直连/中继比例和延迟
cargo run -p wonton-tunnel --example tunnel-client -- id
cargo run -p wonton-tunnel --example tunnel-client -- proxy --server <服务端 ID>
cargo run -p wonton-tunnel --example tunnel-client -- probe --server <服务端 ID> --relay <中继 URL>
```

`scripts/field-test.sh` 把服务端部署到一台 SSH 主机（进程到时自动退出），在本机跑测量，再清理，
用来在真实网络（例如手机热点）下复测 ADR-0004 的验证结果。

行为测试在 `tests/tunnel.rs`，只走本机回环地址，不依赖外网。
