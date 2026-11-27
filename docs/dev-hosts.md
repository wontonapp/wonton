# 开发机代号、真实部署域与占位

公开仓里会同时出现真实域名、内部机器代号和占位地址。这里说清哪些是真值、哪些是占位、换域名该改哪，新增文档照着写。

## 开发机代号

下面几个是内部机器的代号，可以出现在公开仓里。它们不是域名，也不是产品名：

| 代号 | 用途 |
| --- | --- |
| `npchy` | 主开发、运维 |
| `194` | 构建、验证 |
| `1k` | 远程开发和调试 |
| `mini` | iOS 构建 |

## 真值与占位

| 值 | 性质 |
| --- | --- |
| `808cbd.com`，及子域 `relay.808cbd.com`、`push.808cbd.com` | 真实部署值 |
| `omarchy.online` | 真实部署值；同一台机上主人另一个站，不是 Wonton 服务域 |
| `192.0.2.10` | 占位，文档示例地址 |
| `example.com` | 占位 |
| `*.wonton.internal`、`wonton.localhost` | 产品内部默认：Docker 网络别名和开发默认，不对外 |

真值不匿名化，已经写下的历史文档也不回填；新写的地方在旁边标一句“真实部署值，不是占位”即可。

## 写法约定

- 只在讲真实部署时写真值，并紧挨着标注；举通用例子用占位，别拿真值当“随便举个例子”。
- 占位地址统一写 `192.0.2.10`（`AGENTS.md` 的提交约定），不要自造 `10.0.0.x`。
- 家目录写 `~/…`，不写 `/home/<用户>`、`/Users/<用户>` 这类机器路径。
- 内部机器代号（`npchy` 等）直接写，不用换成 `host1`。

## 换域名要改哪

换真实域时，改动集中在安装脚本顶部的 `DOMAIN`（只有换掉主域才连 `OWNER_DOMAIN` 一起改）和 nginx vhost 的 `server_name`；`DOMAIN` 还会流到推送地址来源 `relay.json`，一起改。证书目录 `/etc/letsencrypt/live/$DOMAIN`、certbot 的 `-d` 都从 `DOMAIN` 推出来，不用单独改。带域名的 vhost 文件名被脚本按字面引用，改域时文件名和引用一起改。

- iroh relay：`DOMAIN` / `OWNER_DOMAIN`（[install.sh:22](../cloud/iroh-relay/install.sh#L22)）、`server_name`（[relay.808cbd.com.conf:7](../cloud/iroh-relay/nginx/relay.808cbd.com.conf#L7)）。
- push relay：`DOMAIN` / `OWNER_DOMAIN`（[install.sh:20](../cloud/push-relay/install.sh#L20)）、`server_name`（[push.808cbd.com.conf:6](../cloud/push-relay/nginx/push.808cbd.com.conf#L6)）、推送地址来源（[relay.json:5](../cloud/push-relay/relay.json#L5)）。
- 一次看全其他硬编码：`rg -n '808cbd\.com' cloud crates`（`uninstall.sh`、`cert-deploy-hook.sh`、`verify.sh` 和 bootstrap vhost 里也有）。

`omarchy.online` 不是 Wonton 的域，换 Wonton 域不用动它；只有整台机换域才一起改，见 [iroh README](../cloud/iroh-relay/README.md)。

## 查一遍

```bash
rg -n '808cbd\.com|omarchy\.online' --glob '!docs/dev-hosts.md' .
rg -n '\b(npchy|194|1k|mini)\b' docs AGENTS.md
rg -n '192\.0\.2\.10|example\.com|wonton\.internal|wonton\.localhost' docs
```

命中不等于错：按“真值与占位”一节判断是漏标还是正常占位；真值缺标注就补一句。
