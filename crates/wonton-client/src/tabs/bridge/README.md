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
4. 订阅外观、底栏状态以及平台环境变化，调用 `tab_context_script` 注入；全屏结果以后发布的 `TabsViewModel` 为准。
   字号由壳应用：iOS 保持 `pageZoom = 1`，用 `-webkit-text-size-adjust` 放大文字；Android 用 `textZoom`。
   然后通知环境变化，不往 SDK 上下文加自定义字号字段；安全区和栏高不随文字比例换算。
5. 重选当前标签时注入 `tab_reselected_script()`；连接状态变化时注入 `tab_connection_script()`。
   后者的 `CustomEvent.detail` 为 `{ connected: boolean }`，只在核心确认为 Connected 时为真。

`reply_script` 派发来源为当前 window 的合成 `MessageEvent`。文档脚本在捕获阶段拦住真实的 self-postMessage，
避免 SDK 把自己的请求当成宿主请求；原生回复不再次转给原生。`window.wonton` 不存在。

## 验证

`cargo test -p wonton-client --locked --test it tabs_bridge` 覆盖 Rust 行为，
`cargo test -p wonton-client --locked --test it tabs_bridge::sdk::official_sdk_roundtrip -- --ignored --exact`
用固定摘要的官方 Node 22 镜像跑 SDK 双平台通道。SDK 来源、许可证和驱动协议见
[测试夹具](../../../tests/it/tabs_bridge/fixtures/README.md)。
