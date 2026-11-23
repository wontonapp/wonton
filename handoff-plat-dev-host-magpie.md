# HANDOFF plat-dev-host-magpie（#57 Linux 宿主经转发用宿主 magpie 跑冒烟）

写于 2026-11-24 02:1x。**复审（rv-948）的意见已全部改完，整条冒烟在复审改完后重跑通过。**

## 结果

`stack-slot.sh acquire --priority --project wonton-smoke plat-dev-host-magpie -- ./deploy/dev/core/smoke.sh`
（1k，端口 18491/18492 避开别的栈，宿主 magpie 是那台上的容器，模型 `upstream/group/main`）：01:45:33 开跑，
02:09:41 结束，24 分钟，151 条断言全过，`端到端冒烟通过：安装 30s，卸载 1s`。模型调用 77 次、输入 95,988 /
输出 15,872 token。日志 `plat-dev-host-magpie-host/1k-logs/smoke-1k17.log`。

## 复审改了什么

1. `gateway-key models` 只传 `$model`（参数必须带斜杠），期望 `only $model`；README 补回“填 `/v1/models` 的 id”。
2. 同名凭据按建前/建后 id 差集取新建的那条，断言按 id；不再删错、留错。
3. cleanup 和 SMOKE_ONLY 的 trap 用子壳兜住 `host_magpie` 的 `exit`。
4. `WONTON_SMOKE_MODEL_RELAY` 设了就在两种宿主上都直接用。
5. 本文件改成 `handoff-plat-dev-host-magpie.md`，不和 main 的 `handoff.md` 撞名。
6. 两个核心改动各加行为测试（安装中让网关就绪、清掉 listed 后让网关到达）。
7. 卡片文案不再说房间；起转发后查容器活着；设了 `WONTON_SMOKE_HOST_MAGPIE` 就一律进容器（README 同）；
   探名字的命令坏了就 fail。

## 现场已清

1k 的栈、转发、它 magpie 上的 `upstream` provider、npchy 的 `platdev-upstream` key、1k 的检出和 host 目录都删了；
194 上没有残留容器。

## 剩下的活

推这个提交、把新 head 记到卡上（待审），等 rv-948 复审新 head。

## 复跑要点

- 1k 默认端口 18481-18483 常被别的栈占，先看 `ss`，必要时 `WONTON_DEV_PORT`/`WONTON_DEV_TUNNEL_PORT` 换一对。
- 上游 DeepSeek 会整轮不返回：等 20 分钟或 restart 那个 gateway，探针会换设备重试；冒烟失败即 exit。
- `KEEP=1` 会留栈，重跑前 reset；`fuser ~/wonton-dev/.core-build.lock` 列出的一并杀掉。
- 没缓存的镜像先备好，否则撞 `dev install` 的 10 分钟上限。
