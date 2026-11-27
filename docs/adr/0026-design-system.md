# ADR-0026 App 的外观：两端统一自绘、黑白、白名单生成的图标

- 状态：已接受（2026-11-03）。“两端一模一样”这个前提已被 [ADR-0029](0029-platform-material.md) 改成令牌共用、栏和菜单按平台；被取代的段落就地标出。2026-11-21 补充 Agent 身份色和 App 图标（[ADR-0126](0126-agent-list-bar-and-cloud.md)）；2026-11-26 起 iOS 的图标改用 Kubai 彩色分层，“图标：Phosphor”一节只对 Android 有效（见“图标：彩色分层用 Kubai”）；重点色已被 [ADR-0130](0130-black-and-white.md) 取代（2026-11-27）：界面只有黑白灰和状态色，被取代的段落就地标出

## 背景

按 `4cebb64` 在模拟器上看到的现状：Android 全 App 没有图标和深色模式，根主题是 Material 默认紫；iOS 是系统蓝和 27 个 SF Symbols；两端都没有 App 图标和品牌色，插件占位图一个是灰底符号，一个是灰底首字。颜色和间距直接写在屏幕代码里，两端各写各的。

主人 2026-11-03 看过方案页后定了五件事：

1. 两端一模一样，走 Telegram 的路子，工作量大也认。
2. 黑白为主，重点色有纸质磨砂的质感。默认 `#FEEBEA`，用户可以换，每台设备各自设置。
3. 图标统一用 Phosphor。
4. App 图标是甲骨文“云”，墨色字配米白底（后来换成主人原图的抠像加蓝白渐变底，见 [ADR-0126](0126-agent-list-bar-and-cloud.md)）。
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

> 取值和用法以 [ADR-0130](0130-black-and-white.md) 为准；下表是当时的灰阶，ADR-0130 沿用，只有深色的 `primary`、`onPrimary` 改成了 `#454545`、`#FFFFFF`。

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

> 重点色（`accentFill`、`accentInk`、`accentInkOnFill`、`textOnAccentFill`、`accentFillBorder`，默认 `#FEEBEA`，按 OKLCH 推导）已被 [ADR-0130](0130-black-and-white.md) 取代，原文删去：强调填充用 `primary`，强调字用 `textPrimary`。

### Agent 的身份色（2026-11-21 补充）

只有 Agent 的身份时刻用饱和天蓝，其余界面不变。这里定名字和规则，`design/tokens.toml` 的取值留给实现 PR。

- 令牌三个，由一只来源色推导：`agentIdentity`（填充）、`agentInk`（它上面的字和图标）、`agentBacking`（头像的圆底）。
  来源是角色身体带阴影的取样，约 `#BBCEDA`、偏灰；令牌取它提亮后的天蓝，不用取样值。
- 浅色：保持来源色相，往上提亮到既在 `canvas` 上看得见（≥ 1.5:1），又让 `agentInk` 在它上面 ≥ 4.5:1；彩度不低于 0.06，免得提亮后
  变灰。深色：同一色相落进固定的暗档，`agentInk` 取提亮的那一档。
- `agentBacking` 是默认身体色的圆底令牌。主人给 Bot 换了身体色以后，核心按同一规则（OKLCH 的 L 提亮、C 降一半）从那只 Bot 的身体色
  现算，不走令牌，见 [ADR-0126](0126-agent-list-bar-and-cloud.md#头像分层和素材库)。
- 只用在：出场的整屏渐变、“Agent”那一项的选中图标、Agent 和 Bot 的头像圆底。**不用在：**主按钮（仍是 `primary`）、聊天气泡、
  强调色 `primary` 的场合（选中胶囊、自己的气泡）、插件图标、未读角标、列表选中。
- 推导放核心的常量里，写颜色网格测试；键写在 `design/tokens.toml`，两端只读生成结果。iOS 的玻璃照 [ADR-0107](0107-ios-liquid-glass.md)。

### 状态在核心

颜色由 Rust 算，壳只渲染：

- `AppearanceViewModel` 和相关的 `Action` 定义在 `wonton-protocol`，先单独提只改接口的 PR，由 `wonton-client` 实现。它带浅、深两套完整的颜色令牌，壳按明暗选一套。
- 设置页的样子和可选的项见 [ADR-0028](0028-settings-page.md)。
- 设置存在 App 私有数据目录下的单独文件里，和设备密钥、连接信息同一目录、不同文件，写坏了不连累连接。每台设备各自设置，不跨设备同步。
- 设置文件读不了时，外观用默认值，`AppearanceViewModel` 带上原因，设置页如实显示。这里不让 App 打不开：外观坏了不影响连家，而用户需要进到 App 才能重设。
- 核心起不来时启动失败页也要有颜色：核心导出不依赖 `ClientCore` 的 `default_appearance()`。

### 纸质磨砂

> “叠在底栏上”已被 [ADR-0029](0029-platform-material.md) 取代：栏上不再叠噪点。

一张 128×128 的噪点小图（黑白颗粒各半，透明度不超过 7%），两端用同一个文件，平铺叠在强调色填充（[ADR-0130](0130-black-and-white.md) 起是 `primary`）和底栏上。1 个噪点像素对 1 个物理像素：Compose 用 `ImageShader` 配 `TileMode.Repeated`，SwiftUI 的平铺缩放取 `displayScale`。按 pt 或 dp 平铺时，颗粒会随屏幕密度放大变糊，两端也不一样。

> 按重点色算的对比度余量已被 [ADR-0130](0130-black-and-white.md) 取代，原文删去：`primary` 上的 `onPrimary` 浅色约 18.9:1、深色约 9.6:1，噪点的影响可以不计。

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

Phosphor 没有专门的服务器图形，用 `hard-drives`。App 内的开源许可页列出该平台随 App 分发的那套图标的版本和许可证：`scripts/generate-licenses.py` 从 `design/icons.toml` 的上游声明取，正文是上游的 LICENSE 原文。

### 图标：彩色分层用 Kubai

主人 2026-11-26 看过样例页后定换成 [Kubai Icons](https://icons.kubai.design/) 的彩色分层那一套（MIT），每个图标有自己的颜色，不统一成重点色；2026-11-27 看过 iOS 的实现后再定：iOS 上所有图标都用它，玻璃栏、顶栏、输入栏和列表行的箭头也是彩色，不留 Phosphor 单色。单色只是一项用户设置，用的还是 Kubai 的图形。iOS 先做；Android 仍是上一节的 Phosphor，换过去时把 Phosphor 整个删掉。出问题靠 `git revert`，App 内不留回退到 Phosphor 的开关。

- **白名单：** `design/icons.toml` 每个语义名多写一个 Kubai 名和一个颜色名，颜色取同一文件的 `[palette]`。iOS 的 `WontonIcon` 只含写了 Kubai 名的语义名。
- **上游是 0.x：** 版本和 npm tarball 的校验值钉在 `icons.toml`，用到的 SVG 原件和 LICENSE 放进 `design/kubai/`，逐个文件的 SHA-256 记在 `SHA256SUMS`，生成时核对。
- **换色在生成时做：** 用 `kubai-icons-static` 包（颜色已经展开成具体值，不含 CSS）。上游每张图的颜色都由一个主色调出来，脚本按同样的配比换成 `[palette]` 的颜色，再检查结果里只剩 CoreSVG 认得的元素和具体颜色（带 `color-mix()`、`var()` 的 SVG，Xcode 会画成纯黑）。
- **浅色、深色各一张：** `[palette]` 每个颜色写两个值，生成两张图放进同一个资源的通用和深色外观。浅色页面上取深一档，不然浅面在白底上发灰；深色页面上取亮一档，浅面里混的白再压暗两成，不然比旁边的字还亮。各自对那一边的页面底、卡片底和强调填充（浅色下是黑，深色下是 `#454545`，[ADR-0130](0130-black-and-white.md)）都要有 3:1，生成时检查。
- **单色版：** 同一张上游图再生成一份给模板渲染，颜色跟界面的前景色。模板渲染只看透明度：主色和深色描边画实，主色混白的面按深浅变淡，白色的纸面和字改成把下面挖空，投影和高光去掉。颜色跟所在位置的文字色（`foregroundStyle`），不单独取重点色。壳按环境值 `wontonIconMono` 选彩色还是单色，默认彩色。
- **选中态：** 彩色分层不分线框和实心，实心版的语义名指向同一张图。选中靠底色、字重或勾，不靠图标换形。
- **裸的加号、叉、勾**在分层套里没有，没有自己画。叉和勾用带圆圈的。加号、返回和展开箭头会放进圆钮或在列表里成排出现，带着圆底就是圆里套圆：`icons.toml` 里写 `bare`，生成时去掉上游图里闭合的底，只留线条并放大，线条用主色，没有投影和高光。

验证结果（2026-11-27，Kubai 0.15.0，Xcode 27 的模拟器 iOS 26.5）：换色后的 SVG 放进资源目录、保留矢量，渐变、投影和高光都画得出来。单色版只在 macOS 上用 AppKit 渲染看过，`ban` 的斜杠（蒙版里套裁剪路径）没画出来。没验证：真机、最大辅助字号、“降低透明度”、单色版在 iOS 上的样子。

### 占位图、插件图标、App 图标和启动屏

- **插件占位图：** 两端同一种，`primary` 加噪点（ADR-0130），上面是同一个 Phosphor Fill 图形。不用首字，也不按名字配色。
- **插件图标：** 用上游官方标志，规则只写在 [plugin-spec](../plugin-spec.md) 的“图标用什么”一节。核心保留透明通道，图标的透明部分由 App 垫白，否则透明底的黑色标志在深色模式下看不见。主人一度想按 Wonton 的风格全部重画，看过样例后改了：飞牛也用原标志，原版不滥用就行。
- **App 图标和启动屏：** 用主人原图抠出的那一版（`design/app-icon/mascot/cutout.png`），形状、角度、神态都不改、也不重画，保住云形、浅天蓝、单边独角三个识别点。App 图标把角色居中放在蓝到白的渐变底上，出 iOS 的浅色、深色、着色和 Android 自适应图标的前景、背景、monochrome 图层；启动屏只放角色本身，底色是米白和深色。主人 2026-11-21 看过角色后定了换掉原来的甲骨文“云”，也否掉了早先极简线条勾出云吞的方向；角色来自主人原图，不是图像模型生成的位图。来源和商标归属写进 [design/app-icon](../../design/app-icon/README.md)。

## 理由

- **两端统一（范围已被 ADR-0029 和 ADR-0107 缩小）：** 品牌一致，用户换手机看到的是同一个产品；内容层仍共用一套视觉，平台导航与控制层可以按系统呈现。Telegram 证明统一的内容层样式能做好。代价写在下面，主人知道。
- **颜色放核心：** [ADR-0005](0005-native-clients-thick-core.md) 要求状态和逻辑在 Rust。颜色只写一份，两端和插件网页取的是同一套。
- **令牌用生成而不是手写两份：** 图标反正要生成，顺带生成令牌只多几十行，换来两端的值不会悄悄分叉。

## 代价

- 工作量比原生骨架大得多，每个控件都要在两端写一遍，并自己做读屏和大字号。
- 系统升级带来的新外观（如 iOS 的玻璃效果）由系统容器和 ADR-0107 的 API 自动跟上；自绘功能层仍需按 iOS 版本选择 Liquid Glass 或 `Material`，并自己验证交互与可读性。
- iOS 隐藏系统导航栏后，SwiftUI 默认会失去返回手势，还会把系统的返回识别器整个关掉；它本身也只认贴边那几点，手指自然落点收不到。`UINavigationController` 的扩展在左边 24 点自装一条平移手势，按拖动进度驱动返回转场；系统那条一直关着，SwiftUI 换页时重开就再关回去。返回只在有上一页、从边缘里侧起手、没有别的转场时开始；开始后同一根手指上页面里的手势（按钮、行、滚动）一律取消。UIKit 只在 delegate 同时给了自定义转场动画时才用交互控制器，系统默认的返回动画拖不住，所以拖动时的返回动画是自己画的（`EdgePopAnimator`，照系统的视差、压暗和阴影），完成还是弹回按松手的位置和速度定；拖的是上面一页的快照，拖动的那一会儿它是静止画面（直接挪视图的话，弹回后 SwiftUI 辅助功能里的位置停在屏幕外）；返回键和 iOS 26 整片内容的右滑仍是系统的。每个带导航的屏都要验证。

## 待验证

- 噪点平铺在长列表里的滚动帧率，两端低端设备各测一次，结果写在这里。
