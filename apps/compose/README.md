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

### 版本号

版本号由仓库根的 `scripts/app-version.sh` 一处计算（iOS 用同一条规则，见 `apps/ios/README.md` 的「构建」）：
优先环境变量 `WONTON_VERSION`（`X.Y.Z`，可带 `v` 前缀），其次最近的 `vX.Y.Z` tag。**发版必须显式传版本**：
发版构建是在没有 git 元数据的副本上做的，取不到 tag，不传 Release 构建直接报错。

```bash
WONTON_VERSION=0.2.4 ./gradlew :androidApp:assembleRelease
```

Gradle 也认 `-PwontonVersion=0.2.4`（透传给同一脚本；远程构建通道只透传它）。在自己带 tag 的检出上不传时
取最近的发版 tag；都拿不到时 Debug 构建用 `0.0.0 (0)`。构建号由版本号算出（`major*1000000 + minor*1000 +
patch`），覆盖安装要求它不降。

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

插件安装确认页按预览选择可选权限，安装默认关闭、升级保留预览选择；提交最高风险权限需单独确认主机控制权。
详情页开关以服务器返回的状态为准，打开最高风险权限两步确认、关闭一次确认；显示打开者和日期，失败可重试原目标。
应用图标上的主机权限锁标记独立于更新、故障标记，点锁打开详情（ADR-0057）。

“家里的服务器”顶部始终显示服务器列表和“添加服务器”，当前这台带勾，点另一台切换；长按一台可改名或移除。移除前确认只删手机上的聊天记录和文件，不影响服务器。添加时配对页左上角可取消，首次配对不显示取消（ADR-0042）。
服务器的默认名由界面按核心给的固定编号和 App 语言显示，主人自定义的名字保持原样；改名输入框用显示名预填，只有点击保存才提交该文字（ADR-0043）。

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
底栏里的带栏页保留标题栏：标题、能后退时的返回、接管按钮、重新加载、进入全屏，overlay 再有关闭。
标题栏和原生页静止时的顶栏一样是页面同色（`canvas`），不垫灰色实底；网页框顶边 = 标题栏下沿，
底边 = 浮动底栏上沿（overlay 没有底栏时到导航栏上沿），带栏时不显示胶囊。`edge-to-edge` 页带栏时
照旧铺到标题栏后面，标题栏垫不透明的 `canvas`。
全屏只有胶囊：自动隐藏，往下拉出现、往上推藏起、3 秒空闲淡出，藏起来时不挡点按；尾部按钮是“退出全屏”
（同心圆），“···”菜单有返回、接管、重新加载、“总是全屏”和插件详情。
切换模式保留同一个 WebView；退出目标由核心决定。网页视频全屏也放在胶囊下面，返回先退出视频再退网页历史。
全屏时竖屏状态栏透明可见（那一条是 App 的 `canvas` 底色），横屏隐藏；导航栏透明，系统栏图标按 App 主题
选深浅，不跟网页。全屏都不取网页贴边像素；`inset` 网页框只让出系统栏和横屏挖孔，`edge-to-edge` 仍铺满，
`--wonton-inset-top` 全屏时为状态栏下沿（横屏为 0），带栏时为标题栏下沿，`--wonton-inset-bottom`
为底栏加导航栏。
胶囊浮在页面上，不占网页布局：可见顶边为顶部安全区加 8dp，右距为右安全区加 8dp，高 32dp。
两个按钮命中区各 48×32dp，整行其他位置归网页。隐藏时不接触摸，也不出现在无障碍树里。
胶囊黑色底 20%、不模糊、无描边；20dp 白色 More / Leave 图标带黑色 40%、半径 1dp 的阴影，
中间 hairline 竖线为白色 20%。Android 12 以下图标阴影使用不模糊的黑色底层。
进入全屏、页面加载完和转屏时显示；3 秒未碰胶囊则用 200ms 淡出。点按、按住胶囊及打开菜单时重新计时，胶囊淡出不关闭菜单。
手指在网页同一方向连续拖过 24dp，向下显示、向上隐藏；旁听原生 WebView 的触摸分发，不消费事件，
不依赖页面是否滚动，因此内部滚动、单页和 canvas 规则相同。TalkBack 触摸探索开启时一直显示。
短状态、连接提示、确认卡片横幅和接管提示独立浮在页面上，不随胶囊隐藏。原生页（聊天、应用、设置及推进去的
页面）内容画到系统栏后面：静止时顶栏与页面同色，滚到顶栏后面时垫一层 85% 的 `canvas`；底栏是浮动的
半透明胶囊，列表用 contentPadding 留出底栏加导航栏的高度，最后一项能完整滚出来。

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

规则在 `config/detekt.yml`：函数超过 100 行即失败。detekt 没有文件行数规则，文件行数由仓库的
`scripts/check-file-size.sh` 检查（不算空行和注释，超过 800 行警告、超过 2000 行失败）。CI 见 `.github/workflows/android.yml`。

包名 `app.wonton.*` 是临时的，上架前确定正式的。

## 推送

App 被杀或锁屏时，新消息经 UnifiedPush 分发器（如 ntfy）弹出系统通知，见 ADR-0003 的“设计：加密推送”。启动时先申请
通知权限，再向分发器登记，手机上要装一个分发器；推送的接收、解密和通知在 `androidApp` 的 `push` 包里。
通知携带来源服务器和房间，点开非当前服务器的通知时先切换，等连接成功再打开房间；切换失败显示核心原因，不在旧服务器上打开同名房间。

消息通知可直接回复，只有不带密钥和设置表单的确认卡片给操作按钮；带表单或没有房间的卡片点开后在 App 处理。
动作交给 AndroidX WorkManager（Apache-2.0）持久化，一次点击只尝试一次；进程在执行途中结束时不自动重发，
通知提示先到 App 确认结果。服务器已移除、网络不可用或核心拒绝都会显示失败，原卡片保留。

锁屏默认显示内容，在通知设置中按消息、确认、插件通知分别关闭；关闭后锁屏只显示类别占位。
数字角标使用核心的未读总数，邀请按一条计；只有实际已读才变化，打开 App 不清零。启动器可能只显示圆点或
按通知条数计算，不保证展示绝对数字。Bot 卡片连续等待主人至少 60 秒才显示进行中通知，回答或过期后撤下；
进程重启后重新观察等待时间。Android 16 会申请 Live Update，是否提升由系统和设备厂商决定。
