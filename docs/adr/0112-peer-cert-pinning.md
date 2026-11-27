# ADR-0112 没配公网入口的云吞：经 iroh 配对互钉证书的联邦

- 状态：已接受（2026-11-22）。主人 2026-11-21 选定方案 B
- 补充 [ADR-0090](0090-public-access.md)：总开关关着时，经 iroh 配对过的云吞照常联邦（0090 第 2 节补一条）
- 推翻 [ADR-0051](0051-friends-and-plugin-sharing.md) 第 5 条“两种模式不互通，不做配对证书钉扎”的后半句
- 推翻 [ADR-0050](0050-cross-server-chat.md) 的“不终结 TLS”与 `crates/wonton-tunnel/src/federation/mod.rs` 的同一说法

## 背景

没配公网入口的安装盘上没有能验证的证书。对端一律拨 https，ClientHello 落到明文 HTTP 上（ADR-0090、C3），所以这类云吞连经 iroh 配对的朋友也聊不了。

Tuwunel v2.0.0 出站联邦用 reqwest + rustls + 平台根，只有一个全局的 `[global.network.http].allow_invalid_tls_certificates`（v1 是 `[global].allow_invalid_tls_certificates`，键名相同、路径变了），没有按服务器钉证书或单独信任根的能力（调研 `tasks/people-fed-certpin-research`）。Matrix 规范允许服务器覆盖可信根。

## 决策

### 1. 入口仍是首选，没入口的云吞也能和配对过的朋友聊

公网入口有能验证的证书，优先。没入口时不对公网开 8448、也不自签“公共证书”（8448 变回网络内的明文监听，暴露面只在核心网络里）；配对过的云吞走 iroh 隧道，由核心按配对表校验。朋友以外的人和公网服务器照旧连不上。

### 2. 核心在隧道两端终结 TLS

没入口时链路是“本机 Tuwunel → CONNECT → 核心 → iroh → 对方核心 → 对方 Tuwunel”，明文流两头都在核心手里。出站时核心对本机 Tuwunel 当 TLS 服务端，对对方当客户端；入站反过来。两段握手都通过才转发，Tuwunel 只和核心说话，不改 Tuwunel 源码。

### 3. 钉对方的叶子证书，对外出示的也是自签叶

配对时经已认证的 iroh 通道互换对方自签叶证书的 SPKI SHA-256 和整张 DER，收到即落盘。不钉 CA 私钥、不钉公钥基础设施：CA 私钥被拿走能签任意名字，叶不能。配对时不做 TOFU：没有 pin 不放行；已有配对升级的第一段按 §5b“待钉”放行。

**对外的证书是自签叶**（SAN 是本机服务器名），pin 就是这张叶本身。核心出的私有 CA 只用于一件事：给**本机 Tuwunel 看的那张**证书签名，让 Tuwunel 按正常链验证核心；这张证书按 CONNECT 里的目标服务器名现签（SAN 要和对端名字一致），所以只存 CA、不存固定的叶。这样 rustls 的 `WebPkiServerVerifier` 在对方侧根库只放钉住的叶就成立（实测：根库放 CA 签出的叶 → `BadSignature`；放签发它的 CA 或自签叶 → 通过）。

### 4. 校验怎么做

对方侧：根库只放钉住的那张叶，用 `WebPkiServerVerifier`，另单独比 SPKI 是否等于 pin（只靠根库不够，见上）、并校 Hostname（规范要求证书名对得上）。

本机 Tuwunel 侧：信任文件 = 系统根 + 本机 CA，用 `SSL_CERT_FILE` 指过去，`allow_invalid` 关掉（有已有配对时按 §5b 留窗口，钉齐或窗口到期再关；本机 Tuwunel 侧的证书由核心用本机 CA 签，窗口只影响老对端）。系统根在装机时从镜像抄一份，升级镜像后重抄，写进安装产物。

私用名（`.wonton.internal`）的安装同样纳入：不再让 Tuwunel 自己挂自签 TLS，也不会双重 TLS（ADR-0051 第 5 条同步改）。

### 5. 轮换

证书装机时生成，`notAfter` 给 5 年。轮换由主人触发或随升级：新 key+叶经 `PeerMessage::Certificate` 同步，双方 pin 集合同时收旧和新，收到对方对 `Certificate` 的回执（同一个消息带 ack）或满 7 天宽限后只留新，宽限期仍接受旧叶；到期不自动续也不静默放行，握手失败按“证书过期”如实报。对端离线超过 7 天宽限、只剩旧 pin 时，首次握手会失败，重配对即恢复。

### 5b. 已有配对怎么升级（不做 TOFU 会静默断，这里写清）

- 旧记录没有 pin：升级后第一次握手按「待钉」放行——临时宽松，但仍校 Hostname（端点身份证明不了名字），并记录“待钉”；同时在已认证的配对通道上发 `PeerMessage::Certificate`，收到就写 pin、转严格。
- 待钉窗口 = 两个核心版本（由装机的核心版本决定，开关写进 `peers` 状态；升级到带 pin 的支持版本后第一个正式版本内关闭），不是固定天数；窗口由核心自己在启动时判，主人也能提前关：直接改 `secrets/tuwunel/tuwunel.toml` 的 `allow_invalid_tls_certificates = false`，核心两秒内认这个文件值并热加载，窗口还开着也不覆盖（上次自动写的值记在 `core/window-applied`，重启后仍认得出是手改）；对端都钉齐后自动交回自动，新的待钉对端会重新开。老版本首次升级、还没有 `core/window-applied` 时，停机期间手改的值第一轮可能被自动值覆盖一次，之后不会。
- 一端升级、另一端仍自签：升级端按待钉接受自签，不静默失败；对端升级后自动转严格。
- 窗口到期仍拿不到 pin：保持待钉并如实报「对端还没升级」，不静默放行也不直接断。
- 老对端不认识 `Certificate`：重新配对时把首次握手的证书按“升级专用 TOFU”钉住（只在这一次重新配对时），否则“重新配对”会空。
- 私用名安装的 `allow_invalid` 只在这个窗口里保留，pin 齐或窗口关就关掉；本机侧证书由核心用本机 CA 签，窗口只针对老对端。

### 6. 删好友即吊销

删记录同时去掉 pin 和名单：出站查不到回 403，入站按 endpoint ID 拒，对方离线也立即失效；重连时握手再失败一次。

### 7. 入口例外要有状态，不靠猜

有入口的安装仍用入口的公共证书，配对流入站投到入口 8449，对方按公共根校验；出站走 pin。`PeerRecord` 加 `peer_entry: none|public`，由 `PeerMessage` 双向同步；切换期两边都放行（既接受 pin 也接受公共根）。对端已开入口而本机仍按 pin 时，握手失败并报“对端换了入口，请重配对”，不静默。

### 8. 和 E2EE、签名无关

TLS 本来逐跳，X-Matrix 的 Ed25519 签在 HTTP 层，Olm/Megolm 在载荷里，都不看 TLS。Host 头保持原样。

## 备选（2026-11-21 主人选的 B）

- A：维持现状，没入口就提示先配入口，聊不了。
- B（本文）：配对时互钉证书。
- C：Tuwunel 自签 + 全局放宽校验，所有服务器都认自签。

C 最省事，但全局放宽一旦进 Tuwunel 就不分服务器；B 只在配对过的名字上生效，别的服务器照旧。

## 代价与风险

- 核心从此是 TLS 端点、看得见联邦明文（ADR-0050 的“看不到内容”不再成立）。这是自证的代价：核心本来就持有全部密钥。
- 自写验证器是安全边界：必须用 `WebPkiServerVerifier` 加只含钉叶的根库、并单独比 SPKI 和名字，不能只比指纹。
- 信任文件写坏 Tuwunel 拒启动；空库在安装时报错。
- 系统根是抄来的，镜像更新后会旧；安装产物记下来源镜像 digest，升级时重抄。
- 私钥在宿主盘（0600、只读挂载），备份和导出跟着装机。
- 入口例外要同步状态，漏同步会两边规则不一致。
- 443 的 well-known CONNECT 现在 502 回退 8448，终结后行为不能变。

## 待验证

- `SSL_CERT_FILE` 在 Tuwunel 镜像里实际生效；`probe_tls` 对空库报错的行为（单容器补测）。
- 两段握手对启动顺序、重连、并发的表现。
- 本 ADR 没有实测握手，依据是源码、规范原文和一次 verifier 内存实验。

## 新能力检查单

- 契约：`PeerMessage::Certificate`、`PeerRecord.peer_entry`（core 的配对记录，带 serde default）。
- 提供方：`wonton-tunnel` 的 `federation/{outbound,server}.rs`；消费方：`wonton-core` 的 peers 与安装。
- 契约测试：pin 不匹配、SPKI 不符、名字不符各自失败；入口切换期两边都放行。

## 依据

- 调研：`tasks/people-fed-certpin-research`（Tuwunel v2.0.0 无按服务器钉证书能力；`SSL_CERT_FILE` 生效）。
- Matrix server-server-api：Destination server must provide a TLS certificate signed by a known CA；Requesting servers are ultimately responsible for determining trusted CAs；可覆盖可信根、可按白名单跳过。
- C3（#891）：无入口的安装不自签、不开公网 8448。

## 主人已定（原话，2026-11-21）

方案 B，就是配对的时候安全，把彼此的自签证书记住，只认对方，不对公网开，也不降别处的校验。

负责人归纳：安全来自二维码配对加钉证书；问设置时说的是公网暴露；设好后能直接给朋友发消息。
