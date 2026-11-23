# HANDOFF plat-dev-host-magpie（#57 Linux 宿主经转发用宿主 magpie 跑冒烟）

写于 2026-11-24 05:0x。**第二轮复审（1k-rv-948b）的意见已全部改完，带宿主同名旧条的冒烟重跑通过。**

## 结果

`stack-slot.sh acquire --priority --project wonton-smoke plat-dev-host-magpie -- ./deploy/dev/core/smoke.sh`
（1k，端口 18491/18492 避开别的栈，**先在宿主 magpie 上预置一条同名旧凭据**，模型 `upstream/group/main`）：
04:23:09 开跑，05:01:20 结束，38 分钟，151 条断言全过，`端到端冒烟通过：安装 34s，卸载 1s`；跑完旧条还在、
只删了冒烟建的那条。模型调用 80 次、输入 141,572 / 输出 21,962 token。日志 `plat-dev-host-magpie-host/1k-logs/smoke-1k18.log`。

## 第二轮复审又改了什么

- 卸载断言按 id，`host_model_key_id` 在断言之后清空；带旧条实测通过。
- 新增两条用例的容器名 `app.test` → `app-test`。
- 等网关挪到拿 `dependencies.changing` 之前（起网关的 start 也要那把锁），`provision_models` 直接取状态。
- `smoke.sh:328` 的 shellcheck 改 if 去掉 SC2015；README 示例改成 `/v1/models` 里的 id。

## 首轮复审改了什么

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
