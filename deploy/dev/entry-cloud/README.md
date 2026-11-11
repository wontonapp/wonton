# 本地家名与 SNI 入口云

只用于 `.test`、本地测试 CA。不会修改宿主 DNS、信任库或服务；所有发布端口仅绑定
`127.0.0.1` 且大于 1023。Docker 网络为 `internal`，无公网 DNS/ACME 回退。
前提：Docker Compose v2（支持 `up --wait`）、Python 3、OpenSSL、curl；仓库包含
`wonton-home` workspace 成员及更新后的 `Cargo.lock`。

## 为什么有一层家名服务

转发、DNS、证书签发和 iroh 中继使用未修改的上游组件：frps、PowerDNS、dnsdist、acme-dns、Pebble、iroh-relay。
它们没有“iroh endpoint 签名 → 一次性邀请 → 永不复用的家名 → 独立 acme-dns 账号 →
CAA 账号绑定 → frp 准入与停用”的统一接口。因此 `wonton-home` 只实现这层管理策略，
不实现 DNS 服务器、ACME CA 或 TLS 代理。私钥与业务流量留在家里。

```text
家里 ── HTTPS 签名管理请求 ── HAProxy :8443 ── wonton-home :8080
家里 frpc ── 强制 TLS 控制连接 ── frps :443 ── 内部准入钩子 :8081
联邦 ── TLS ClientHello/SNI ── frps 同一个 :443 ── 家里的 TLS 入口
家里 lego ── DNS-01 更新 ── HAProxy /acme/update ── 真实 acme-dns API
Pebble + 家里 DNS ── Recursor ── dnsdist ── PowerDNS / auth 子区 acme-dns
```

frps 的控制端口和 HTTPS vhost 都是 443，`proxyBindAddr` 与 `bindAddr` 相同。
**frpc 必须设 `transport.tls.disableCustomTLSFirstByte = false`**：控制连接使用 frp
自定义首字节，普通 TLS ClientHello 才进入 SNI 路由。`transport.tls.force=true`；
云端只有 relay 控制面证书，**没有任何家的叶子证书或私钥**。
frps dashboard/Prometheus 仅监听共享网络命名空间的 `127.0.0.1:7500`，随机密码；
管理服务每 30 秒读真实指标，指标不可用时按服务策略拒绝准入。
管理服务启动不重试 PDNS/指标：Compose 必须先通过 PowerDNS API TCP 健康检查，执行真实
zone/基础记录 REST bootstrap，再通过 acme-dns `/health` 与带认证的 frps `/metrics` 健康检查，
才启动 home。仅 `service_started` 不够；这些检查不依赖 home 钩子，避免启动环。

## 获取名额、启动与拆除

在仓库根目录（不要同时使用另一个栈的名额/子网）：

```bash
export COMPOSE_PROJECT_NAME=wonton-entry-cloud
~/wonton-dev/stack-slot.sh acquire --project "$COMPOSE_PROJECT_NAME" fr-entry-cloud
export WONTON_DEV_CORE_SUBNET="$(~/wonton-dev/stack-slot.sh subnet fr-entry-cloud)"
export WONTON_HOME_ZONE=homes.test
# 可选：在 prepare 前配置下表的网络、地址和宿主端口。
bash deploy/dev/entry-cloud/dev.sh up
```

`up` 先生成配置和控制面测试 CA，再按摘要取镜像、从仓库根构建管理服务，启动真实服务，
等待健康门，最后经验证过的 Pebble management HTTPS 导出本次运行的签发根。
启动失败不自动拆栈或放名额，先检查 `dev.sh compose logs`，不要在半栈还在时 release。
只生成配置、不启动：`dev.sh prepare`。脚本不自动 acquire/release。

已有通过 cargo-slot 构建的管理服务 ELF 时，在 `prepare/up` 前导出
`WONTON_HOME_BINARY=/absolute/path/to/wonton-home`，脚本自动加 `compose.binary.yaml` 并用
`up --no-build`；变量持久化在 stack.env。默认运行时 Debian bookworm（glibc ≤ 2.36）。
Arch 新 glibc 编译的 ELF 必须同时设 `WONTON_HOME_RUNTIME_IMAGE=官方兼容镜像@sha256:摘要`，
例如由集成负责人选定的 Arch Linux image；不接受未固定摘要的覆盖。运行时必须包含 bash，且
`compose.binary.yaml` 会把生成的控制面 CA 挂到 `/control-ca.pem` 并通过 `SSL_CERT_FILE` 注入，供
预构建 ELF 的严格 TLS 校验使用。
未设置 ELF 才使用 Dockerfile 构建。工作站验证的 Cargo 编译仍须由 `cargo-slot.sh` 调度；
不要用 Docker build 绕过名额。
`WONTON_CORE_BINARY` 属于家庭栈，本云栈不启动 core，不消费这个变量。

```bash
bash deploy/dev/entry-cloud/dev.sh down       # 保留数据卷与 .runtime
~/wonton-dev/stack-slot.sh release fr-entry-cloud
```

重新启动仍要先 acquire 并设置所得子网；若名额变化，旧的显式 IP 必须一并更新，
最简单的是在不需要数据时先 destroy，再 acquire 新名额。保留数据时不要改 zone。
彻底移除仅此项目的容器、网络、命名卷与生成的控制面密钥：

```bash
bash deploy/dev/entry-cloud/dev.sh destroy
~/wonton-dev/stack-slot.sh release fr-entry-cloud
```

命名卷是 `${COMPOSE_PROJECT_NAME}_pdns`、`_acme-dns`、`_home`；`destroy` 用
`docker compose down --volumes`，不 prune、不移除别的项目。不提交 `.runtime`。
先拆除挂在云网络上的家庭容器，否则 Docker 不能删除仍被使用的网络。

## 环境与地址

`prepare` 接受环境变量并写入 `.runtime/stack.env`；后续命令使用该文件。
同一台宿主的多个云栈须使用不同项目名、子网和发布端口。所有 IPv4 地址须在私有 `/24`
内，互不相同，不能占 `.0`、网关 `.1` 或 `.255`。默认地址以名额子网为准：

| 环境变量 | 默认/含义 |
| --- | --- |
| `COMPOSE_PROJECT_NAME` | `wonton-entry-cloud` |
| `WONTON_DEV_CORE_SUBNET` | **必填**，stack-slot 提供 |
| `WONTON_HOME_ZONE` | `homes.test`，仅允许小写 `.test` |
| `WONTON_ENTRY_NETWORK` | `${COMPOSE_PROJECT_NAME}-entry` |
| `WONTON_HOME_BINARY` | 可选，cargo-slot 预构建的绝对 ELF 路径；不在容器内编译 |
| `WONTON_HOME_RUNTIME_IMAGE` | 可选，预构建 ELF 的兼容运行时，必须 `@sha256:` 固定摘要 |
| `WONTON_ENTRY_DNSDIST_IP` | 子网 `.10`，权威入口 TCP/UDP 53 |
| `WONTON_ENTRY_PDNS_IP` | `.11`，PowerDNS/API 仅内部 |
| `WONTON_ENTRY_ACME_DNS_IP` | `.12`，auth 区 DNS/API 仅内部 |
| `WONTON_ENTRY_RESOLVER_IP` | `.13`，家庭及 Pebble 的递归 DNS 53 |
| `WONTON_ENTRY_PEBBLE_IP` | `.14`，ACME 14000、management 15000 |
| `WONTON_ENTRY_RELAY_IP` | `.15`，relay 443，同时承载 home :8080/:8081 |
| `WONTON_ENTRY_IROH_IP` | `.18`，iroh 中继 443（家配对用，证书由控制 CA 签） |
| `WONTON_ENTRY_API_IP` | `.16`，管理 TLS :8443 |
| `WONTON_ENTRY_API_PORT` | 宿主 8443 → API 8443 |
| `WONTON_ENTRY_RELAY_PORT` | 宿主 9443 → relay 443 |
| `WONTON_ENTRY_DNS_PORT` | 宿主 15353 → Recursor 53 TCP/UDP |
| `WONTON_ENTRY_AUTH_PORT` | 宿主 15354 → dnsdist 53 TCP/UDP |
| `WONTON_ENTRY_PEBBLE_PORT` | 宿主 14000 → Pebble 14000 |
| `WONTON_ENTRY_PEBBLE_MANAGEMENT_PORT` | 宿主 15000 → Pebble management 15000 |

这些 `_PORT` **只改宿主发布端口**，家庭容器仍用 8443/443/53/14000；注册响应也用
容器网络端口。`prepare` 创建 PowerDNS zone、基础 NS/A 和 `auth.<zone>` NS 委派；
家名的 A、通配 A、CAA 和 `_acme-challenge` CNAME 由管理服务经 PowerDNS REST 管理。
Recursor 的所有查询只转到 dnsdist；dnsdist 对区外名字 REFUSED，不接公共递归。
Recursor 必不可少：Pebble 的 DNS-01 实现只读回答中的 TXT，不自己追 CNAME。
不能用假的 TXT / `pebble-challtestsrv` / `PEBBLE_VA_ALWAYS_VALID=1` 代替这条链路。

## 家庭网络与信任配置

在云栈启动后，在家里的 Compose 添加一个外部网络（以下 `core`、`tuwunel`、`frpc`
代表家庭实际的相应 service；原有私有网络要保留）：

```yaml
networks:
  entry-cloud:
    external: true
    name: ${WONTON_ENTRY_NETWORK}
services:
  core:
    networks: [default, entry-cloud]
    dns: ["${WONTON_ENTRY_RESOLVER_IP}"]
    volumes:
      - ${ENTRY_CLOUD_CA_DIR}:/entry-cloud-ca:ro
    environment:
      WONTON_CLIENT_SERVICE: https://names.${WONTON_HOME_ZONE}:8443
      WONTON_CLIENT_CA_FILE: /entry-cloud-ca/control-ca.pem
      WONTON_ACME_DIRECTORY: https://pebble.${WONTON_HOME_ZONE}:14000/dir
      WONTON_ACME_CA_FILE: /entry-cloud-ca/control-ca.pem
      WONTON_ACME_RESOLVERS: ${WONTON_ENTRY_RESOLVER_IP}:53
  tuwunel:
    networks: [default, entry-cloud]
    dns: ["${WONTON_ENTRY_RESOLVER_IP}"]
  frpc:
    networks: [default, entry-cloud]
    dns: ["${WONTON_ENTRY_RESOLVER_IP}"]
    volumes:
      - ${ENTRY_CLOUD_CA_DIR}:/entry-cloud-ca:ro
```

从云目录 `set -a; source .runtime/stack.env; set +a`，再导出
`ENTRY_CLOUD_CA_DIR="$(pwd)/.runtime/ca"` 给家庭 Compose（使用绝对路径）。家庭自身的
网络必须使用另一个已分配子网，不能重新声明云的 IPAM/子网。
核心动态容器的网络配置使用 `WONTON_CLOUD_NETWORK="$WONTON_ENTRY_NETWORK"`；
把它传给家庭核心，而不是把家庭自己的私有网络名填成云网络。
动态创建的 frpc/lego/入口容器也必须接上该外部网络并用同一个 resolver，不能只给 core 接网。
Docker 内置 DNS 仍处理家庭服务名；`.test` 由配置的 resolver 处理。

此 `ca` 目录只含下面的公开证书；CA 私钥独立留在云 `.runtime/secrets`，不挂载给家庭：

* `control-ca.pem`：API、relay 控制连接、Pebble HTTPS API。frpc 用
  `transport.tls.trustedCaFile=/entry-cloud-ca/control-ca.pem`、
  `transport.tls.serverName=relay.<zone>`；强制启用 TLS，不用 skip-verify。
* `issuance-ca.pem`：本次 Pebble 真正签发家里叶子证书的根。Tuwunel/联邦测试客户端信任它，
  `allow_invalid_tls_certificates=false`，不要把控制面 CA 当成家里证书的签发根。
* `home-trust.pem`：前两个根的 bundle，给既访问控制面又验证家里叶子证书的**测试**进程。

Pebble 每次重启都丢失账号、订单和随机签发 CA：`dev.sh roots` 重新导出，重新分发 CA，
并重新创建家里的 ACME 账号、绑定和证书。管理服务对更换账号有 24 小时 CAA 冷却；
完整重跑宜销毁云及家庭测试数据，不能绕过冷却伪装续签成功。
控制面 CA 和三张控制面证书保留 30 天，过期后销毁并重建测试环境。不要安装到宿主全局信任库。

## 签名注册与管理接口验收

以下命令在云目录运行。密钥、邀请和注册响应存到**家庭测试目录**，不是云卷。
`openssl genpkey` 生成的是家庭 endpoint 身份，不是家庭 TLS 叶子密钥。

```bash
umask 077
export HOME_CASE="$(mktemp -d)"
openssl genpkey -algorithm Ed25519 -out "$HOME_CASE/endpoint.pem"
bash dev.sh invite 3600 > "$HOME_CASE/invite.txt"
jq -n --rawfile invite "$HOME_CASE/invite.txt" '{invite:($invite|rtrimstr("\n"))}' > "$HOME_CASE/register.json"
python3 signed-request.py POST /homes --key "$HOME_CASE/endpoint.pem" \
  --body "$HOME_CASE/register.json" --output "$HOME_CASE/registration.json"
export HOME_LABEL="$(jq -r .home "$HOME_CASE/registration.json")"
export HOME_DOMAIN="$(jq -r .domain "$HOME_CASE/registration.json")"
python3 signed-request.py GET "/homes/$HOME_LABEL" --key "$HOME_CASE/endpoint.pem"
```

初次注册 201；相同 endpoint+原邀请重新签名 POST 返回 200，同一家名、原 acme-dns 账号、
**轮换 FRP token**，旧 token 马上失效。每次请求用新时间戳和 nonce；重放原 nonce 返回 401。
helper 使用标准库、OpenSSL 与 curl：Ed25519 公钥原始 32 字节小写 hex；签名覆盖原始 body：

```text
wonton-home-v1\nMETHOD\n/PATH\nUNIX_SECONDS\n32_HEX_NONCE\nSHA256_LOWER_HEX_RAW_BODY
```

末尾没有换行。不带查询字符串；头为 `X-Wonton-Endpoint`、`X-Wonton-Timestamp`、
`X-Wonton-Nonce`、`X-Wonton-Signature`（128 位小写 hex）。`/frp` 仅内部；
`/acme/register` 永远不对外开放，只有 `/acme/update` 代理到真实 acme-dns。

## 真实 lego + acme-dns + Pebble：apex 和 wildcard

从返回的 registration 文件预置 lego 的 acme-dns 账号存储，不能让 lego 向公开 `/register`
申请另一个账号。这两个名字共用 `_acme-challenge.<home>` CNAME，acme-dns 保留两条 TXT。
`lan_acme_dns` 是另一套账号，不要混用。

```bash
jq '{(.domain):.acme_dns}' "$HOME_CASE/registration.json" > "$HOME_CASE/acme-dns.json"
export ACME_DNS_URL="$(jq -r .acme_dns_url "$HOME_CASE/registration.json")"
export HOME_NETWORK="$(pwd)/.runtime/stack.env"
set -a; source "$HOME_NETWORK"; set +a
export CLOUD_CA="$(pwd)/.runtime/ca"
export LEGO_IMAGE=goacme/lego:v4.26.0@sha256:39e65153badce8c67fa16caed2f89ebb15db5747ffbbd502603ce0bbaf2ed218
# 先注册真正 ACME 账号（无证书申请）；账号私钥只在 HOME_CASE。
python3 acme-account.py --home "$HOME_CASE" --email "admin@$HOME_DOMAIN"
```

从真实账号文件取得 account URI，然后签名绑定 CAA，必须等服务返回 `effective:true` 才申请证书：

```bash
ACCOUNT_FILE="$(find "$HOME_CASE/lego/accounts" -name account.json -print -quit)"
jq '{account_uri:.registration.uri}' "$ACCOUNT_FILE" > "$HOME_CASE/caa.json"
python3 signed-request.py PUT "/homes/$HOME_LABEL/caa" --key "$HOME_CASE/endpoint.pem" \
  --body "$HOME_CASE/caa.json" --output "$HOME_CASE/caa-response.json"
jq -e '.effective == true' "$HOME_CASE/caa-response.json"
docker run --rm --network "$WONTON_ENTRY_NETWORK" --dns "$WONTON_ENTRY_RESOLVER_IP" \
  -v "$HOME_CASE:/home" -v "$CLOUD_CA/control-ca.pem:/ca.pem:ro" \
  -e LEGO_CA_CERTIFICATES=/ca.pem -e ACME_DNS_API_BASE="$ACME_DNS_URL" \
  -e ACME_DNS_STORAGE_PATH=/home/acme-dns.json "$LEGO_IMAGE" \
  --path /home/lego --server "https://pebble.$WONTON_HOME_ZONE:14000/dir" \
  --email "admin@$HOME_DOMAIN" --accept-tos --dns acme-dns \
  --dns.resolvers "$WONTON_ENTRY_RESOLVER_IP:53" \
  --domains "$HOME_DOMAIN" --domains "*.$HOME_DOMAIN" run
```

lego 4.26.0 CLI 没有单独的注册子命令；`acme-account.py` 使用真实 ACME newNonce/newAccount
和 OpenSSL RS256 签名，把账号存成 lego 原生格式，不申请订单或家庭叶子证书。接着 lego
复用同一账号申请证书，而不是先发证再绑定 CAA。证书和私钥在
`$HOME_CASE/lego/certificates/`，只给家里的 TLS 入口；**不要复制给云端**。
用 `dig @127.0.0.1 -p "$WONTON_ENTRY_AUTH_PORT" "_acme-challenge.$HOME_DOMAIN" CNAME`、
以及 `dig ... TXT` 经 resolver 查看真实委派与 TXT，不是 mock。

Pebble v2.8.0 的 VA **不执行 CAA issuer/accounturi 授权检查**。这里证明真实 DNS 写入、
客户端等 DNS 生效以及 DNS-01 签发，不能把“换另一个 ACME 账号依然签发成功”宣称为账号锁定
安全性通过。生产 CA 的 RFC 8657 `accounturi` 拒签必须另行在获准环境验证；这里禁止公共 CA。

## 停用验收

```bash
bash dev.sh suspend "$HOME_LABEL" 'local acceptance'
python3 signed-request.py GET "/homes/$HOME_LABEL" --key "$HOME_CASE/endpoint.pem"
# 再从同一私有网络的联邦测试客户端（信任 issuance-ca.pem）请求：
# https://$HOME_DOMAIN/.well-known/matrix/server
# 即使缓存旧 A 或强制 resolve 到 relay IP，新公开连接也必须失败。
bash dev.sh resume "$HOME_LABEL"
# 删除是不可复用墓碑，不是释放标签：
# python3 signed-request.py DELETE "/homes/$HOME_LABEL" --key "$HOME_CASE/endpoint.pem"
```

停用同时去 DNS 和拒绝 Login/NewProxy/NewUserConn/Ping；DNS 缓存不能绕过新连接准入。
Ping 断开控制连接受 frp 心跳时序影响，配置服务端 heartbeat timeout 15 秒；不能只测 NXDOMAIN
就声称已经拆掉已建长连接。配对过的家经 iroh 不过云，另行断言其聊天不受停用影响。

## 上游与许可证 / 验证边界

所有拉取镜像（含 Rust/Debian build base、文中 lego）固定 manifest 摘要；摘要通过 registry
manifest API 获取，未拉镜像。acme-dns v1.0 上游镜像只有 linux/amd64，因此 Compose 显式
指定平台；arm64 宿主要已有可用的 amd64 仿真，否则须另行固定官方构建，不能悄悄换标签。
Rust 构建使用仓库 lockfile，系统包依赖 Debian 仓库（镜像摘要固定不等于 apt 仓库快照）。

| 独立组件 | 版本 | 许可证 / 上游 |
| --- | --- | --- |
| frps / frpc | 0.71.1 | [Apache-2.0](https://github.com/fatedier/frp/tree/v0.71.1) |
| PowerDNS Authoritative | 4.9.11 | [GPL-2.0](https://github.com/PowerDNS/pdns/tree/rel/auth-4.9.x) |
| PowerDNS Recursor | 5.1.9 | [GPL-2.0](https://github.com/PowerDNS/pdns/tree/rel/rec-5.1.x) |
| dnsdist | 1.9.10 | [GPL-2.0](https://github.com/PowerDNS/pdns/tree/rel/dnsdist-1.9.x) |
| acme-dns | 1.0 | [MIT](https://github.com/joohoi/acme-dns/tree/v1.0) |
| Pebble | 2.8.0 | [MPL-2.0](https://github.com/letsencrypt/pebble/tree/v2.8.0) |
| HAProxy | 3.2.8 | [GPL-2.0 / LGPL-2.1（部分库）](https://www.haproxy.org/#licence) |
| lego（家庭验收工具） | 4.26.0 | [MIT](https://github.com/go-acme/lego/tree/v4.26.0) |
| Rust / 标准库（构建） | 1.99.1 | MIT / Apache-2.0；GNU toolchain 仅构建工具 |
| Debian / Alpine（镜像 OS） | 各镜像指定版本 | 各包许可证，不链接 PowerDNS/HAProxy GPL 代码进 Rust |

管理服务 Rust 依赖的许可证见 `crates/wonton-home/README.md`。本目录 Python/shell 工具
没有新增第三方语言包。OpenSSL 是宿主工具，Apache-2.0（3.x）。

已查阅上游 [frps 配置](https://github.com/fatedier/frp/blob/v0.71.1/conf/frps_full_example.toml)、
[同端口分流源码](https://github.com/fatedier/frp/blob/v0.71.1/server/service.go)、
[acme-dns 配置](https://github.com/joohoi/acme-dns/blob/v1.0/config.cfg)、
[Pebble DNS-01 源码](https://github.com/letsencrypt/pebble/blob/v2.8.0/va/va.go)、
[随机签发根说明](https://github.com/letsencrypt/pebble/tree/v2.8.0#management-interface)、
[lego acme-dns provider](https://github.com/go-acme/lego/blob/v4.26.0/providers/dns/acmedns/acmedns.go)。
账号初始化的存储字段按 [lego 原生账号存储](https://github.com/go-acme/lego/blob/v4.26.0/cmd/accounts_storage.go)
与 [注册资源](https://github.com/go-acme/lego/blob/v4.26.0/registration/registar.go) 实现。
未执行 build/test/lint/format、Compose 配置检查、启动、镜像 pull 或上述验收命令。
合并后的负责人须实跑：配置生成与健康门、双家庭 apex/wildcard DNS-01、严格 TLS 联邦、
带旧 DNS 缓存的停用/已有长连接断开、iroh 配对路径不受影响，以及项目专属 destroy/release。
