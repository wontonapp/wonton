# HANDOFF plat-dev-host-magpie（#57 Linux 宿主经转发用宿主 magpie 跑冒烟）

写于 2026-11-23 23:5x，npchy。**整条冒烟已在 1k 的开发栈跑绿。**

## 结果

`stack-slot.sh acquire --priority --project wonton-smoke plat-dev-host-magpie -- ./deploy/dev/core/smoke.sh`
（1k，宿主 magpie 是那台上的容器，模型 `upstream/group/main`）：23:23:54 开跑，23:47:48 结束，24 分钟，
151 条断言全过，`端到端冒烟通过：安装 26s，卸载 1s`。模型调用 88 次、输入 128,286 / 输出 23,882 token，全 200。
整份日志在 `plat-dev-host-magpie-host/1k-logs/smoke-1k15.log`。

## 分支

`test/dev-host-magpie`。主要改动：冒烟在 Hermes 容器里探 `host.docker.internal`，解析不到就在宿主机起 socat
转发、用 Hermes 网络网关连；宿主 magpie 的 CLI 走 `host_magpie()`；Hermes v0.8.0 不申请 models，冒烟改为在宿主
magpie 上按 Bot 名字建限模型凭据、经 Hermes 自己的 provider 接口配它（上下文按 64k 下限设，ask 前每轮对一遍）；
卡片改读确认列表（不再读 Matrix 房间，`[[ == ]]` 每项补尾随空格）。README 同步。

## 现场已清

1k 的栈、转发容器、它 magpie 上的 `upstream` provider、npchy 上的 `platdev-upstream` key、1k 的检出和 host 目录
都删了；194 的栈和临时 magpie 容器上一轮已拆。只有 `/tmp` 里几个 root 属主的旧数据目录还在（tmpfs，重启即清）。

## 剩下的活

1. 推分支、开草稿 PR（正文 `plat-dev-host-magpie-host/pr-body.md` 的“整条冒烟”已填好），卡改待审。
2. 在 issue #57 贴 `plat-dev-host-magpie-host/issue-57.md`（已填实测），本条打勾，不关 issue。
3. 等复审/CI；负责人转来的红再修。

## 已知的坑（复跑时）

- 上游 DeepSeek 很慢、偶尔整轮不返回：MCP/存文件步骤会等 5～20 分钟，`ask_hermes` 的 `--wait` 是 20 分钟；
  真挂住就 `docker restart` 那个 gateway、等 step 失败后用重试（探针会自己换设备重发）。
- 冒烟失败会 `exit`（不是只红一步）。KEEP=1 会留栈，重跑前必须 reset；run.sh 的 flock 会被上一轮遗留的
  probe/build 占着，重跑前把 `<data>/` 清了并杀 `fuser ~/wonton-dev/.core-build.lock` 里列出的进程。
- 首次跑没缓存的 Grafana 镜像会撞 `dev install` 10 分钟上限；先 `docker pull`/从别处 load 再跑。
