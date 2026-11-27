# 生产入口云（单机）

一套可以照抄到一台全新公网 Ubuntu 机器上的部署件：frps 独占 80/443 并作为唯一
明面入口，nginx 留在本机回环后面继续服务 push / relay / 个人站点 / 安装页，PowerDNS +
dnsdist 持有 `@DOMAIN@` 权威区，acme-dns 只回答 `auth.@DOMAIN@` 的委派，wonton-home
只做注册、钩子和 DNS 对账。家的证书和私钥从不落到这台机器上。

地址、IP、主机名一律是占位符；真正的值只写进运维自己的 `.env`，不进公开仓库。

```
client ──443/tcp──▶ frps (host net)
                       │  SNI vhosts
                       ├── 户名.@DOMAIN@ ──▶ 对应家的 frpc ──▶ 家里的 HAProxy
                       └── push/relay/names/apex（TLS） + 个人站点（HTTP）
                                            ──▶ system-frpc ──▶ nginx 127.0.0.1:4443|4080
client ──80/tcp───▶ frps vhostHTTPPort ──▶ 同一个 system-frpc
client ──53/udp───▶ dnsdist ──┬── auth.@DOMAIN@ ──▶ acme-dns
                              └── 其余 ──▶ PowerDNS
```

为什么是 frps 在前而不是 nginx `stream` 在前：frps 不解析 PROXY 头；只有 frps 自己
收到客户端的 TCP 连接，才能把真实来源 IP 交给家里的 HAProxy。nginx `stream` 前置会让
HAProxy 看到 `127.0.0.1`，按 IP 的失败计数和限速失效。

**个人站点只走 HTTP。** 它在入口机所在云里没有备案，80/443 被云厂商拦截：HTTP 回
`beian-block` 403、HTTPS reset。所以个人站点不签证书、不做 301；apex 也不再跳它，只回
最小 404。它的站内 SPA 回退、gzip 和缓存头保留。8080 备用站点照留。

上线前 K5 必须已合入：核心给每条入口代理发
`[<户名>.<域名>, *.<户名>.<域名>]` 两条 `customDomains`（钩子已接受一条或这两条）。
只发后者时插件网页 `<应用>.<户名>.<域名>` 不会路由到家里。以后做“用户自己起户名”时，
`push`、`relay`、`names`、`www`、`auth`、`ns-auth`、`ns2-auth`、`system`、`lan` 这些首标签要保留。

## 文件

| 路径 | 作用 |
| --- | --- |
| `compose.yaml` | frps、wonton-home、pdns、dnsdist、acme-dns、system-frpc；全部 host 网络 |
| `prepare.py` | 生成密钥、CA、自签占位证书，渲染 `templates/` 到 `--root`、`$ROOT/certs` 和 `$ROOT/scripts/cert-deploy.sh` |
| `templates/` | frps/frpc/pdns/dnsdist/acme-dns 配置、nginx vhost、certbot deploy hook，`@VAR@` 由 prepare 替换 |
| `scripts/dns-bootstrap.py` | 建区并写入固定记录（环境变量里的额外 A 也由它写） |
| `scripts/nginx-cutover.py` | 第一期 `check`/`apply`/`rollback`、续期配置核对 `renewal-check`、第二期换模板 `swap` |
| `scripts/verify.sh` | 探针矩阵；`verify.sh phase1` 只跑第一期该绿的项 |
| `scripts/backup.sh` + `systemd/` | 每小时本地快照，保留 24 小时 + 28 天 |
| `tests/cutover/run.sh` | 在一次性容器（Ubuntu 24.04 的 nginx，无网络）里跑 `nginx-cutover.py` 和 nginx 模板的行为测试 |
| `.env.example` | 全部占位符；复制为 `.env`（0600）后填真值 |

改了 `nginx-cutover.py` 或 `templates/nginx/` 就跑一次 `sh tests/cutover/run.sh`（要 Docker）。

## 分两期（主人 2026-11-27 定）

- **第一期只做 443 分流**：frps 接 80/443，nginx 退回环；**旧 nginx 站点内容原样**，只改
  80/443 的 `listen` 行，并给挪过去的 443 块加一条 ACME webroot location。不起 dnsdist、不建
  DNS Host、不切 NS。验 push/relay/个人站点和三条旧证书的续期。回滚是一条命令。
- **第二期才建 PowerDNS 对外 53 并切 NS**：切之前在外部机器 `dig @<公网IP>` 答对所有记录。

两期用同一套 compose 和同一次 `prepare.py` 渲染。`templates/nginx/` 里的 push/relay/个人站点
vhost 是给全新机器用的；已有站点的机器上，这三个保留第一期挪过的旧 vhost，第二期只可选地把
apex 换成模板（见第 5 步）。

## 前置

0. 在开发机构建并送入 `wonton-home:prod`（目标机不编译）：
   `docker build -t wonton-home:prod -f deploy/dev/entry-cloud/Dockerfile .`，
   `docker save` 后 `docker load`。其余服务用官方镜像，发光前把 `.env` 里的
   `FRPS_IMAGE` / `FRPC_IMAGE` / `PDNS_IMAGE` / `DNSDIST_IMAGE` / `ACME_DNS_IMAGE`
   全部改成按 digest 固定。

1. Ubuntu 24.04+，root，Docker Engine + compose 插件，`nginx`（**≥1.24**；模板用 1.24 也认的
   `listen … http2` 写法，1.25+ 会提示 deprecation 但不影响）、`certbot`、
   `python3-certbot-nginx`、`openssl`、`curl`、`dnsutils` 已装。
2. 域名注册商（阿里云）把 `@DOMAIN@` 的 NS 改成 `ns-auth.@DOMAIN@`、
   `ns2-auth.@DOMAIN@`，并在注册商登记这两个名字的 glue A = `@PUBLIC_IP@`。
   整区迁到本机 PowerDNS 的路线：一次切换，之后不再碰注册商。
3. 安全组入方向只放：22（限来源）、80、443；**53/tcp、53/udp 第二期再放**；`7842/udp`
   保持 iroh QAD 的放行；`8080` 是否保留问主人。其余关闭。`@LISTEN_IP@` 是网卡的私网
   地址，`@PUBLIC_IP@` 是云 NAT 的公网地址；dnsdist 绑前者，外网才能经 NAT 把 53 送到。
4. 备份总则：先把现有 nginx 配置、`/etc/letsencrypt`、站点根打成两个回滚包，
   `tar -tzf` 数一遍文件：

   ```sh
   tar -C /etc -czf /root/nginx-certbot-$(date +%F).tgz nginx letsencrypt
   tar -C / -czf /root/omarchy-sites-$(date +%F).tgz var/www/omarchy
   ```

## 上线顺序

所有命令在目标机 root shell、本目录下执行；`$ROOT` 默认 `/opt/wonton-home`。

```sh
ROOT=/opt/wonton-home
```

### 1. 渲染与静态检查

```sh
cp .env.example .env && chmod 600 .env && vi .env   # 填真值；WONTON_HOME_SYSTEM_DOMAINS 也要改
python3 -m py_compile prepare.py scripts/dns-bootstrap.py scripts/nginx-cutover.py
./prepare.py
docker compose --env-file .env config -q
```

`prepare.py` 把 7 天自签占位证书放 `$ROOT/certs/<主机名>/`，nginx 只读这里；certbot
写 `/etc/letsencrypt`，deploy hook 再复制过来。

### 2. 第一期：检查会失效的名字

```sh
set -a; . ./.env; set +a
python3 scripts/nginx-cutover.py check --frpc "$ROOT/frpc/system-frpc.toml"
```

`check` 只读。它解析整份 nginx 配置（含 include），列出今天 nginx 在 80/443 上回答、而
system-frpc 第一期之后不会再送回 nginx 的东西：不在 system-frpc 代理里、或不在
`WONTON_HOME_SYSTEM_DOMAINS` 里的 `server_name`，通配和正则名字，以及
`default_server`（直接用 IP 或未知 Host 访问时落到的块）。有一条就拒绝，什么都不改。
80/443 之外的 `listen`（如 `listen 8080;`）不动；80/443 的 listen 在 `sites-enabled` 之外、
带不认识的参数、或 server 块没有 listen（nginx 会默认占 `*:80`）也拒绝。

对每一条，要么补进 system-frpc 和 `WONTON_HOME_SYSTEM_DOMAINS` 后重新 `prepare.py`，
要么确认不要了，按它打印的 `--accept 'http:<名字>'` 显式放行。放行的每一条都记进下面的
“预期会变”，和主人对过再执行。

### 3. 第一期：切换

```sh
docker compose --env-file .env pull
docker compose --env-file .env up -d pdns acme-dns           # 不占 80/443
BACKUP=/root/wonton-cutover-$(date -u +%Y%m%dT%H%M%SZ)
python3 scripts/nginx-cutover.py apply --frpc "$ROOT/frpc/system-frpc.toml" \
  --names-conf "$ROOT/nginx/names.conf" --realip "$ROOT/nginx/realip.conf" \
  --backup-dir "$BACKUP" --accept '…'                         # 和 check 放行的一致
docker compose --env-file .env up -d frps home system-frpc
docker compose ps
```

`apply` 先把 `sites-enabled` 和 `/etc/letsencrypt/renewal` 整份备份到 `$BACKUP`，然后只改
80/443 的 `listen` 行：IPv4 改成 `127.0.0.1:4080`/`4443` 加 `proxy_protocol`，同一块里重复的
（如 `[::]:443`）注释掉；挪过去的 443 块开头加
`location ^~ /.well-known/acme-challenge/ { root /var/www/letsencrypt; }`。旧的 80 块不用加：
certbot 写的 `if ($host = …) { return 301 … }` 把挑战跳到 443，Let's Encrypt 跟随跳转。
`sites-enabled` 里的符号链接换成改过的普通文件，`sites-available` 原件不动。另外只新增
`wonton-names.conf` 和 `conf.d/wonton-realip.conf`。`nginx -t` 失败时自动还原文件、不 reload；
成功才 reload，并等到 nginx 让出 80/443、占上 4080/4443。停机从 reload 到 frps 起来，几秒。

**预期会变**（执行前和主人逐条对过）：

- 80/443 由 frps 接。push、relay、apex/www、个人站点（HTTP）照旧由原来的 vhost 回答。
- `check` 列出、用 `--accept` 放行的名字不再有人回答：HTTPS 回 `unrecognized_name`，HTTP 回
  frps 的 404。直接用 IP 访问 80/443 也不再落到原来的 `default_server`。
- IPv6 的 80/443 不再监听（system-frpc 只走 `127.0.0.1`）；区里有 AAAA 的名字会连不上。
- **push、relay 的新连接从此要过 wonton-home 的钩子。** frps 对 Login、NewProxy、Ping、
  NewUserConn 都同步调用钩子，钩子出错就拒（frp v0.72）。wonton-home 停了或不响应时，
  push/relay 的新连接会被拒，system-frpc 的心跳被拒后会掉线重连。
- 三条旧证书的 `authenticator` 从 `nginx` 改成 `webroot`（下一步）。

**回滚条件**，任一条成立就回滚：`docker compose ps` 里 home 或 system-frpc 不是 healthy；
`verify.sh phase1` 有 FAIL；`certbot renew --dry-run` 不绿且当场修不好；主人报站点异常。

```sh
python3 scripts/nginx-cutover.py rollback --backup-dir "$BACKUP" --compose-dir "$PWD"
```

`rollback` 按顺序：`docker compose stop frps system-frpc` 并等 80/443 空出来；还原
`sites-enabled`、删掉新增的两个文件、还原 `renewal/*.conf`；`nginx -t` 后 reload；确认 80/443
回到 nginx，不对就非零退出。备份之后才新建的 lineage 不删，会打印出来。pdns/acme-dns/home
不占公网端口，可以留着。

### 4. 第一期：证书改 webroot + 续期演练

现网三条旧 lineage 的 `authenticator = nginx`；nginx 不再监听 80 以后，certbot-nginx 续期时会
自己加 `listen 80` 跟 frps 抢口，所以改成 webroot。`reconfigure` 先用新设置跑一次 dry-run，
通过了才写回；不带 `--deploy-hook`，原有的 `renew_hook`（例如把 relay 证书复制给 iroh-relay
的那条）保持不变。第一期的旧 vhost 仍然直接读 `/etc/letsencrypt/live`，不需要新 hook。

```sh
for name in "$WONTON_DOMAIN" "$WONTON_PUSH_HOST" "$WONTON_RELAY_HOST"; do
  certbot reconfigure --cert-name "$name" --webroot -w /var/www/letsencrypt
done
python3 scripts/nginx-cutover.py renewal-check --backup-dir "$BACKUP" \
  --cert-name "$WONTON_DOMAIN" --cert-name "$WONTON_PUSH_HOST" --cert-name "$WONTON_RELAY_HOST"
certbot renew --dry-run     # 全部 lineage 都必须绿
```

`renewal-check` 对照 `$BACKUP/renewal`：三条都是 `webroot`，`renew_hook`/`pre_hook`/
`post_hook` 一字未变，否则非零退出。

### 5. 第一期自检

```sh
scripts/verify.sh phase1
```

全部请求打本机 `127.0.0.1` 的 frps，用真名字做 Host/SNI。第一期不查 names 证书、站点别名和
DNS。个人站点在外网会被备案页挡住是预期的，别拿外网 `curl https://<个人站点>` 当失败。
全绿、续期演练全绿后才进第二期。

### 6. 第二期：建区、DNS 门槛、切 NS

```sh
set -a; . ./.env; set +a
docker compose --env-file .env up -d dnsdist
python3 scripts/dns-bootstrap.py
dig +short @"$WONTON_LISTEN_IP" "$WONTON_DOMAIN" SOA
dig +short @"$WONTON_LISTEN_IP" "auth.$WONTON_DOMAIN" NS
```

**NS 门槛（外网另一台机器，两个都要答对）**：

```sh
dig +norec @"<公网IP>" "$WONTON_DOMAIN" SOA
dig +norec @"<公网IP>" "push.$WONTON_DOMAIN" A
```

再从注册商的云解析控制台导出解析记录，和 PowerDNS 区逐条对照；区里有 `*` 通配，按名字
`dig` 列不全，导出表才全。`.com` 上无 DS，不用先撤 DNSSEC。

切 NS 前先补三件事：

1. 安全组入方向放 53/tcp+udp（外网才答得到），并在云解析控制台给 `names` 加一条 A 记录指向
   `<公网IP>`（公网 DNS 仍由旧服务商答，LE 靠它做 HTTP-01；NS 切后由本机答）。
2. 装 nginx 只读证书目录的 deploy hook，放进 `renewal-hooks/deploy/`：目录 hook 和每条
   lineage 自己的 `renew_hook` 一起跑，不会替换它；只复制 `prepare.py` 建过占位目录的 lineage。
   然后签 `names`：

   ```sh
   grep -H renew_hook /etc/letsencrypt/renewal/*.conf     # 看一眼原有的 hook，都保留
   install -m 755 "$ROOT/scripts/cert-deploy.sh" /etc/letsencrypt/renewal-hooks/deploy/wonton-cert-deploy.sh
   certbot certonly --webroot -w /var/www/letsencrypt -d "$WONTON_NAMES_HOST" \
     --email "$WONTON_ACME_EMAIL" --agree-tos --non-interactive
   ```

3. （可选，主人定 apex/www 不再显示旧站点时）把旧 apex 站点换成模板：先把现有 apex 证书复制
   进 `$ROOT/certs`，再用 `swap` 停用旧 vhost、装模板，`nginx -t` 不过就自动还原：

   ```sh
   RENEWED_LINEAGE="/etc/letsencrypt/live/$WONTON_DOMAIN" "$ROOT/scripts/cert-deploy.sh"
   ls /etc/nginx/sites-enabled/                           # 找到旧 apex 站点的条目名
   SWAP=/root/wonton-swap-$(date -u +%Y%m%dT%H%M%SZ)
   python3 scripts/nginx-cutover.py swap --backup-dir "$SWAP" \
     --remove <旧 apex 条目> --install "$ROOT/nginx/apex.conf"
   # 撤销：python3 scripts/nginx-cutover.py rollback --nginx-only --backup-dir "$SWAP"
   ```

   apex 证书要含 www：旧 lineage 没有 www 时，`certbot certonly --webroot -w /var/www/letsencrypt
   --cert-name "$WONTON_DOMAIN" -d "$WONTON_DOMAIN" -d "www.$WONTON_DOMAIN" --expand`。

然后按主人的控制台操作单（私有，不进仓库）：禁止更新锁 → 建 `ns-auth`/`ns2-auth` DNS Host →
确认外网 `dig` → 改 NS → 等 24–48h。三个公共解析器都只剩 `ns-*.<域名>` 后跑 `scripts/verify.sh`。

### 全新机器

没有旧站点的机器不走 `nginx-cutover.py`：把渲染好的 vhost 全部启用，再签证书。

```sh
install -m 644 "$ROOT/nginx/realip.conf" /etc/nginx/conf.d/wonton-realip.conf
for f in apex names omarchy push relay; do ln -sf "$ROOT/nginx/$f.conf" /etc/nginx/sites-enabled/wonton-$f.conf; done
install -d -m 755 /var/www/letsencrypt
install -m 755 "$ROOT/scripts/cert-deploy.sh" /etc/letsencrypt/renewal-hooks/deploy/wonton-cert-deploy.sh
nginx -t && systemctl reload nginx
```

### 7. 注册第一个家

在 `names.<域名>` 上创建邀请，把 `https://names.<域名>/i/<code>` 发给对方。`/i` 和
`/install.sh` 都先取 `/relay-ca.pem` 并导出 `WONTON_ACME_DIRECTORY`、`WONTON_ACME_ACCEPT_TERMS`、
`WONTON_FRP_CA_FILE`；缺一个安装器就停。

### 8. 备份计时器

```sh
install -m 755 scripts/backup.sh "$ROOT/scripts/backup.sh"
install -m 644 systemd/wonton-entry-backup.{service,timer} /etc/systemd/system/
echo 'WONTON_BACKUP_EXTRA="/var/www/<站点目录>"' > /etc/default/wonton-entry-backup   # 可选
systemctl daemon-reload
systemctl enable --now wonton-entry-backup.timer
```

`backup.sh` 只容忍 tar 的退出码 1（文件读时变化）；路径不存在等其他失败直接非零退出，
不留半截的包。

## 回滚

第一期的回滚见第 3 步。第二期：

1. 注册商「DNS 修改」改回原来的两条 NS，删除两个 DNS Host；等 48 小时旧 NS 生效。
2. 机器侧只 `docker compose stop frps system-frpc home`（pdns/dnsdist/acme-dns 继续跑）
   ≥48 小时：`.com` 委派 TTL 是 172800 秒，NS 还没回切完时停掉 DNS = 整区解析中断。
3. nginx 回到切换前：`python3 scripts/nginx-cutover.py rollback --backup-dir "$BACKUP"
   --compose-dir "$PWD"`（第一期的备份；做过 `swap` 的先 `rollback --nginx-only` 撤 swap）。
4. 48 小时后再 `docker compose down`；`$ROOT` 数据留。

## 每月检查

`scripts/verify.sh`；`certbot certificates` 剩余天数；`systemctl list-timers` 有备份；
`df -h`；`docker compose ps` 全 healthy；dnsdist 的 53 只对必要来源开放。
