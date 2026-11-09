# 回声 Bot

最小的 Matrix Bot 示例插件，代码在 [`crates/wonton-echo-bot`](../../../crates/wonton-echo-bot)。它只用核心注入的
`WONTON_MATRIX_*`：按令牌恢复会话，把名单里的人（`WONTON_MATRIX_ALLOWED_USERS`）发来的文字原样回成 `回声：<原文>`，加密房间里也一样；名单里的人邀请它进别的
房间时接受。它没有网页、不申请出网，Matrix 也只经核心在插件网络里的别名访问。

镜像在本地构建（`pull_policy: never`），然后像别的示例一样安装：

```bash
docker build -f deploy/dev/core/Dockerfile --target echo-bot -t wonton-echo-bot:dev .
deploy/dev/core/dev.sh install plugins/examples/echo-bot
```

装好后 App 里会出现和“回声”的加密私聊。`echo-bot probe` 在冒烟测试里扮演主人的设备，经隧道发一条加密消息、
等回声，用法见 `crates/wonton-echo-bot/src/probe.rs`。

## 图标

`icon.svg` 的来源及许可见[自制图标](../../README.md#自制图标)。
