# 官方 SDK 桥接夹具

`generated/ext-apps-2.0.4.mjs` 是 npm 官方
[`@modelcontextprotocol/ext-apps@2.0.4`](https://www.npmjs.com/package/@modelcontextprotocol/ext-apps/v/2.0.4)
的 `package/dist/src/app-with-deps.js`，逐字节保留，只改文件名。该预打包文件没有外部导入，
不需要本地打包器、node_modules 或运行时联网。上游提交：
[`7af9a75da720b8a81be0f9aff47b25f75e9530ad`](https://github.com/modelcontextprotocol/ext-apps/tree/7af9a75da720b8a81be0f9aff47b25f75e9530ad)。

SHA-256：

- npm tarball：`d12699dc721fe6444fab3df6efd9d888a485a44484ffbee3ef07c3caf82fa2fd`
- `.mjs`：`d24b6ae50dbc989539db8287e217f41f0833f2b0a81ac4c6dbde444715ef5cd0`

包元数据声明 MIT；上游随包 LICENSE 同时记录 MIT → Apache-2.0 的授权过渡，不能删减成
仅 MIT。`generated/LICENSE.ext-apps` 原样保留全部文本。其预打包依赖的通知也完整保留：
`@modelcontextprotocol/client` 2.0.0、`@modelcontextprotocol/core` 2.0.0（MIT 元数据、同样的过渡文本），
`zod` 4.6.6、`content-type` 1.0.5（MIT）。依赖版本取自该提交的 `package-lock.json`；
包内 `content-type` 版权注释也未修改。未使用文档内容，但随包许可证中的文档授权说明仍原样保留。

从仓库根目录复现获取（中间产物在仓库外，无构建步骤）：

```sh
fixture="$PWD/crates/wonton-client/tests/it/tabs_bridge/fixtures"
tmp="$(mktemp -d)"
curl -fL https://registry.npmjs.org/@modelcontextprotocol/ext-apps/-/ext-apps-2.0.4.tgz -o "$tmp/ext-apps.tgz"
echo "d12699dc721fe6444fab3df6efd9d888a485a44484ffbee3ef07c3caf82fa2fd  $tmp/ext-apps.tgz" | sha256sum -c -
tar -xzf "$tmp/ext-apps.tgz" -C "$tmp"
mkdir -p "$fixture/generated"
cp "$tmp/package/dist/src/app-with-deps.js" "$fixture/generated/ext-apps-2.0.4.mjs"
cp "$tmp/package/LICENSE" "$fixture/generated/LICENSE.ext-apps"
for entry in client:2.0.0 core:2.0.0 zod:4.6.6 content-type:1.0.5; do
  name="${entry%:*}"; version="${entry#*:}"; package="$name"
  case "$name" in client|core) package="@modelcontextprotocol/$name" ;; esac
  curl -fL "https://registry.npmjs.org/$package/-/$name-$version.tgz" -o "$tmp/$name.tgz"
  tar -xOf "$tmp/$name.tgz" package/LICENSE > "$fixture/generated/LICENSE.$name"
done
echo "d24b6ae50dbc989539db8287e217f41f0833f2b0a81ac4c6dbde444715ef5cd0  $fixture/generated/ext-apps-2.0.4.mjs" | sha256sum -c -
```

## matrix-widget-api（widget 页）

`widget.mjs` 用 `generated/matrix-widget-api-1.20.0.min.js` 当 widget 页，它是 npm 官方
[`matrix-widget-api@1.20.0`](https://www.npmjs.com/package/matrix-widget-api/v/1.20.0) 的 `package/dist/api.min.js`，
逐字节保留，只改文件名。它是 UMD 打包、没有外部导入，载入后设全局 `mxwidgets`。包声明 Apache-2.0，`generated/LICENSE.matrix-widget-api`
是随包 LICENSE；打包进去的 `events@3.3.0`（MIT）的 LICENSE 在 `generated/LICENSE.events`。只在测试里跑，不进任何二进制。

SHA-256：

- npm tarball：`f46a20c0b39b7c60df7d15fa5e2af84edb28f6abfd590bb0796a279f3628587b`
- `.min.js`：`6a66356cede9b91b7cc8fe02089758d9cc6ac07b12f6b6ca3f67245a2c7eb7fb`
- `events-3.3.0.tgz`：`2c30dd630b58299bdfaac8688f763c6f312d29779c500b1e3701a1d4fb3b534b`

```sh
fixture="$PWD/crates/wonton-client/tests/it/tabs_bridge/fixtures"
tmp="$(mktemp -d)"
curl -fL https://registry.npmjs.org/matrix-widget-api/-/matrix-widget-api-1.20.0.tgz -o "$tmp/mwa.tgz"
echo "f46a20c0b39b7c60df7d15fa5e2af84edb28f6abfd590bb0796a279f3628587b  $tmp/mwa.tgz" | sha256sum -c -
tar -xzf "$tmp/mwa.tgz" -C "$tmp"
cp "$tmp/package/dist/api.min.js" "$fixture/generated/matrix-widget-api-1.20.0.min.js"
cp "$tmp/package/LICENSE" "$fixture/generated/LICENSE.matrix-widget-api"
curl -fL https://registry.npmjs.org/events/-/events-3.3.0.tgz -o "$tmp/events.tgz"
echo "2c30dd630b58299bdfaac8688f763c6f312d29779c500b1e3701a1d4fb3b534b  $tmp/events.tgz" | sha256sum -c -
tar -xOf "$tmp/events.tgz" package/LICENSE > "$fixture/generated/LICENSE.events"
echo "6a66356cede9b91b7cc8fe02089758d9cc6ac07b12f6b6ca3f67245a2c7eb7fb  $fixture/generated/matrix-widget-api-1.20.0.min.js" | sha256sum -c -
```

`scripts/check-file-size.sh` 的 `:(exclude)**/generated/**` 已排除生成文件，无需 `.size-allowlist`。

## Rust 驱动协议

用官方 `node:22` 运行 `node sdk.mjs`；stdin/stdout 每行一个 JSON，日志只写 stderr。
最小页面替身只存在测试中：`window.parent === window`，页面 `postMessage` 异步派发，
来源为该 window，origin 取自首行输入的页面 URL；原生注入的 MessageEvent 默认 origin 是空字符串。
根元素在文档开始脚本执行时不存在，随后插入并触发 MutationObserver、DOMContentLoaded。
SDK 使用真实 App 类，`autoResize: false`。

Rust 驱动默认使用 `node:22@sha256:18a6d90fd60542269850c80ca03b29b1d9b14d6b221ac7ee8e35b0d33ef904bd`，
容器禁用网络、只读挂载夹具。容器内的标准 shell 等 `node` 真正退出后，输出
`{"sdkExitCode":<退出码>}`，再以相同状态退出。收到 `closed` 后，驱动在原 10 秒内
检查这条状态为 0；本地 Node 路径直接检查子进程退出。
`docker run` 的返回还要等 Docker 处理容器退出，因此单独等待并检查 CLI 成功，
随后同步执行不带 `-f` 的 `docker rm` 并检查清理成功，不把 Docker 收尾算作 SDK 退出。
测试失败时驱动只清理自己的容器。
开发机不启动容器时可显式设 `WONTON_TABS_NODE=/绝对路径/node`，同一条测试改用该二进制；
这不是 Docker 不可用时的自动降级，CI 不设置它。

1. 首行输入 `{ "script": "<文档开始 JS>", "platform": "ios", "url": "http://app.feed.wonton.localhost/feed" }`（也支持 `android`）。
2. 原生通道输出 `{ "message": "<JSON-RPC 字符串>", "origin": "<页面来源>" }`。Rust 将来源与消息一起交给真实处理器，再输入
   `{ "script": "<回复 JS>" }`；脚本注入不单独应答，也可用于主题、重选、连接事件。
3. 握手完成输出 `{ "ready": true, "context": <getHostContext()>, "capabilities": <getHostCapabilities()> }`。
   `initialized` 的原生消息先于 ready。
4. 串行发送 `{ "id": 1, "command": "openLink", "params": { "url": "https://example.com" } }`，
   `command` 还支持 `sendMessage`、`requestDisplayMode`、`sendSizeChanged`；params 原样交给 SDK。
   等待结果期间继续处理原生消息并注入回复，不要等命令结束才回包。成功输出 `{ "id": 1, "result": ... }`。
   预期 JSON-RPC 错误时加 `"expectError": true`，输出 `{ "id": 1, "error": { "code": ..., "message": ... } }`；
   意外成功、意外拒绝、SDK 错误、协议断言失败均非零退出。
5. `{ "id": 2, "command": "inspect" }` 返回 id、context、capabilities、css（根 style 键值）、
   events（累计 `{type,detail}`）、contextChanges（SDK 回调收到的累计更新）。先注入再 inspect 可验证通知效果。
6. `{ "id": 3, "command": "exit" }` 关闭 SDK，输出 `{ "id": 3, "closed": true }` 并退出 0。
   未发 exit 就关闭 stdin 视为驱动协议错误。Rust 驱动负责超时和行为断言。

禁止把这个最小 DOM 替身用于生产；它不模拟布局或浏览器安全策略，也不替代真机 WebView 验证。
