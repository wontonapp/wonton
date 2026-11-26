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
| `prepare.py` | 生成密钥、CA、自签占位证书，渲染 `templates/` 到 `--root` 和 `$ROOT/certs` |
| `templates/` | frps/frpc/pdns/dnsdist/acme-dns 配置与 nginx vhost，`@VAR@` 由 prepare 替换 |
| `scripts/dns-bootstrap.py` | 建区并写入固定记录（环境变量里的额外 A 也由它写） |
| `scripts/cert-deploy.sh` | certbot deploy hook：把续期结果复制到 nginx 只读的 `$ROOT/certs` 并 reload |
| `scripts/phase1-nginx.sh` | 第一期：只改现网 vhost 的 listen 行为回环，内容原样；备份目录一条命令回滚 |
| `scripts/phase1-nginx-rollback.sh` | 第一期回滚：还原备份并 reload |
| `scripts/verify.sh` | 换手前后的 HTTP/TLS/DNS 探针矩阵 |
| `scripts/backup.sh` + `systemd/` | 每小时本地快照，保留 24 小时 + 28 天 |
| `.env.example` | 全部占位符；复制为 `.env`（0600）后填真值 |

## 分两期（主人 2026-11-27 定）

- **第一期只做 443 分流**：frps 接 443，nginx 退回环；**旧 nginx 站点内容原样**，只改 `listen` 行。不建 DNS Host、不切 NS。验 push/relay/omarchy 和证书续期。回滚用 `phase1-nginx-rollback.sh` 一条命令。
- **第二期才建 PowerDNS 对外 53 并切 NS**：切之前在外部机器 `dig @<公网IP>` 答对所有记录。DNS 不切，则 53 被拒也不影响第一阶段验收。

> 单台机器上两期可以合用同一套 compose；第二期只是把旧 nginx vhost 换成模板 vhost 并切 NS。`prepare.py` 一次渲染即可。

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
python3 -m py_compile prepare.py scripts/dns-bootstrap.py
./prepare.py
docker compose --env-file .env config -q
```

`prepare.py` 把 7 天自签占位证书放 `$ROOT/certs/<主机名>/`，nginx 只读这里；certbot
写 `/etc/letsencrypt`，deploy hook 再复制过来。

### 2. 第一期：只改 listen、保留旧站点内容

```sh
systemctl stop nginx
docker compose --env-file .env pull
set -a; . ./.env; set +a
install -d -m 755 /root/nginx-sites-backup "$ROOT/scripts"
install -m 755 scripts/phase1-nginx.sh /root/phase1-nginx.sh
install -m 755 scripts/cert-deploy.sh "$ROOT/scripts/cert-deploy.sh"
# 旧 vhost 内容一字不改，只把 80/443 listen 换到 127.0.0.1:4080/4443
/root/phase1-nginx.sh "$ROOT/nginx/names.conf"
systemctl start nginx
nginx -t
ss -lntp | grep -E ':(80|443)\b' || true        # 80/443 必须已空
install -d -m 755 /var/www/letsencrypt /run
docker compose --env-file .env up -d
docker compose ps
```

脚本把旧 `sites-enabled` 全量备份到 `/root/nginx-sites-backup-<时间>/` 后，只把 `listen 80`/`443`
改成 `127.0.0.1:4080/4443 proxy_protocol`；`omarchy-alt`（8080）不动。新增的只有
`names.conf` 和 `realip.conf`。旧 vhost 内容、SPA 回退、HSTS、缓存头都还在。

验证：`verify.sh` 的 push/relay 健康探针、`curl --resolve` 的 HTTP→HTTPS 跳转、以及
下面证书续期。全绿后才可以进第二期。回滚：`/root/nginx-sites-backup-<时间>/` 传给
`scripts/phase1-nginx-rollback.sh`，一条命令还原。

### 3. 第一期：证书 authenticator 换 webroot + 续期演练

现网三条旧 lineage 的 `authenticator = nginx`；迁移后 nginx 不再监听 80，certbot-nginx
会自己加 `listen 80` 跟 frps 抢口。必须改成 webroot：

```sh
HOOK="$ROOT/scripts/cert-deploy.sh"
for name in "$WONTON_DOMAIN" "$WONTON_PUSH_HOST" "$WONTON_RELAY_HOST"; do
  certbot reconfigure --cert-name "$name" --webroot -w /var/www/letsencrypt --deploy-hook "$HOOK"
done
# reconfigure 不重签；现有 lineage 先手搬到 nginx 只读目录，以后由 hook 复制
for name in "$WONTON_DOMAIN" "$WONTON_PUSH_HOST" "$WONTON_RELAY_HOST"; do
  install -d -m 755 "$ROOT/certs/$name"
  install -m 644 "/etc/letsencrypt/live/$name/fullchain.pem" "$ROOT/certs/$name/fullchain.pem"
  install -m 600 "/etc/letsencrypt/live/$name/privkey.pem" "$ROOT/certs/$name/privkey.pem"
done
certbot renew --dry-run     # 三条都必须绿
systemctl reload nginx
```

新的 `names` 证书和 apex 双名证书在第二期 DNS 通了以后再签；本期只保证旧三条能续。

### 4. 第一期自检

```sh
curl -sk --noproxy '*' -o /dev/null -w '%{http_code}\n' --resolve "$WONTON_PUSH_HOST:443:127.0.0.1" "https://$WONTON_PUSH_HOST/healthz" # 200
curl -sk --noproxy '*' -o /dev/null -w '%{http_code}\n' --resolve "$WONTON_RELAY_HOST:443:127.0.0.1" "https://$WONTON_RELAY_HOST/healthz" # 200
# 个人站点是 HTTP-only，走本机 frps 的 80
curl -s --noproxy '*' -H "Host: $WONTON_OMARCHY_HOST" http://127.0.0.1/ -o /dev/null -w '%{http_code}\n' # 200
curl -sk --noproxy '*' -o /dev/null -w '%{http_code}\n' "https://$WONTON_DOMAIN/"   # 旧 apex 站点，200/301 都可能
openssl s_client -connect 127.0.0.1:443 -servername "unknown.invalid" </dev/null 2>&1 | grep unrecognized
```

`verify.sh` 也做这些检查（DNS SOA 只查本机）。个人站点在外网会被备案页挡住是预期的，
别拿外网 `curl https://<个人站点>` 当失败。

### 5. 第二期：建区、DNS 门槛、切 NS

```sh
set -a; . ./.env; set +a
python3 scripts/dns-bootstrap.py
dig +short @"$WONTON_LISTEN_IP" "$WONTON_DOMAIN" SOA
dig +short @"$WONTON_LISTEN_IP" "auth.$WONTON_DOMAIN" NS
```

**NS 门槛（外网另一台机器，两个都要答对）**：

```sh
dig +norec @"<公网IP>" "$WONTON_DOMAIN" SOA
dig +norec @"<公网IP>" "push.$WONTON_DOMAIN" A
```

再从阿里云云解析控制台导出解析记录，和 PowerDNS 区逐条对照；区里有 `*` 通配，按名
字 `dig` 列不全，导出表才全。`.com` 上无 DS，不用先撤 DNSSEC。

切 NS 前先补两件事：
1. 安全组入方向放 53/tcp+udp（外网才答得到），并在云解析控制台给 `names` 加一条
   A 记录指向 `@PUBLIC_IP@`（公网 DNS 仍由阿里云答，LE 靠它做 HTTP-01；NS 切后由本机答）。
2. 签 `names` 和 apex 双名证书（apex+www 在旧 lineage 里已有，这里只 ensure）：

```sh
HOOK="$ROOT/scripts/cert-deploy.sh"
certbot certonly --webroot -w /var/www/letsencrypt -d "$WONTON_NAMES_HOST" \
  --deploy-hook "$HOOK" --email "$WONTON_ACME_EMAIL" --agree-tos --non-interactive
# apex 若已有就 keep，没有则签
certbot certonly --webroot -w /var/www/letsencrypt -d "$WONTON_DOMAIN" -d "www.$WONTON_DOMAIN" \
  --deploy-hook "$HOOK" --email "$WONTON_ACME_EMAIL" --agree-tos --non-interactive --keep-until-expiring
```

然后按控制台单（`tasks/home-808-deploy.owner-steps.md`，私有）：
禁止更新锁 → 建 `ns-auth`/`ns2-auth` DNS Host → 确认外网 `dig` → 改 NS → 等 24–48h。
全量验证三个公共解析器只剩 `ns-*@DOMAIN@` 后跑 `scripts/verify.sh`。

### 6. 注册第一个家

在 `names.@DOMAIN@` 上创建邀请，把 `https://names.@DOMAIN@/i/<code>` 发给对方：

```sh
install -m 644 "$ROOT"/bootstrap/install.sh /var/www/  # 可选：不带邀请的用法
```

`/i` 和 `/install.sh` 都先取 `/relay-ca.pem` 并导出 `WONTON_ACME_DIRECTORY`、
`WONTON_ACME_ACCEPT_TERMS`、`WONTON_FRP_CA_FILE`；缺一个安装器就停。

### 7. 备份计时器

```sh
install -d -m 755 "$ROOT"/scripts
install -m 755 scripts/backup.sh "$ROOT/scripts/backup.sh"
install -m 644 systemd/wonton-entry-backup.{service,timer} /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now wonton-entry-backup.timer
```

## 回滚

1. 注册商「DNS 修改」改回原来的两条 NS，删除两个 DNS Host；等 48 小时旧 NS 生效。
2. 机器侧先 `docker compose stop frps system-frpc home`（保留 pdns/dnsdist/acme-dns 继续跑）
   ≥48 小时。NS 还没回切完时停掉 DNS = 整区解析中断。
3. `tar -C /etc -xzf /root/nginx-certbot-*.tgz`；`tar -C / -xzf /root/omarchy-sites-*.tgz`；
   `systemctl restart nginx`。
4. `rm /etc/nginx/sites-enabled/{push,relay,omarchy,names,apex}.conf`（按需）。
5. 48 小时后再 `docker compose down`；`$ROOT` 数据留。

## 每月检查

`scripts/verify.sh`；`certbot certificates` 剩余天数；`systemctl list-timers` 有备份；
`df -h`；`docker compose ps` 全 healthy；dnsdist 的 53 只对必要来源开放。
