# ADR-0095 没配公网入口的云吞：经 iroh 配对互钉证书的联邦

- 状态：提议（2026-11-22）。主人 2026-11-21 选定方案 B
- 补充 [ADR-0090](0090-public-access.md)：总开关关着时经 iroh 配对的云吞照常联邦
- 相关：[ADR-0051](0051-friends-and-plugin-sharing.md)、[ADR-0090]（C3 的已知限制）

## 背景

没配公网入口的安装盘上没有能验证的证书。对端一律拨 https，ClientHello 落到明文 HTTP 上（ADR-0090、C3），所以这类云吞连经 iroh 配对的朋友也聊不了。

Tuwunel v1.9.4 出站联邦用 reqwest + rustls + 平台根，只有一个全局 allow_invalid_tls_certificates，没有按服务器钉证书或单独信任根的能力（调研 `research/people-fed-certpin.md`）。Matrix 规范允许服务器覆盖可信根。

## 决策

### 1. 入口仍是首选，没入口的云吞也能和配对过的朋友聊

公网入口有能验证的证书，优先。没入口时不对公网开 8448，也不自签；配对过的云吞走 iroh 隧道，由核心按配对表校验。除朋友以外的人和公网服务器照旧连不上。

### 2. 核心在隧道两端终结 TLS

链路本来就是“本机 Tuwunel → CONNECT → 核心 → iroh → 对方核心 → 对方 Tuwunel”，明文流两头都在核心手里。出站时核心对本机 Tuwunel 当 TLS 服务端，对对方当客户端；入站反过来。两段握手都通过才转发，Tuwunel 只和核心说话，不改 Tuwunel 源码。

### 3. 钉对方的叶子证书

配对时经已认证的 iroh 通道互换对方证书的 SPKI SHA-256，收到即落盘。不钉私钥、不钉 CA：CA 私钥被拿走能签任意名字，叶不能。不做 TOFU：没有 pin 不放行。

### 4. 校验怎么做

每台装一个私有 CA（核心持私钥、给对方名字签叶证书）。Tuwunel 的信任文件 = 系统根 + 本机 CA，用 SSL_CERT_FILE 指过去，去掉有名字安装的全局 allow_invalid。校验在核心：用 rustls 的 WebPkiServerVerifier，根库只放钉住的证书，另单独校 Hostname（规范要求证书名对得上）。

### 5. 轮换

证书装机时生成一次，不自动过期。轮换由主人触发或随升级：新 key+cert 经 PeerMessage::Certificate 同步，双方 pin 集合同时收旧和新，双方确认或 7 天后只留新，宽限期仍接受旧 key。

### 6. 删好友即吊销

删记录同时去掉 pin 和名单：出站查不到回 403，入站按 endpoint ID 拒，对方离线也立即失效；重连时握手再失败一次。

### 7. 入口例外

有入口的安装仍用入口的公共证书，配对流入站投到入口 8449，对方按公共根校验；出站走 pin。同一条配对存在两套规则，状态和文档都要写明“已开入口时不走 pin”。

### 8. 和 E2EE、签名无关

TLS 本来逐跳，X-Matrix 的 Ed25519 签在 HTTP 层，Olm/Megolm 在载荷里，都不看 TLS。Host 头保持原样。

## 备选（2026-11-21 主人选的 B）

- A：维持现状，没入口就提示先配入口，聊不了。
- B（本文）：配对时互钉证书。
- C：Tuwunel 自签 + 全局放宽校验，所有服务器都认自签。

C 最省事，但全局放宽的文件进 Tuwunel 一次就不分开；B 只在配对过的名字上生效，别的服务器照旧。

## 代价与风险

- 自己写的验证器是安全边界：必须用 WebpkiServerVerifier 加只含钉证书的根库并单独校名字，不能只比指纹。
- 信任文件写坏 Tuwunel 拒启动；空库要在安装时当错误报出来。
- 私钥在宿主盘（0600、只读挂载），备份和导出跟着装机。
- 入口例外是两套规则，漏写会让人以为 pin 总生效。
- 443 的 well-known CONNECT 现在 502 回退 8448，终结后行为不能变。

## 待验证

- SSL_CERT_FILE 在 Tuwunel 镜像里实际生效；probe_tls 对空库报错的行为（单容器补测）。
- 两段握手对启动顺序、重连、并发的表现。
- 本 ADR 没有实测，依据是源码和规范原文。

## 依据

- 调研：tasks/people-fed-certpin-research 的结论（Tuwunel v1.9.4 无按服务器钉证书能力）。
- Matrix server-server-api：Destination server must provide a TLS certificate signed by a known CA；Requesting servers are ultimately responsible for determining trusted CAs；可覆盖可信根、可按白名单跳过。
- C3：无入口的安装不自签、不开公网 8448。


## 主人已定（原话，2026-11-21）

方案 B，就是配对的时候安全，把彼此的自签证书记住，只认对方，不对公网开，也不降别处的校验。

负责人归纳：安全来自二维码配对加钉证书；问设置时说的是公网暴露；设好后能直接给朋友发消息。
