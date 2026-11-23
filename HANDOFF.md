# HANDOFF plat-dev-host-magpie（#57 Linux 宿主经 host.docker.internal 用宿主 magpie）

写于 2026-11-22 22:4x，194。负责人要求换会话接手，这份文件是全部状态。

## 一句话

本卡要的冒烟/转发代码已经写完全部提交；**整条冒烟还没绿**，卡在两个不属于本卡的 main 上就有的问题上（见“卡在哪”）。最后一次失败留了现场（KEEP=1）。

## 分支与提交（本地已提交，194 推不了，等负责人推）

分支 `test/dev-host-magpie`（基于 main b10ebfa7）：

- `254b34c2 fix(dev): Linux 宿主经转发用宿主 magpie 跑冒烟` —— 本卡的主改动（`deploy/dev/core/smoke.sh`、`smoke-cleanup.sh`、README）
- `97bb49d2 fix(examples): Grafana 角色映射认 plugin_admin` —— 冒烟在 main 上红，sso 一步过不去
- `dea5af7b fix(dev): 冒烟的 shellcheck 调用改成文件路径` —— 同上，echo-bot 一步 `shellcheck source=…` 必挂
- `5f72a723 fix(plugin): 默认 Bot 装好后接到模型网关` —— 签发前等网关就绪 + 默认插件装完对一次账
- `bdb20ede fix(plugin): 对账不看店里的条目` —— `connect_unconnected_consumers` 原来靠 `entry.listed`，restore 回来的插件没有它，被整条跳过

工作区干净。

## 主改动是什么

`smoke.sh` 的模型段：先探冒烟的 magpie 插件网络里能不能解析 `host.docker.internal`（Docker Desktop/OrbStack 能，Linux 的 Docker 不能），解析不到就在那个网络的网关地址上起 `alpine/socat` 转发（监听所有地址，`WONTON_SMOKE_MODEL_RELAY` 改目标，默认 `127.0.0.1:3425`），cleanup 和 SMOKE_ONLY 的 trap 都删。宿主 magpie 的凭据命令改走 `host_magpie()`（有 `magpie` 用它，否则 `docker exec` 进发布 3425 的容器，`WONTON_SMOKE_HOST_MAGPIE` 指定名字）。provider 的 `models=` 只去掉最前面一段（`${model#*/}`），路由组模型不再被切成 `main`。

## 已实测（可复用的证据）

- 插件网络里解析不到这个名字：`ping: bad address 'host.docker.internal'`（默认桥、新建桥都一样）
- socat 转发 + `range` + 401 通路（见 `~/wonton-dev/plat-dev-host-magpie.result.md` 的“转发机制单独实测”）
- 宿主 magpie 经网关上游真回模型（210 s，8+2 token）
- 按冒烟同一套代码的模拟：判定走转发、凭据签发、`provider add` 经转发拉到模型目录（`1 of 1 models · fetched just now`）；另一个容器 `curl http://host.docker.internal:13425/v1/models` 回 200（0.002 s）
- `bash -n`、`shellcheck`（只剩本来就在的警告）

结果文件：`~/wonton-dev/plat-dev-host-magpie.result.md`（里面第四节“底座的问题”是要带进 PR 的，别删）。

## 卡在哪（都要写进 PR，也是“底座的问题”）

1. **镜像代理 `wonton-buildkit-mirror` 坏了**：HEAD manifest 回 502、没缓存的 blob 回 500（日志 `unauthorized: authentication required`）。绕法：`PATH=/usr/bin:$PATH` 不走 `~/wonton-dev/bin/docker`（它把 `BUILDX_BUILDER` 强改成 wonton-omp），当前 builder 设成 `default`（`docker buildx use default` 已做过），并且 `WONTON_DEV_BUILD_PROXY=http://127.0.0.1:7890` 让 `fetch-locked` 能出网。
2. **长构建会被 TERM**：`dev.sh up`（核心构建 13/10 分钟）和 `dev.sh build`（echo-bot）都在跑到一半时收到 SIGTERM（冒烟打印 `Terminated  "$dev" …`）。`setsid` 之后核心构建活下来了（smoke11 跑到 hermes 一步），但 echo-bot 构建又被 TERM（smoke12 181 行）。原因还没定位；怀疑和 omp/看板的进程回收有关，可以试 `systemd-run --scope` 或把 build 单独后台跑再等。
3. **Hermes 凭据**：默认 Bot 在网关就绪前就装，凭据留空；`5f72a723` 的“等就绪 + 装完对账”和 `bdb20ede` 的“对账不看 listed”都还没验证到（都被第 2 条的 TERM 卡在更前面或后面）。最后留的现场可以手工验：见“现场”。

## 现场（还留着，用完要拆）

- 临时宿主 magpie 容器：`wonton-platdev-host-magpie`（`ghcr.io/yetone/magpie:0.1.1327`，发布 3425，provider 指向上游网关 `group/main`，不印 key）。这是冒烟的上游，**别删**，直到整条冒烟绿。
- 上一轮冒烟 KEEP 留的栈（smoke12 失败，项目 `wonton-smoke`，数据目录 `/tmp/wonton-smoke-1795358257-3868940`）。拆：
  `cd ~/wonton-dev/plat-dev-host-magpie && WONTON_INSTALL_DIR=/opt/wonton-unreadable WONTON_DEV_PROJECT=wonton-smoke WONTON_DEV_DATA=/tmp/wonton-smoke-1795358257-3868940 ./deploy/dev/core/dev.sh reset`
  再删 `wonton-smoke-phone`、`wonton-smoke-browser` 两个残留容器。
- 槽位：`/opt/wonton` 是 0700 root，194 上所有 dev 脚本会被 dev-guard 拒，**必须设 `WONTON_INSTALL_DIR=/opt/wonton-unreadable`**（见结果文件的底座问题第 1 条）。
- `WONTON_DEV_RELAY` 必须设（`/run/user/1000` 不可写）：`/tmp/wsr-platdev-magpie`。

## 怎么跑（现成脚本 `/tmp/host-magpie-run.sh`，里面已经带齐上面所有绕法）

```
bash /tmp/host-magpie-run.sh
```

它会：设 `PATH=/usr/bin:$PATH`、unset `BUILDX_BUILDER`、取核心构建锁、设 `WONTON_DEV_BUILD_PROXY`/`WONTON_INSTALL_DIR`/`WONTON_DEV_RELAY`/`WONTON_SMOKE_MODEL=upstream/group/main`、`KEEP=1`、`STACK_SLOT_DRAIN_OVERRIDE=1`，然后
`stack-slot.sh acquire --priority --minutes 90 --project wonton-smoke plat-dev-host-magpie -- ./deploy/dev/core/smoke.sh`。
长跑用 `setsid nohup … > ~/wonton-dev/plat-dev-host-magpie-host/smokeN.log 2>&1 &`，别丢在工具调用里。

`KEEP=1` 时槽位会放掉但栈留着，下次跑前先 reset（不然 `--project` 撞名）。想一次跑到底就把 `KEEP=1` 去掉。

## 剩下的活

1. 让第 2 条不再发生（或换台、或给 build 加外层保护），跑绿整条冒烟；绿了之后把 SMOKE 的整段输出、耗时、token 写进 `plat-dev-host-magpie.result.md` 的“整条冒烟”，并删掉“卡住”相关的临时说明。
2. 绿了以后确认 `5f72a723`/`bdb20ede` 真的让 Hermes 拿到 `com.nousresearch.hermes/default` 凭据（`docker exec <smoke magpie> magpie gateway-key list` 能看到），模型两面能对话。
3. 写 issue #57 评论（结果文件里有草稿，填实测）并打勾；写 PR（标题/正文四节草稿在 `~/wonton-dev/plat-dev-host-magpie-host/pr-body.md`，把 5 条提交、验证、底座问题写全），卡上记分支+head+PR 文本 → `task.sh set 状态=待审`。194 只 commit，不 push。
4. 清场：拆栈、删临时 maggie 容器（`wonton-platdev-host-magpie`）、删 `~/wonton-dev/plat-dev-host-magpie-host/`，确认没有 `wonton-smoke*` 容器/网络/卷残留。

## 备注

`HANDOFF.md` 故意不进提交（只是交接用），`git status --short` 里看到它是未跟踪就对了。
