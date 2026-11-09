# ADR-0074 镜像加速按仓库分开：ghcr 和 quay 各有自己的名单，一个站不行换下一个

- 状态：已接受（2026-11-09，主人同意）
- 取代：[ADR-0040](0040-registry-mirror.md) 的“只加速 `docker.io`”和“同时只用一个”

## 背景

国内主机直连不了 ghcr.io，比 Docker Hub 还彻底：官方索引里我们自己的插件镜像、ACP 桥镜像，以及 immich、paperless-ngx、
open-webui、karakeep 的上游镜像都在 ghcr。quay.io 上是 minio、node-exporter 这类常用的上游。ADR-0040 只加速
`docker.io`，这些镜像照旧直连，装到一半卡在拉镜像。

ADR-0040 还有一条“同时只用一个加速地址”。公开加速服务说停就停，换了站只能由主人去设置里换；一个站限流或者只挂了
ghcr 的路径，整次安装就失败，明明名单里别的站还活着。

## 决策

### 名单按仓库主机分开

官方索引的 `registry_mirrors` 从一维数组改成按仓库主机分的表，每个仓库一个有序列表：

```json
{
  "docker.io": ["dockerproxy.net", "docker.1panel.live"],
  "ghcr.io": ["ghcr.dockerproxy.net", "ghcr.nju.edu.cn"],
  "quay.io": ["quay.dockerproxy.net", "quay.nju.edu.cn"]
}
```

- 只认这三个仓库；别的仓库（localhost、自建 registry、gitlab）照旧直连，没有名单。
- 一维列表不再兼容：这是第一个外部用户之前，按 [AGENTS.md](../AGENTS.md) 的约定直接改。上线顺序是核心先合、索引后合：
  新核心读到旧索引时整份名单读不懂，设置页显示一条问题、地址只能手填，插件照旧装得上；反过来旧核心读到新索引也是
  “读不懂一条问题”。索引的 `registry_mirrors` 读不懂只作废这一份名单，不让整个索引读不了（`plugin check-index` 的路不变）。
- **设置里的开关总控**：关掉加速时 ghcr、quay 的名单也不用，全部直连；打开时三类仓库都按名单走，Docker Hub 用主人选的那个站排在最前面。
- 设置页和协议不动：`choices` 仍只列 `docker.io` 的站，`mirror` 仍是主人选的 `docker.io` 地址。ghcr、quay 的名单这次
  只进核心，App 不显示。

### 按顺序换站

一个引用按名单顺序逐个试，前一个做不成再试下一个：

- 连不上、超时、TLS 错、被 401/403/429/5xx 拒、按摘要拉下来但摘要对不上，都算这个站失败。
- **主人手填或者在 App 里选的 `docker.io` 地址排在最前面**，其次是索引里的顺序。同一个地址在索引里也有时只试一次。
- 全都失败就报“镜像拉不下来”，详情写清是哪个仓库、试了哪几个站、每个站各自的失败。**不改成直连**：打开加速就是因为
  直连不通，悄悄回退只会变成长时间的卡住（ADR-0040 的理由不变）。
- 成功以后的核对照 ADR-0040：Docker 按摘要核对内容，核心再核对本地镜像的 RepoDigests；原引用能解析就用原引用建容器，
  解析不了才用加速地址的引用。

仍然是**只加速按摘要钉住的引用**：只写标签的引用没有能核对的东西，加速站给什么都会被收下，一律直连。

## 没选

- **所有仓库共用一份名单。** 一个站点通常只代理一个仓库的路径，`dockerproxy.net` 和 `ghcr.dockerproxy.net` 是两台不同的
  服务；共用一份名单要每个站都试每个仓库，失败次数乘上去。
- **全失败后回退直连。** 理由同上；而且回退后错误里混着直连的错误，看不出加速站到底哪一步坏了。
- **保留一维列表的写法做兼容。** 第一个外部用户之前，留两套写法只会让索引维护的人猜该写哪套。代价是靠上线顺序挡：
  新核心先把两种形状里的一种当常态（空名单），旧核心升级前设置页显示一条问题。
- **写 `daemon.json` 的 `registry-mirrors`。** ADR-0040 已经否掉：要 root、要重启 dockerd，失败时 Docker 悄悄回退，
  而且 `daemon.json` 只对 `docker.io` 生效。
- **替 ghcr、quay 维护一份同摘要副本。** 要我们替每个上游复制，还要能在插件仓库的 Actions 里跑；只解决我们自己的镜像。

## 实测

2026-11-09，一台国内主机（Docker 29，宿主机直连，不经代理），按 ADR-0040 的办法：带 Docker 的 User-Agent 按摘要取
清单，核对摘要，再取 amd64 最大的一层 8 秒记速度；跳转后不带 Authorization，回 401 时按挑战匿名换令牌。

| 仓库 | 地址 | 取清单 | 摘要对 | 下载速度 | 说明 |
| --- | --- | --- | --- | --- | --- |
| ghcr.io | ghcr.io | 1.5 秒 | 是 | 15.0 MB/s（30 MB 层）、331 MB 层 15.6 MB/s、414 MB 层 8 秒没下完 | 这台通，但大层明显慢 |
| ghcr.io | ghcr.dockerproxy.net | 0.9 秒 | 是 | 14.4、33.9、36.4 MB/s | 三张镜像都最快 |
| ghcr.io | ghcr.nju.edu.cn | 0.2–0.7 秒 | 是 | 20.5、18.2、20.1 MB/s | 能用，清单最快 |
| ghcr.io | ghcr.1ms.run | 1.3 秒 | 是 | 10.6、12.5 MB/s | 能用 |
| ghcr.io | ghcr.m.daocloud.io | 403 | | | 有白名单或者要登录 |
| ghcr.io | ghcr.chenby.cn | 换令牌时 403 | | | 要登录 |
| ghcr.io | ghcr.geekery.cn、ghcr.xuanyuan.me、ghcr.static.cn | 解析不到 | | | |
| quay.io | quay.io | 0.2–0.7 秒 | 是 | 1.2 MB/s（0.8 MB 层）、12 MB 层 8 秒没下完 | 清单能取，层慢 |
| quay.io | quay.dockerproxy.net | 1.0–1.3 秒 | 是 | 0.8、7.5 MB/s | 能用 |
| quay.io | quay.nju.edu.cn | 0.2 秒 | 是 | 3.1、16.8 MB/s | 最快 |
| quay.io | quay.m.daocloud.io | 0.4 秒 | 是 | 0.8、6.9 MB/s | quay 走它通，ghcr 的路径不通，再次说明要按仓库分 |
| quay.io | quay.1ms.run | 2.9–4.7 秒 | 是 | 1.2、8.5 MB/s | 清单慢 |
| quay.io | quay.chenby.cn | 1.0 秒 | 是 | 1.0、7.6 MB/s | 能用，ghcr 的路径要登录 |
| quay.io | quay.xuanyuan.me、quay.static.cn | 解析不到 | | | |

同一段时间里的两次速度能差几倍，所以名单要放在能随时改的索引里，设置页的测速也留着（只测 `docker.io` 的站）。
这些站能不能换上，就看哪一个失败得快；一个站挂了只多一次超时，不影响后面的站。

Docker Hub 的站没有重测，沿用 ADR-0040 的实测。

## 代价与风险

- 名单从一份变成三份，维护量增加；一个地址可以只挂在它真正代理的仓库下面，别的仓库不会去试它。
- 加速站能看到这台主机拉了哪些 ghcr、quay 的镜像。换不了镜像内容：内容由摘要核对。
- 强制的顺序意味着一个坏站在名单里就是每次安装多一次超时，站点维护的人要及时删掉。
- 旧核心读不懂新索引的名单，设置页会一直显示一条问题，直到核心升级；插件安装不受影响（只是得不到自动地址）。
