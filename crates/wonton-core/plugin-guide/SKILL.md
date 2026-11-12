---
name: wonton-plugin
description: 写一个 Wonton 插件，装到主人的服务器上，让主人在 App 里用。主人想要一个新页面、新应用、小工具，或者要改你之前写的插件时用。
---

# 写一个 Wonton 插件

插件是一个目录：`docker-compose.yml`，加上它引用的文件。你没有 Docker，也不能构建镜像，所以只有一种写法：**公开镜像加 `configs`**，代码放在插件包里。页面写普通网页，不要编译。

完整规范：<https://github.com/wontonapp/wonton-docs/blob/main/plugin-spec.md>。

## 步骤

1. 从 `assets/` 选一个例子复制进 outbox 下的新目录（`outbox/<名字>/`）。只有页面用 `static`，要存数据或抓外部内容用 `backend`。
2. 改 `id`（`app.<你的名字>.<插件名>`）、`title`、`description`、页面和代码。**只申请用得到的权限**，主人会逐项看。
3. 调 `wonton__check_plugin`，`dir` 是 `<名字>`。它一次列出全部问题，改到没有 `problems`。这一步不打扰主人，要改多少次都行。
4. 调 `wonton__install_plugin`，`dir` 同上。主人收到确认卡片，工具一直等到主人回答。
5. 失败时错误里有容器日志，旧版本不受影响。改好再装。同一个错误改两次还不对，就把错误告诉主人，别一直试。
6. 装好后调 `wonton__plugin_status` 看网页探测的状态码。你看不到渲染出来的样子，请主人看一眼。
7. 主人要改时，改同一个目录，再调一次 `wonton__install_plugin`。数据保留，失败回滚，版本号不用改。权限没变宽时不用主人确认，装好后在聊天里说一声改了什么。主人自己在 App 里装过或升级过的插件，你的更新会发卡片请主人确认，同意后这个插件归你管；被拒绝后 10 分钟内不会再发卡，别一直重试。

要能退回时，在 outbox 的目录里用 git 提交。要退回就再装一次旧版。

## 检查的两档

`wonton__check_plugin` 的问题必须清零，提醒可以留着：私用（自己装、Agent 写的，从目录安装）到这一步就够了。要上官方商店，还得通过 `--official` 那一档，比私用多要求：必须提供图标；镜像按摘要固定；长期运行的服务在 compose 里写健康检查；`tags`、`license`、`author`、`notices` 必填；`description`、`tags`、`about`、`release_notes`、`setup` 写了就要 `zh` 和 `en` 都有；代理地址交给核心注入，不写死；用请求头认用户的应用只信核心的代理地址（出网插件做不到就不收）。上架流程见[插件的开发指南](https://github.com/wontonapp/wonton-docs/blob/main/develop.md)。

## 不能写的

`ports`、`build`、`networks`、`privileged`、`devices`、`cap_add`、`container_name`、`env_file`、`extends`，以及相对路径的卷。写了都报错。

- 网页经核心网关访问，声明 `webui` 就行，不要 `ports`。
- 代码和页面用 `configs` 交给容器，不用卷。`configs` 的 `target` 不能在卷的挂载点里。
- 数据放 `${WONTON_APP_DATA}`，挂成 `/data`。容器里别的地方在更新时都会丢。
- 单个文件不超过 1 MiB，整个插件包不超过 2 MiB。依赖尽量用标准库。

## 一定要写的

- `init: true`。否则程序作为 1 号进程不响应停止信号，每次更新都多等 10 秒，这段时间页面打不开。
- `healthcheck`，带 `start_period` 和 `start_interval`。检查真正可用的地址，只用镜像里有的命令。
- `user: "${PUID}:${PGID}"`，写数据时用。这样数据目录里的文件归主人，不归 root。

## 容易猜错的

- **`configs` 的属主：** `user` 写 `${PUID}:${PGID}` 时按插值后的数字算，默认模式 `0444`，所有用户都能读。不用另写 `uid`、`gid`。
- **健康检查和代理：** 只有 `wget` 和 shell 的写法由核心改写成不走代理。`python3 -c` 不改写，用 `urllib.request.build_opener(urllib.request.ProxyHandler({}))` 自己绕开，见 `assets/backend`。
- **没有登录的页面：** `sso: none` 又不做登录是可以的，所有请求先经网关校验主人的会话，别人打不开。不要自己加默认密码。
- **出网：** 要访问外部地址就申请 `network.egress`，这个权限同时能访问主人的局域网，主人确认时看得到。用户填的地址不用替他拦，但不要把抓回来的内容不转义就放进页面。
- **跨源隔离：** 页面需要 `SharedArrayBuffer` 时，声明 `webui.cross_origin_isolated: true`。网关会加 `Document-Isolation-Policy: isolate-and-credentialless`；在 Android 上，这会放宽隔离，页面可能和其他网页共用进程。首次安装会提示主人，从未声明改为声明的更新必须重新确认。

## 让页面像 App 的一部分

- 颜色和字体用 CSS 变量，带后备值，`assets/` 的例子都这样写。这样跟随 App 的明暗和重点色。按钮用 `--wonton-accent-fill` 和 `--wonton-accent-ink`。
- 不要自己画应用外壳，App 的页面是整页，只带右上角的胶囊。页面内部的分段自己做。
- `tabs` 声明应用的页面，图标要单色、背景透明。它们出现在侧边栏的“我的应用”里，放不放由主人决定。
- 给 `html` 设背景色，Home 条后面露出的是它。
- 输入框字号不小于 16px。更小的字号在手机浏览器里一点就放大页面，App 里拦住了，浏览器里拦不住。
- 带“讨论”这类按钮时用 MCP Apps 的 `ui/message`，文字进输入框，主人点发送才发。

## 常见报错

| 报错说的 | 怎么改 |
| --- | --- |
| `ports` 不支持 | 删掉，用 `webui.port` |
| `build` 不支持 | 用公开镜像加 `configs` |
| 未知字段 | 按报错列出的可选字段改 |
| 引用的文件不存在 | 路径相对 compose 所在目录，不能有 `..` |
| 权限尚未支持 | 换一种做法，或者告诉主人做不到 |
| `unhealthy`，带日志 | 看日志里的报错。常见：写了只读路径（改写到 `/data`）、数据目录没权限（加 `user`）、端口和 `webui.port` 不一致 |
| 镜像没按摘要固定、没有图标（提醒） | 不影响安装，可以不管 |
| 没有变化 | 目录和上次相同，确认改到了对的目录 |
