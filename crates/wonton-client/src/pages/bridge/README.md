# 插件页桥

协议边界见 [ADR-0033](../../../../../docs/adr/0033-plugin-tabs.md#桥)。令牌映射的输入是
`design/tokens.toml` 的 `web_variables`，`scripts/generate-design.py --check` 检查 Rust 生成表；
`--wonton-*` 只注入根元素，不进入 SDK 的 `styles.variables`。

## 壳接入

1. 壳把 WebView 绑定到一个 `TabRef`。
   iOS 注册 `webkit.messageHandlers.wontonBridge`，Android 用 `addWebMessageListener("wontonBridge", …)`。
   主框架标志和 `origin` 取自原生回调：iOS 的 `frameInfo.securityOrigin`，Android 的 `sourceOrigin`，不能从消息正文取。
2. 文档开始前注入 `ClientCore.tab_document_script(tab, environment)`，先于页面脚本执行。
   公网网页（`TabRef::Url`）没有桥：脚本接口返回 `CoreError.InvalidInput`，壳按页面种类跳过注入和原生通道，
   `--wonton-*` 和 `--wonton-inset-*` 都没有（ADR-0052“公网网页”）。
   `environment.dark` 是按 App 设置和系统合成的有效明暗；`platform` 只选择平台圆角，对页面统一报 `mobile`。
   安全区和栏高使用 WebView 的 CSS px，`inset_top/bottom` 已包含安全区；核心把 `inset` 布局的两个栏高变量归零。
3. 原生通道收到字符串后调用 `tab_handle_message(tab, is_main_frame, origin, message, environment)`。
   公网网页的实例即使注入了也一律回 JSON-RPC `-32001`，`initialized` 恒为假。
   核心比较来源与 `TabPage.url` 的 scheme、host 和有效端口；非主框架、认证域名、其他来源或页面 URL 缺失时均忽略。
   壳不另写来源判定，把 `reply_script` 原样注入同一个页面；`initialized` 为真时可撤骨架。
   只按 `action` 枚举打开系统链接或切到聊天填草稿，不自动发送、不解析 JSON-RPC。
4. 订阅外观和平台环境变化，调用 `tab_context_script` 注入。
   字号由壳应用：iOS 保持 `pageZoom = 1`，用 `-webkit-text-size-adjust` 放大文字；Android 用 `textZoom`。
   然后通知环境变化，不往 SDK 上下文加自定义字号字段；安全区和栏高不随文字比例换算。
5. 重选当前标签时注入 `tab_reselected_script()`；连接状态变化时注入 `tab_connection_script()`。
   后者的 `CustomEvent.detail` 为 `{ connected: boolean }`，只在核心确认为 Connected 时为真。

`reply_script` 派发来源为当前 window 的合成 `MessageEvent`。文档脚本在捕获阶段拦住真实的 self-postMessage，
避免 SDK 把自己的请求当成宿主请求；原生回复不再次转给原生。`window.wonton` 不存在。

## widget 页

`TabRef::Widget { plugin_id, room_id }` 是插件的 Matrix widget 页（[ADR-0096](../../../../../docs/adr/0096-calls-matrixrtc-widget.md)），只经 `Action::OpenPage` 从房间里打开、压入页面栈，不能加入“我的应用”或面板。同一时间只保留一个 widget 页；被其他页面盖住时驱动继续运行，出栈时结束。上面 1–3 步照旧，另加：

- `TabPage.url` 出来、页面开始加载后调用 `subscribe_tab_widget(tab)`，循环 `next()`，把每段脚本原样注入同一个页面的主框架；
  返回空（页面关了、换了房间、退出登录、重新加载）就停，`cancel()` 用于壳自己先拆掉页面时。
- 页面发来的 widget 报文照常交给 `tab_handle_message`，`reply_script` 为空，应答从订阅回来。

## 每 Bot 网页接口

每 Bot 页面身份使用 `TabRef::Bot { bot: BotRef, path: Option<String> }`，不保存 URL，不降成
`TabRef::Plugin` 或 `TabRef::Url`。`path: None` 打开对应 Bot 的 `BotView.web.Ready.page.url`；
`Some(path)` 只接受该 origin 内的路径，可含查询和片段。打开沿用 `Action::OpenPage`，加入沿用
`AddMyApp`、`PinCurrentPage` 和 `Panel`，`PlacedTab` 保留同一 Bot 身份和路径。

实现时把聊天已有的 `BotView` watch 通道接入 `PagesLink`，Bot 网页状态变化时重算 `PagesViewModel` 的
`TabPage`；URL 只能来自精确匹配该 `BotRef` 的 `BotView.web.Ready.page.url`，不能从 `plugin.web_url`
取值、猜地址或回退默认 Bot。`BotView.web: None` 表示没有验证可用的 URL；未知、缺失或错误时不给页面
URL，断连或身份变化时清空。桥的来源判定仍按 `TabPage.url` 的 scheme、host 和有效端口，不另写 Bot 来源规则。

Bot 页沿用上述 WebView 容器、文档／消息／上下文接口、`--wonton-*` 和既有媒体授权接口，后者同样通过
`TabRef`／`PlacedTab` 取得该精确页面 origin；不能另建壳层 WebView，也不能按公网网页跳过桥或令牌注入。
这是接口约定，不是运行时入口接线或端到端验证结论；接口阶段不暴露 Bot 页面，未支持的客户端明确拒绝，
不能转开其他页面。

## 验证

`cargo test -p wonton-client --locked --test it tabs_bridge` 覆盖 Rust 行为，
`cargo test -p wonton-client --locked --test it tabs_bridge::sdk::official_sdk_roundtrip -- --ignored --exact`
用固定摘要的官方 Node 22 镜像跑 SDK 双平台通道。widget 页对真实 Tuwunel 的行为测试是 `cargo test -p wonton-client --test it chat::widget -- --include-ignored`，页面用固定版本的官方 matrix-widget-api。SDK 来源、许可证和驱动协议见
[测试夹具](../../../tests/it/tabs_bridge/fixtures/README.md)。
