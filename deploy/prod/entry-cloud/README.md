# 生产入口云（单机）

一套可以照抄到一台全新公网 Ubuntu 机器上的部署件：frps 独占 80/443 并作为唯一
明面入口，nginx 留在本机回环后面继续服务 push / relay / omarchy / 安装页，PowerDNS +
dnsdist 持有 `@DOMAIN@` 权威区，acme-dns 只回答 `auth.@DOMAIN@` 的委派，wonton-home
只做注册、钩子和 DNS 对账。家的证书和私钥从不落到这台机器上。

地址、IP、主机名一律是占位符；真正的值只写进运维自己的 `.env`，不进公开仓库。

```
client ──443/tcp──▶ frps (host net)
                       │  SNI vhosts
                       ├── 户名.@DOMAIN@ ──▶ 对应家的 frpc ──▶ 家里的 HAProxy
                       └── push/relay/omarchy/names/apex
                                            ──▶ system-frpc ──▶ nginx 127.0.0.1:4443|4080
client ──80/tcp───▶ frps vhostHTTPPort ──▶ 同一个 system-frpc（301 到 https）
client ──53/udp───▶ dnsdist ──┬── auth.@DOMAIN@ ──▶ acme-dns
                              └── 其余 ──▶ PowerDNS
```

为什么是 frps 在前而不是 nginx `stream` 在前：frps v0.72 的 HTTPS vhost 会读
ClientHello 选出代理，然后把**原始字节**连同 PROXY v2 送到 frpc，家里的 HAProxy 才能
看到真实来源 IP。nginx `stream` 前置同样能工作，但它不能给 frps 加 PROXY 头（frps 不
解析），所有客户端在 HAProxy 里会变成同一个 `127.0.0.1`，按 IP 的失败计数和限速全失效。
固定站点照旧由 nginx 服务，只是回源要走本机 system-frpc。

上线前 K5 必须已合入：核心给每条入口代理发
`[<户名>.<域名>, *.<户名>.<域名>]` 两条 `customDomains`（钩子已接受一条或这两条）。
只发后者时插件网页 `<应用>.<户名>.<域名>` 不会路由到家里。

## 文件

| 路径 | 作用 |
| --- | --- |
| `compose.yaml` | frps、wonton-home、pdns、dnsdist、acme-dns、system-frpc；全部 host 网络 |
| `prepare.py` | 生成密钥、CA、自签占位证书，渲染 `templates/` 到 `--root` 和 `/etc/letsencrypt/live` |
| `templates/` | frps/frpc/pdns/dnsdist/acme-dns 配置与 nginx vhost，`@VAR@` 由 prepare 替换 |
| `scripts/dns-bootstrap.py` | 建区并写入固定记录（幂等） |
| `scripts/verify.sh` | 换手前后的 curl/TLS/DNS 探针矩阵 |
| `scripts/backup.sh` + `systemd/` | 每小时本地快照，保留 24 小时 + 28 天 |
| `.env.example` | 全部占位符；复制为 `.env`（0600）后填真值 |

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
   这就是整区迁到本机 PowerDNS 的路线：一次切换，之后不再碰注册商。
3. 安全组只放：22（限来源）、80/tcp、443/tcp、53/tcp+udp；`7842/udp` 保持现在
   iroh-relay QAD 的放行。其余全部关闭。
4. 先把现有 nginx 配置、`/etc/letsencrypt`、两个站点根打成
   `/root/nginx-certbot-$(date +%F).tgz`，`tar -tzf` 数一遍文件；这是回滚包。

## 上线顺序

所有命令在目标机 root shell、本目录下执行；`$ROOT` 默认 `/opt/wonton-home`。

```sh
ROOT=/opt/wonton-home
```

### 1. 渲染与静态检查

```sh
cp .env.example .env && chmod 600 .env && vi .env   # 填真值，删掉不用的行
python3 -m py_compile prepare.py scripts/dns-bootstrap.py
./prepare.py
docker compose --env-file .env config -q
```

`prepare.py` 会在 `/etc/letsencrypt/live/<每个主机名>/` 放 7 天自签占位证书；
certbot 签发后覆盖同名文件，不会改 vhost。

### 2. 停 nginx、装 vhost、起栈

```sh
systemctl stop nginx
install -m 644 "$ROOT"/nginx/*.conf /etc/nginx/sites-available/
for f in "$ROOT"/nginx/*.conf; do ln -sf "/etc/nginx/sites-available/$(basename "$f")" /etc/nginx/sites-enabled/; done
install -m 644 templates/nginx/realip.conf.tmpl /etc/nginx/conf.d/wonton-realip.conf
nginx -t && systemctl start nginx
install -d -m 755 /var/www/letsencrypt /run
docker compose --env-file .env up -d
docker compose ps
```

如果 80/443 上还有旧 nginx 监听，先 `ss -lntp` 确认两个口都已释放；起不来就看
`docker compose logs frps`。

### 3. 建区、签证书

```sh
set -a; . ./.env; set +a
python3 scripts/dns-bootstrap.py
```

DNS 记录此时只在 PowerDNS 里，公网仍由注册商旧 NS 回答，所以要先在本机验证：

```sh
dig +short @127.0.0.1 "$WONTON_DOMAIN" SOA
dig +short @127.0.0.1 "ns-auth.$WONTON_DOMAIN" A
dig +short @127.0.0.1 "auth.$WONTON_DOMAIN" NS
```

逐条比对旧区清单（apex、www、push、relay、oma、omarchy、`*`、CAA）后才改注册商。

证书全部走 HTTP-01（`webroot`），不依赖 DNS 切换：

```sh
certbot certonly --webroot -w /var/www/letsencrypt \
  -d "$WONTON_DOMAIN" -d "www.$WONTON_DOMAIN" \
  --email "$WONTON_ACME_EMAIL" --agree-tos --non-interactive
for h in "$WONTON_NAMES_HOST" "$WONTON_PUSH_HOST" "$WONTON_RELAY_HOST"; do
  certbot certonly --webroot -w /var/www/letsencrypt -d "$h" --email "$WONTON_ACME_EMAIL" --agree-tos --non-interactive
done
certbot certonly --expand --webroot -w /var/www/letsencrypt \
  -d "$WONTON_OMARCHY_HOST" -d "omarchy.$WONTON_DOMAIN" \
  --email "$WONTON_ACME_EMAIL" --agree-tos --non-interactive
systemctl reload nginx
certbot renew --dry-run
```

把 `certbot renew` 的 deploy hook 设成 `systemctl reload nginx`（`/etc/letsencrypt/cli.ini`
或 `renewal-hooks/deploy/`）。占位证书 7 天到期前必须完成这一步。

### 4. 本机自检

```sh
curl -sk --noproxy '*' -o /dev/null -w '%{http_code}\n' "https://$WONTON_NAMES_HOST/health"   # 200
curl -sk --noproxy '*' -o /dev/null -w '%{http_code}\n' "https://$WONTON_PUSH_HOST/healthz"    # 200
curl -sk --noproxy '*' -o /dev/null -w '%{http_code}\n' "https://$WONTON_RELAY_HOST/healthz"   # 200
openssl s_client -connect 127.0.0.1:443 -servername "unknown.invalid" </dev/null 2>&1 | grep unrecognized
```

HTTP vhost 也要验：`curl -s --noproxy '*' -H "Host: $WONTON_DOMAIN" http://127.0.0.1/ -o /dev/null -w '%{http_code}\n'` 应为 301。

### 5. 改注册商 NS（切换点）

阿里云域名控制台，逐项：

1. 域名列表 → `@DOMAIN@` → 管理 → 左侧「DNS 管理」→「自定义 DNS Host」→「创建 DNS 服务器」：
   DNS 服务器 `ns-auth`，IP `@PUBLIC_IP@`，确定；再建 `ns2-auth` 同 IP。
2. 若「安全设置」里的「禁止更新锁」开着，先关闭（短信验证）。
3. 同页「DNS 修改」→「修改 DNS 服务器」→ 填 `ns-auth.@DOMAIN@`、`ns2-auth.@DOMAIN@` → 确定 → 短信验证。
4. 等 24–48 小时。期间 `dig +short NS @DOMAIN@ @8.8.8.8` 会新旧混杂，属正常。
5. 全量验证：`dig +short NS @DOMAIN@ @8.8.8.8`、`@1.1.1.1`、`@114.114.114.114`
   都只剩两条 `ns-*.@DOMAIN@` 后，跑 `scripts/verify.sh` 并保存输出。

### 6. 注册第一个家

在 `names.@DOMAIN@` 上创建邀请，把 `https://names.@DOMAIN@/i/<code>` 发给对方：

```sh
install -m 644 "$ROOT"/bootstrap/install.sh /var/www/  # 可选：给不带邀请的用法
# 邀请由 wonton-home 管理命令发出（见 crates/wonton-home/README.md）
```

对方执行 `curl -fsSL "https://names.@DOMAIN@/i/<code>" | sudo sh`。脚本把服务地址
和邀请写进安装器，用户不需要知道域名，也不需要第二次粘贴。

`/i` 脚本（和静态 `/install.sh`）会先从 `/relay-ca.pem` 下载 frps 的 CA，并导出安装器
强制要求的 `WONTON_ACME_DIRECTORY`、`WONTON_ACME_ACCEPT_TERMS`、`WONTON_FRP_CA_FILE`；
缺一个安装器就会在装任何东西前停下。`/relay-ca.pem` 就是 `$ROOT/frps/ca.crt`，是公开材料。

### 7. 备份计时器

```sh
install -d -m 755 "$ROOT"/scripts
install -m 755 scripts/backup.sh "$ROOT"/scripts/backup.sh
install -m 644 systemd/wonton-entry-backup.{service,timer} /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now wonton-entry-backup.timer
```

## 回滚

1. 注册商「DNS 修改」改回原来的两条 NS，删除两个自定义 DNS Host；等旧 NS 恢复。
2. `docker compose down`。
3. `tar -C / -xzf /root/nginx-certbot-*.tgz`，`systemctl restart nginx`。
4. `rm /etc/nginx/sites-enabled/{push,relay,omarchy,names,apex}.conf` 视情况保留。
5. 数据和区都在 `$ROOT`，不删；确认恢复后再决定是否清理。

## 每月检查

`scripts/verify.sh`；`certbot certificates` 剩余天数；`systemctl list-timers` 有备份；
`df -h`；`docker compose ps` 全 healthy；dnsdist 的 53 只对本机之外开放必要来源。
