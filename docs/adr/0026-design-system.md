# ADR-0026 App 的外观：两端统一自绘、黑白加可换的重点色、Phosphor 图标

- 状态：已接受（2026-11-03）。“两端一模一样”这个前提已被 [ADR-0029](0029-platform-material.md) 改成令牌共用、栏和菜单按平台；被取代的段落就地标出

## 背景

按 `4cebb64` 在模拟器上看到的现状：Android 全 App 没有图标和深色模式，根主题是 Material 默认紫；iOS 是系统蓝和 27 个 SF Symbols；两端都没有 App 图标和品牌色，插件占位图一个是灰底符号，一个是灰底首字。颜色和间距直接写在屏幕代码里，两端各写各的。

主人 2026-11-03 看过方案页后定了五件事：

1. 两端一模一样，走 Telegram 的路子，工作量大也认。
2. 黑白为主，重点色有纸质磨砂的质感。默认 `#FEEBEA`，用户可以换，每台设备各自设置。
3. 图标统一用 Phosphor。
4. App 图标是甲骨文“云”，墨色字配米白底。
5. 插件图标用上游官方标志，原版不滥用。

本篇不改 [ADR-0005](0005-native-clients-thick-core.md)：壳还是 SwiftUI 和 Compose，自绘的是控件的样子，不是换渲染层。

## 决策

### 什么自己画，什么用系统的

> 下面两条已被 [ADR-0029](0029-platform-material.md) 取代：导航栏、底栏、输入栏、长按菜单和弹窗的底按平台走，圆角可以分平台。

- **自己画：** 导航栏、底栏、按钮、输入框、列表行、开关、弹窗和底部面板、提示条、气泡、卡片。两端的尺寸、圆角、颜色和动效取同一组值。
- **用系统的：** 返回手势、分享面板、相册和文件选择、相机、键盘和输入法、文字选择、状态栏、通知、触感、读屏、系统字号。

返回到进来的地方：从应用页或应用商店打开和 Bot 的聊天，返回回到那一页，不到聊天列表。

**自己画的是样式，语义用系统原语。** SwiftUI 用 `ButtonStyle`、`ToggleStyle`、`.textFieldStyle(.plain)` 这类自定义样式；Compose 用 foundation 层的 `clickable`、`selectable`、`toggleable` 和 `BasicTextField`，不直接用 `pointerInput` 写控件。每个组件都要有：

- 读屏的角色、名字和状态值：选中和开关要读得出来（`.isSelected`、`accessibilityValue`；`selected`、`toggleableState`、`stateDescription`）。
- 系统字号放大后不截断。iOS 的底栏提供 Large Content Viewer。
- 响应“粗体文本”和“增强对比度”。
- 48pt/48dp 的命中区，按下和禁用状态。
- 桌面端的悬停、键盘焦点和焦点环。

布局一律用 leading/trailing、start/end。是否支持从右到左的语言以后再定。

**字体只用系统自带的，不打包，也不单独带拉丁字体**（用户可以在系统字体里选，见 [ADR-0028](0028-settings-page.md)）。 这是“一模一样”有意保留的例外：英文和数字在 iOS 上是 SF、在 Android 上是 Roboto，Android 各厂商的中文字体也不同；非默认字号下，两端系统的放大曲线也不同，换行和截断的位置可能不一样。

### 颜色

灰阶不带色相，只有这一份，由核心输出给两端：

| 令牌 | 浅色 | 深色 |
| --- | --- | --- |
| `canvas` | `#FFFFFF` | `#0E0E0E` |
| `surface` | `#FFFFFF` | `#191919` |
| `surfaceSubtle` | `#F4F4F4` | `#262626` |
| `textPrimary` | `#111111` | `#F5F5F5` |
| `textSecondary` | `#5E5E5E` | `#ABABAB` |
| `separator` | `#E4E4E4` | `#333333` |
| `primary` | `#111111` | `#F5F5F5` |
| `onPrimary` | `#FFFFFF` | `#111111` |

主按钮用 `primary`。错误和警告用独立的状态色：`success` `#18753C`/`#7CDCA0`，`warning` `#8A4B00`/`#FFC379`，`danger` `#B42318`/`#FFA69F`。

重点色只用在选中、自己的气泡、占位图和角标上，分两层，由一个颜色推导。“页面”指 `canvas`、`surface`、`surfaceSubtle` 三种底色：

| 令牌 | 用在 | 浅色 | 深色 |
| --- | --- | --- | --- |
| `accentFill` | 填充和底色 | 用户选的颜色，不改 | 保持色相，亮度统一到一档固定的暗值，原色更暗也提亮 |
| `accentInk` | 页面上的字和图标 | 用户的颜色对页面都 ≥ 5.5:1 时原样用，否则保持色相往深找 | 同样的规则，往浅找 |
| `accentInkOnFill` | 填充上的图标和小标签 | `accentInk`；它在填充上不到 4.5:1 时取 `textOnAccentFill` | 同左 |
| `textOnAccentFill` | 填充上的正文 | `textPrimary`；不到 4.5:1 时取黑或白里对比度高的 | 同左 |
| `accentFillBorder` | 填充的描边 | 填充对任一页面底色不到 1.5:1 时才有：保持色相，找对页面都 ≥ 1.5:1 的一档 | 同左，浅色和深色分别计算 |

推导在 OKLCH 里做，超出 sRGB 时只降彩度。很浅的颜色彩度太低，直接压暗会变成灰色，所以推导出的颜色有最低彩度；彩度接近零的颜色按比例收向灰，所以接近白色的颜色不会一个推出灰、一个推出明显带色。满足对比度的颜色原样保留，这条阈值两侧的结果会不连续（`#6E5E5F` 的 `accentInk` 是它自己，`#6F5F60` 的是 `#984651`），这是有意的：用户选的颜色能用就不改。常量在 Rust 里。

默认 `#FEEBEA` 的结果：

| 令牌 | 浅色 | 深色 |
| --- | --- | --- |
| `accentFill` | `#FEEBEA` | `#402929` |
| `accentInk` | `#9B494A`（对 `#F4F4F4` 5.54:1） | `#FEEBEA` |
| `accentInkOnFill` | `#9B494A`（5.31:1） | `#FEEBEA` |
| `textOnAccentFill` | `#111111` | `#F5F5F5` |
| `accentFillBorder` | `#FFB7B4` | `#76282B` |

**选中不能只靠 `accentFill`。** 默认的填充对白色画布只有 1.15:1，低视力用户看不出来。选中时至少再换一样：字或图标换成 `accentInkOnFill`、图标换成 Fill，或者加选中标记。

用户能选任意 RGB，所以测试按颜色网格加接近灰色和白色的边界颜色，断言浅色和深色下每个令牌都满足上面的对比度。

### 状态在核心

颜色由 Rust 算，壳只渲染：

- `AppearanceViewModel` 和相关的 `Action` 定义在 `wonton-protocol`，先单独提只改接口的 PR，由 `wonton-client` 实现。它带用户选的颜色和浅、深两套完整的颜色令牌，壳按明暗选一套。
- 设置页的样子和可选的项见 [ADR-0028](0028-settings-page.md)。
- 设置存在 App 私有数据目录下的单独文件里，和设备密钥、连接信息同一目录、不同文件，写坏了不连累连接。每台设备各自设置，不跨设备同步。
- 设置文件读不了时，外观用默认值，`AppearanceViewModel` 带上原因，设置页如实显示。这里不让 App 打不开：外观坏了不影响连家，而用户需要进到 App 才能重设。
- 核心起不来时启动失败页也要有颜色：核心导出不依赖 `ClientCore` 的 `default_appearance()`。
- 十六进制的解析和格式化也在核心（`parse_accent()`、`accent_hex()`），壳不各写一份。

### 纸质磨砂

> “叠在底栏上”已被 [ADR-0029](0029-platform-material.md) 取代：栏上不再叠噪点。

一张 128×128 的噪点小图（黑白颗粒各半，透明度不超过 7%），两端用同一个文件，平铺叠在重点色填充和底栏上。1 个噪点像素对 1 个物理像素：Compose 用 `ImageShader` 配 `TileMode.Repeated`，SwiftUI 的平铺缩放取 `displayScale`。按 pt 或 dp 平铺时，颗粒会随屏幕密度放大变糊，两端也不一样。

对比度按不叠噪点的颜色算。黑白颗粒的影响大体抵消，但亮度是非线性的，平均对比度仍会降一点。按这张图的颗粒分布取平均亮度：默认色上 `accentInkOnFill` 从 5.31 降到约 5.1，随机 2 万个颜色的填充上最低约 4.3。不按最坏情况算，因为中间亮度的填充上黑白里最好的一个也只有约 4.58:1，留不出余量。

> 下面这段已被 [ADR-0029](0029-platform-material.md) 取代：iOS 用系统材质，栏上不再叠噪点。

不做实时模糊。`UIVisualEffectView` 和 Android 的 `RenderEffect` 两端效果不同，Android 12 以下没有，滚动时还要逐帧计算。底栏用不透明度很高的表面加噪点，看起来是磨砂，代价只是一次平铺贴图。

### 其他令牌

间距、圆角、字号、尺寸、动效和手势阈值是固定值，写在 `design/tokens.toml`，脚本生成 Swift 和 Kotlin 两份常量，生成结果提交进仓库，CI 重新生成并比对。两端名字对齐：`WontonTheme.spacing.s4`、`WontonTheme.radii.card`、`WontonTheme.typography.body`、`WontonTheme.motion.state`、`WontonTheme.gesture.pullDistance`。屏幕代码不再写字面量。

取值只写在 `design/tokens.toml`（[ADR-0029](0029-platform-material.md)）。字号默认跟随系统，用户也可以选固定的档（[ADR-0028](0028-settings-page.md)）。系统“减少动态”打开时一律改成不超过 100ms 的淡入淡出。动画结束不代表业务完成，状态只听核心的。

手势距离用 pt/dp，速度用 pt/dp 每秒，壳只负责转换为平台坐标。慢拖达到完整距离，或实际距离达到快甩的最小距离且松手速度达标，才改变开合状态；不能只靠速度投影把一次轻碰算作拉开。取消手势回到起始状态。聊天面板和侧栏共用这一规则，面板级别见 [ADR-0052](0052-chat-pull-panel.md)。

### 图标：Phosphor

MIT，每个图标都有线框和实心。未选中用 Regular，选中用 Fill。

不引入运行时依赖。仓库放一份白名单 `design/icons.toml`，写明语义名、Phosphor 名、上游版本和 `mirror`（返回箭头这类带方向的图标），以及上游原始 SVG 和 LICENSE。脚本从 SVG 生成，每个图标一个文件，生成结果提交进仓库，CI 重新生成并比对：

- iOS：Xcode 的 SVG 资源（模板渲染，保留矢量，`mirror` 对应资源的方向属性）和 `WontonIcon` 枚举。
- Compose：`ImageVector` 和 `WontonIcon` 对象，Android 和桌面共用，`mirror` 对应 `autoMirror`。转换用 [Valkyrie](https://github.com/ComposeGears/Valkyrie) 的命令行工具（Apache-2.0，固定版本，只在生成时用，不进 App）：它输出的就是一个图标一个文件的 `ImageVector`（1.2.1 验证过，`--auto-mirror` 按次运行，带方向和不带方向的各跑一次）。没选 svg-to-compose：2024 年后没有更新。

没选的方案（2026-11 核对，Phosphor core 2.1.1）：

- **两端各接社区或官方包：** iOS 有官方包，Compose 只有社区包，两边的图形版本对不上，还会把整库带进来。
- **Lucide：** 更新勤，但没有实心，选中只能靠颜色。
- **Tabler：** 只有约两成有实心。
- **Material Symbols：** 观感太 Android，和两端统一的自有风格不合。

Phosphor 没有专门的服务器图形，用 `hard-drives`。App 内的开源许可页列出 Phosphor 的版本和许可证。

### 占位图、插件图标和 App 图标

- **插件占位图：** 两端同一种，`accentFill` 加噪点，上面是同一个 Phosphor Fill 图形。不用首字，也不按名字配色。
- **插件图标：** 用上游官方标志，规则只写在 [plugin-spec](../plugin-spec.md) 的“图标用什么”一节。核心保留透明通道，图标的透明部分由 App 垫白，否则透明底的黑色标志在深色模式下看不见。主人一度想按 Wonton 的风格全部重画，看过样例后改了：飞牛也用原标志，原版不滥用就行。
- **App 图标：** 甲骨文“云”，墨色字配米白底，字形是公有领域的矢量稿，出处见 [design/app-icon](../../design/app-icon/README.md)。主人否掉了早先极简线条勾出云吞的方向。定稿是矢量 SVG，由脚本导出两端需要的尺寸，包括 iOS 的深色和着色版本、Android 自适应图标的 monochrome 图层。图像模型只用来出方向，不用它的位图做定稿。

## 理由

- **两端统一（范围已被 ADR-0029 和 ADR-0107 缩小）：** 品牌一致，用户换手机看到的是同一个产品；内容层仍共用一套视觉，平台导航与控制层可以按系统呈现。Telegram 证明统一的内容层样式能做好。代价写在下面，主人知道。
- **颜色放核心：** [ADR-0005](0005-native-clients-thick-core.md) 要求状态和逻辑在 Rust。推导算法写两遍迟早不一致，用户在两台设备上选同一个颜色时结果也应该相同。
- **令牌用生成而不是手写两份：** 图标反正要生成，顺带生成令牌只多几十行，换来两端的值不会悄悄分叉。

## 代价

- 工作量比原生骨架大得多，每个控件都要在两端写一遍，并自己做读屏和大字号。
- 系统升级带来的新外观（如 iOS 的玻璃效果）由系统容器和 ADR-0107 的 API 自动跟上；自绘功能层仍需按 iOS 版本选择 Liquid Glass 或 `Material`，并自己验证交互与可读性。
- iOS 隐藏系统导航栏后，SwiftUI 默认会失去返回手势。`UINavigationController` 的扩展把手势的 delegate 指回自己，有上一页就放行（iOS 26.5 模拟器上从屏幕边缘滑动验证过）；每个带导航的屏都要验证。
- 重点色在核心里推导，每次切换要经过一次 ViewModel，比本地常量多一层。切换是低频操作，不算问题。

## 待验证

- 噪点平铺在长列表里的滚动帧率，两端低端设备各测一次，结果写在这里。
- 噪点叠在高饱和和中间亮度的填充上时文字的实际可读性，这是对比度余量最小的情况，真机截图看一次。
