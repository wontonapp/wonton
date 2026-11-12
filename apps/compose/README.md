# Compose 客户端

Kotlin Multiplatform 工程：共享 UI 模块、Android 应用，以及桌面端（M2，Compose Multiplatform）。
状态和业务逻辑都在 `crates/wonton-client`，本目录只负责渲染 ViewModel、把用户操作发回核心，
以及调用平台能力。

## 结构

- `shared/`：共享 UI 模块（KMP 库）。`buildRustCore` 任务用 cargo-ndk 编译 Rust 核心，并生成
  UniFFI 的 Kotlin 绑定，`.so` 和绑定都不提交。
- `androidApp/`：Android 应用，只有 `Application`（持有唯一的核心实例）和 `Activity`，以及把下载好的文件交给系统打开的
  `FileProvider`（ADR-0020）。
- `scripts/build-core.sh`：被 `buildRustCore` 调用，一般不需要手动运行。

UniFFI 的 Kotlin 绑定依赖 JNA，只能编译到 JVM 类目标，不能放进 `commonMain`。所以绑定和界面暂时
放在 `shared/src/androidMain`；M2 加桌面端时，新增 JVM 目标，把它们移到 Android 与桌面共用的
源集。

## 构建

需要 JDK 21、Android SDK（platform 37 和 NDK）、Rust 的 Android target 和 cargo-ndk：

```bash
rustup target add aarch64-linux-android
```

```bash
cargo install cargo-ndk --locked
```

SDK 位置写在 `apps/compose/local.properties`（`sdk.dir=...`，不提交）或环境变量 `ANDROID_HOME`。
在 `apps/compose` 下构建 Debug APK：

```bash
./gradlew :androidApp:assembleDebug
```

默认只编译 `arm64-v8a`（真机和 Apple 芯片上的模拟器）。要别的 ABI 或 release 构建，用
`-Pwonton.abis="arm64-v8a x86_64"`、`-Pwonton.rustProfile=release`。

## 连家里的服务器（开发）

起本地核心后（`deploy/dev/core/dev.sh up`），还没配对时“应用”页就是配对页：扫局域网状态页上或 `dev.sh pair` 打出的
二维码（CameraX 取景、ZXing 识别），也可以粘贴码的文字。配上以后设置的“家里的服务器”里的“添加设备”向核心要一个码，用 ZXing
画成二维码给新设备扫。票据的直连地址必须能从模拟器到达。Linux 的 headless 模拟器可用核心容器的 Docker
网络地址和隧道端口；`10.0.2.2` 经宿主端口映射时可能改变 UDP 回包源地址，导致握手超时。经 intent 把码传给 Debug 构建：

```bash
direct_addr=192.0.2.10:18482 # 换成模拟器可达的核心直连地址
adb shell "am start -n app.wonton.android/.MainActivity --es wonton_pairing_code '$(deploy/dev/core/dev.sh pair "$direct_addr" | tail -n 1)'"
```

- 配上以后连接信息存在 App 的数据目录里，之后直接启动也会自动重连，不用再传码；配对码只能用一次。
- `--es wonton_open_web <插件网页地址>`（比如装好 Grafana 示例插件后的 `http://app-grafana.wonton.localhost:18480/`）
  选择已安装插件中地址匹配的页面，不把任意 URL 当成插件。加载结果在 logcat 的 `WontonWeb` 标签下。
- 核心日志写在 App 的 `files/core.log`（`--es wonton_log` 改过滤规则）：
  `adb shell run-as app.wonton.android cat files/core.log`。
- WebView 的代理由 androidx.webkit 的 `ProxyController` 设置，对整个 App 生效。

设置首页订阅客户端核心的 `SettingsViewModel`，按 `sections` 的分组和顺序画列表；明暗、重点色、字号和字体收在“外观”二级页，“通知”页列出系统通知、推送状态和类别，点击进入对应的系统通知设置；“关于”只显示 App 版本和构建号（ADR-0028）。

“应用”页的商店入口独立放在已装插件网格上方，有更新时显示数量，未安装插件时保留“去应用商店”空状态按钮。底栏根页不显示大标题，连接状态仍在顶栏显示，状态栏后是不透明的 `canvas`。

## 新用户引导

配对后的五步引导遵循 [ADR-0034](../../docs/adr/0034-onboarding-and-help.md)：资料、Agent、模型、应用、确认。
每一步只在客户端核心的内存里记下选择，最后一页列出权限和说明，点“完成”才保存资料、模型、默认 Agent 并在后台安装应用。
每项显示进展和失败重试；保存完成即可进入聊天，不等待镜像下载。核心没有给出默认 Agent 的聊天身份时留在聊天列表。
首次状态加载失败会显示整屏错误，可重试或只在这次运行里先跳过；设置里的“重看引导”会重新打开完整流程。
验证首次体验使用独立开发数据目录；重看时显示服务器上现有设置，不删除既有应用或模型配置。

名字必填、邮箱选填，字段错误由核心给出。模型页可用已接好的模型、测试服务商 key、测试自定义 OpenAI 兼容地址，或选 Agent 的账号登录。
测试不保存配置；key 不进入 Android 的可恢复状态，不提供自动填充保存或输入法个性化学习，发出测试操作后清空输入框。
测试前 key 只在壳里，因此模型页主按钮在核心允许继续，或展开表单已填 key（自定义还需地址和模型名）且未测试时可点；有 key 时先测试再前进。
“更多服务商”在同页展开核心给出的其余服务商；列表为空时才提供网关网页入口，经已有 WebView 的隧道代理打开。
取 key 的厂商链接交给系统浏览器。

## 插件标签页

固定页和未固定的页面共用 WebView 容器。只在首次打开时创建；切换底栏保留网页历史和滚动位置，重选回到顶部。
页面可从详情页、应用图标长按菜单和安装完成页固定；设置的“底栏”页只列当前项，可拖动排序、移除，“添加更多”进入使用插件自身图标的候选页。“显示文字”开关可让底栏只显示图标，长按底栏也能进入设置。
移除设置后，入口在“应用”页右上角。满栏时必须选替换项，不自动挤掉页面。
顶栏展开按钮和底栏长按菜单可进入全屏；胶囊提供退出、重新加载、“总是全屏”和插件详情。
切换模式保留同一个 WebView；退出目标由核心决定。网页视频全屏也放在胶囊下面，返回先退出视频再退网页历史。
`inset` 页的网页框留在系统安全区和胶囊条内；`edge-to-edge` 页用核心注入的四边 `--wonton-inset-*` 留白。
留白处按 ADR-0033 拉伸网页框内贴边的 1px 像素，不使用 `theme-color`：页面在前台可见时每秒通过窗口
`PixelCopy` 取样，失败保留上次图像，首样本前用 `canvas`。只补有留白的边，另取系统栏所在边用于图标配色；
带栏只补网页底边，键盘已占的底部不重复留白。`inset` 的取样行在胶囊、短状态和确认横幅下方；
`edge-to-edge` 顶边被确认横幅盖住时跳过该边。
状态栏隐藏，导航栏透明可见；全屏及未固定页的导航图标按导航栏所在边的亮度选择（三键横屏可在侧边），
带栏按 App 主题选择。
胶囊背景高 32dp，两个按钮的命中区各 48dp；顶部挖孔安全区至少 56dp、与胶囊横向不重叠且没有短状态时，
胶囊才放在该安全区内居中，否则位于安全区下方 8dp。

停止页的“启动”只发送核心操作，失败显示核心原因。明暗、字号与 WebView 生命周期的实测边界见
[ADR-0033 的 Android 验证](../../docs/adr/0033-plugin-tabs.md#android-标签页容器)。

声明跨源隔离的插件遵循 [ADR-0033 的跨源隔离规则](../../docs/adr/0033-plugin-tabs.md#跨源隔离)。
Android 按 WebView 能力检测配置默认 Profile 的精确来源允许列表；列表在已装插件更新时替换。
已装列表晚到时不额外阻止已有地址的页面加载，允许列表更新也作用于已加载的页面。
不支持时，底栏和覆盖层都先显示状态页，折叠详情列出实际 WebView 版本和所需版本。
“仍然打开”只在本次 App 进程里对该插件生效；不替网页开启降级模式、不持久保存选择。

## App 链接和桌面快捷方式

链接格式遵循 [ADR-0041](../../docs/adr/0041-app-links-and-shortcuts.md)，只由核心 `appLinkUrl` / `parseAppLink`
编解码。`androidApp/.../MainActivity.kt` 接收 VIEW Intent，`shared/.../ui/AppNavigation.kt` 消费一次并打开页面。
未配对或仍在引导时忽略导航；已配对的冷启动等首次状态恢复后再决定。扫码、粘贴与 Debug 配对仍交给 `PairHome`。
未固定的设置页沿用原生覆盖层，不自动修改底栏（核心 `SelectTab` 只接受已固定的原生页）。

`shared/.../ui/AppShortcuts.kt` 用 AndroidX Core（Apache-2.0）发布“新会话”“问 Bot”和固定的插件页；固定入口图标
复用设计系统，插件图标经核心下载后转 `IconCompat`。总数不超过系统上限：两个固定入口优先，插件按底栏顺序显示。
图标获取和系统更新失败会提示原因。卸载后同时停用动态项和已固定到桌面的项，显示“这个应用已卸载”；只移出底栏不禁用
桌面项，重装后重新启用。实际菜单容量由桌面决定，`adb shell dumpsys shortcut` 可查看动态、固定和停用状态。

## 聊天（开发）

“聊天”页登录主人的 Matrix 账号，家里的核心要接好 Tuwunel（`deploy/dev/core` 自带）；登录、本地存储和恢复密钥见
[ADR-0005](../../docs/adr/0005-native-clients-thick-core.md) 的“App 内聊天的核心接口”。要有个能聊的对象，装上
[回声 Bot 示例插件](../../plugins/examples/echo-bot/README.md)。

Debug 构建默认编译 debug 的 Rust 核心，加上 matrix-sdk 后 `.so` 将近 1 GB，装进模拟器很慢，编译产物也很占磁盘；只看
界面时用 `-Pwonton.rustProfile=release`。

聊天列表右下角的 Material 3 加号按钮进入“发起私聊”，位于底栏上方；列表底部留出按钮空间，最后一行可以滚动到按钮上方。

## 检查

```bash
./gradlew detekt
```

规则在 `config/detekt.yml`：函数超过 100 行即失败。detekt 没有文件行数规则，文件 ≤ 800 行由仓库的
`scripts/check-file-size.sh` 检查。CI 见 `.github/workflows/android.yml`。

包名 `app.wonton.*` 是临时的，上架前确定正式的。

## 推送

App 被杀或锁屏时，新消息经 UnifiedPush 分发器（如 ntfy）弹出系统通知，见 ADR-0003 的“设计：加密推送”。启动时先申请
通知权限，再向分发器登记，手机上要装一个分发器；推送的接收、解密和通知在 `androidApp` 的 `push` 包里。
