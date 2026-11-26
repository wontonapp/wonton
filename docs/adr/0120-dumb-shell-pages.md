# ADR-0120 界面：核心给页面描述，壳每种组件只写一次

- 状态：提议（2026-11-25）
- 扩展：[ADR-0005](0005-native-clients-thick-core.md)（状态和 ViewModel 在核心、壳只渲染）、
  [ADR-0077](0077-plugin-settings.md)（表单形状由核心给、壳渲染控件子集、未知项要求更新 App，
  这里把同一套做法从一张表单扩到整页）
- 不动：[ADR-0026](0026-design-system.md)、[ADR-0029](0029-platform-material.md)（组件怎么画、按不按平台走都不变）、
  [ADR-0043](0043-app-languages.md)（错误和状态仍然给类别、壳写文字，见“文字”一节）

今天加一个普通界面（一个设置项、一页列表、一张表单），要分别改 Rust 核心、给手机用的绑定层、
苹果和安卓两套界面，四处都要动、四处都可能出错；同样大小的活，两头一起改比只改一头慢两到三倍、
被复审打回的也多。这份方案把“页面长什么样”从苹果和安卓两边收回到核心一处：核心说这页有哪些分组、
每行写什么、点什么，两个 App 各自照自己平台的画法把每种组件只实现一次。以后加这类界面只改核心，
两个 App 同时就有了。

## 结论

一句话：**普通页面由核心发一棵“分组 + 组件”的树，壳每种组件只实现一次、按 ADR-0026/0029 的令牌
和平台材质画；重交互页继续手写。**

- 新增界面大多只改核心一处：页面形状、顺序、显示条件、按钮和取值都在客户端核心里写，壳的
  `PageRenderer.swift` / `PageRenderer.kt` 不动。
- 组件是封闭的清单（正文 12 种），核心不描述动画、手势、布局细节，也不在格式里放表达式；
  格式长成一门语言就收手。
- 手写清单是白名单，只有聊天时间线/输入、聊天列表、相册看图、播放器、插件网页、根导航和引导：
  这些页的主体是滚动位置、手势、增量更新或平台能力。
- 向前兼容照 [AGENTS.md](../../AGENTS.md) 的“兼容”：未知组件、未知取值不显示（要求更新 App 的
  那种组件显式说明），未知动作灰掉；页面带版本号。
- 先迁改动最频繁的“设置首页”和“应用商店”两页做样板，再按“文字页 → 列表页 → 表单和详情页”推进，
  每迁一页删掉一页壳的实现。

## 背景

主人在 2026-11-25 拍板不重写 Rust。一个功能今天要同时改核心、UniFFI 绑定、Swift 和 Kotlin 四处；
实测近一个月（10-25 至 11-25，基线 `225ea5cb`）`apps/` 下的改动，两套口径都列：按提交 276 个，其中 141 个
两端一起改（51%）；按（提交，页面）对 614 个，其中 251 对两端同改。命令和口径见
`scripts/adr-shell-pages.py`（`python3 scripts/adr-shell-pages.py --rev 225ea5cb --since 2026-10-25`，
`--rev` 必填，只算该 rev 可达的历史）。
PR 中位 1.5–6.6 小时，核心加两个壳一起改的慢两到三倍，被复审打回的约 3 倍。

两端同改最集中的是页面壳，按（提交，页面）对：聊天房间 71 对（两端同改 32 对）、插件详情 55（32）、
插件网页 52（15）、聊天面板 32（6）、应用首页 31（13）、聊天列表 31（11）、输入框 31（10）。反过来，
已经由核心给数据的页只有 19（8）、表单 4（2）、关于 1（1）。数据见文末“现状盘点”。

也就是说：**壳里的页面代码就是成本**；今天已经有半套地基，缺的是“壳不为每个新页面再写一遍”：
设置首页的 `SettingsViewModel`、`StoreListing`、`PeersViewModel`、`PluginSettingsView` 都已经由核心给
结构、顺序、状态和可点的按钮，壳却还是每页各写一个 `switch` 把每种取值翻成一个手写的行。

## 决策

### 1. 页面描述：分组 + 组件，只有内容

新协议在 `crates/wonton-protocol`，一页只有这些：

```text
PageView { id, version, title, subtitle?, toolbar: [Button], body: [Section], refresh?: Action,
           error?: PageProblem, empty?: PageEmpty }
Section  { header?, footer?, blocks: [Block] }
PageProblem { kind: PageProblemKind, detail? }
PageEmpty { kind: PageEmptyKind, action?: Action }

- 内容全部来自核心：标题、值、开关状态、字段、段落、按钮文字和能不能点，都由核心按当前状态算好。
- 页面的空态和出错由核心给类别（`PageEmptyKind`、`PageProblemKind`）和可选的技术细节，
  壳按 ADR-0043 用各语言的文字渲染，句子不从核心走。
- 格式里没有表达式、条件、循环、模板：要变的内容在核心变成字段，组件只做“显示这个值”。
- 没有动画和手势描述。字符串类型的滚动位置、列表锚点由壳记，核心只给一次性 id。

### 2. 组件清单（v1）


每种组件在两端各实现一次，落在 `Design/` 已有的控件上，不新画控件。核心只给内容，颜色、字号、圆角、
间距和栏的材质都由壳按 ADR-0026/0029 的令牌定；格式里不出现色值和尺寸。

| 组件 | 用途 | 落在哪 |
| --- | --- | --- |
| Section / Separator | 分组、分隔 | `WontonGroup`、`WontonSeparator` |
| Row | 标题、副标题、前置图标或头像、右侧值、角标、勾选、前进箭头、开关 | `WontonRow`、`WontonSwitch` |
| Field | 文本、整数、小数、单选、多选（沿用 ADR-0077 的 `PluginSettingKind` 子集和问题行）、密钥（沿用 `SecretInputs` 的输入形态） | `WontonTextField`、`WontonRadioRow`、`WontonSwitch` |
| Button | 页面工具栏和卡片上的主/次/危险按钮 | `WontonButton`、`WontonIconButton` |
| Notice | 提示（信息、警告、错误），可带一个动作 | `WontonNotice` |
| Card | 标题 + 正文 + 按钮，可带角标 | `StepCardFrame` |
| Paragraph | 标题 + 段落正文，不做 Markdown | `Text` 排版 |
| Search | 搜索框（商店、列表） | `WontonSearchField` |
| Chips | 单选过滤（商店分类） | `WontonChip`（`Design/WontonPills`） |
| Progress | 有长度的进度、批量结果（安装、升级） | 现有进展条 |
| Empty / Error | 空态和错误态，带重试动作 | `ProblemRow` 系列 |
| Icon / Avatar / Image | 前置图形 | 现有图标与 `PluginIcon`、`BotAvatar` |

新组件先进本表、再进检查脚本的白名单；表外的画法仍写在页面里。

### 3. 动作

每个可点的行、按钮、字段带一个动作，来自页面自己的动作枚举（今天已经有 `SettingsAction`、
`PluginAction`、`PageAction` 这些），经 `Action.Pages` 一个入口回核心。壳只做“把动作原样发回去”，
不解释。要确认的动作带一句确认文字，确认页由壳呈现（ADR-0024 的三类不变）。

### 4. 哪些页面继续手写（白名单）

- 聊天房间：时间线增量、滚动锚定、引用拖拽、下拉面板、输入框和附件。
- 聊天列表：滑动、长按菜单、未读和置顶。
- 相册看图、图片查看器、播放器：手势、缩放、解码、后台音频。
- 插件网页（`TabPage`）、分享扩展进来的一屏：WebView 和桥。
- 根导航、侧边栏、页面栈外壳。
- 引导：多步流程、相机扫码、平台权限。

划线规则：**主体是“静态的行、字段、段落、卡片”就迁；主体是“跟着手指或增量流变”就手写。**
白名单里的页如果出现大段可描述的区块（例如房间页的“等确认”区、设置的浮层），那一段仍可用组件，
整页仍手写。

### 5. 向前兼容

- 页面的 `version` 是清单版本，壳只渲染自己认识的版本；高版本照常渲染认识的部分。
- 未知组件：直接不显示；未知组件如果被标了 `requires_update`（例如未知字段类型，见 ADR-0077），
  在页尾显示一句“要更新 App 才能用”，不显示可点占位。
- 未知动作：组件照常显示，灰掉。
- 协议层的 `#[serde(other)]` 和测试照 `SettingsEntry` 的写法补一份（`settings.rs` 里已有同样的用例）。

### 6. 文字

- 页面内容文字（标题、值、字段名、按钮、段落）由核心按 App 语言写好。客户端核心已有 Fluent 目录
  （`crates/wonton-client/locales`，按 `SetLanguages` 协商，见 ARCHITECTURE.md 的“本地化”），
  组件自带的通用词（“重试”“搜索”“保存”“取消”）进同一个目录。
- 错误、空态、权限和来源的类别仍按 ADR-0043 由壳写：壳按类别渲染，不按核心的句子。
- 壳只为白名单页和平台权限保留自己的字符串；迁走的页面把对应 `xcstrings`/`strings.xml` 条目一起删。

### 7. 契约、提供方、消费方

- 契约：`crates/wonton-protocol` 的 `PageView` 及其组件枚举，随 UniFFI 生成两端的类型。
- 提供方：客户端核心里每页的 mapper（今天出 ViewModel 的函数改成出 `PageView`），不需要服务器。
- 消费方：`apps/ios`、`apps/compose` 各一个 `PageRenderer`。
- 契约测试：
  - `wonton-protocol` 的金样本例：每种组件一份 JSON，解析、再序列化、未知组件落 `Other`；
  - `scripts/check-shell-parity.py` 增加组件清单：某个组件只有一个壳实现或两边都没实现时报警；
  - 两端各一份预览测试读同一份 fixture，同一份 fixture 驱动一条 iOS UI 测试和一条 Android 仪器测试，
    断言行数、文字和动作一致；
  - 换页面的提供方（同一页改由另一个核心模块出）时壳不动，作为“换提供方消费方不用改”的验收。

### 8. 不采用现成方案的理由

不造轮子先看现成的东西，结论是格式自留、做法能抄就抄：

- **Microsoft Adaptive Cards**（MIT）：只描述卡片，没有整页、工具栏、分段和页面栈；渲染靠 host config
  再定义一层样式，等于在两套外观体系之间再做一次映射。卡片可借鉴的“`fallback` 写进格式”抄了
  （第 5 节）。<https://adaptivecards.microsoft.com/schemas/adaptive-card.json>
- **Slack Block Kit**：blocks/elements/composition objects，一条消息最多 50 个块、弹窗 100 个；是
  Slack 的界面概念（Home tab、modal），校验只到字段长度和数值范围（`plain_text_input` 的
  `min_length`/`max_length`、`number_input` 的 `min_value`/`max_value`），没有跨字段和提交门的校验、
  没有页面动作和页面栈。 <https://docs.slack.dev/block-kit/>
- **Google Remote Compose**（Apache-2.0，`androidx.compose.remote`，2026-11-18 还是 `1.0.0-alpha24`）：
  文档 + player 的结构值得抄，但 player 只有 Android（App 小组件、Wear OS），iOS 端要自己写 player
  （未核实有第三方）；而且 player 是在画布上画，不产出我们已有的原生控件，ADR-0029 的平台材质要重做一遍。
  <https://developer.android.com/develop/ui/compose/system/remote-compose>、
  <https://developer.android.com/jetpack/androidx/releases/compose-remote>
- **JSON Forms / JSON Schema 渲染**：只管表单；字段种类、问题行、未知项的做法 ADR-0077 已经有了，不重复引入。
- **Matrix 小部件 / Shopify remote-dom**（matrix-widget-api 是 Apache-2.0，remote-dom 是 MIT）：
  都是“把第三方代码放进沙箱、经消息传 UI 树”，适合插件自带界面；我们的插件页面已经走 WebView + MCP Apps，
  原生页再引一层 JS 运行时换不回原生体验。
  <https://github.com/matrix-org/matrix-widget-api>、<https://github.com/Shopify/remote-dom>
- **Rust 侧的现成 crate**：`ferro-json-ui`（MIT）、`raui-json-renderer`（MIT/Apache-2.0）都是给自家渲染器
  用的 JSON UI，编进 SwiftUI/Compose 用不上，只当先例看。

自研的代价因此很小：一棵无表达式的小树、客户端核心里已经带 serde 和 UniFFI、不跨网络（处理进程内），
也没有服务端驱动 UI 常见的响应体积和延迟问题。

### 9. 迁移顺序

每步一页（或一小组同类页）一个 PR，同一 PR 删掉两端的旧实现和迁走的文案：

1. **样板**：设置首页（近月 19 次、两端同改 8）和应用商店（17 次、11）——两页都是纯行和卡片，一个证明
   分组和行，一个证明卡片、搜索、过滤和批量进度。
2. 关于、隐私、许可、通知、App 锁、外观、出网代理、镜像源、聊天备份、我的应用：文字页和表单页。
3. 插件详情按节迁（来源、共享、密钥、权限、工具策略、端口），插件设置表单改字段组件。
4. 其他服务器、好友、Bot 资料和联系人、确认卡片、分享进来的一屏、设备验证、核心升级、应用首页。
5. 引导最后做；做完把白名单以外的页面代码清零。

### 10. AGENTS.md

“不造轮子”一节增加一句：新写的独立组件（服务端插件、运维工具、网页）按合适的语言选，Go、TS 都行，
不必用 Rust；客户端核心和服务器核心保持 Rust（AGENTS.md 的同一节随本 ADR 一起改）。

## 理由

- 一个功能的壳侧成本从“两边各写一页”降到零；核心一处写完，两端同时得到。
- 壳里不再有 40 份 `switch` 和 1200 多条只给一页用的文案；组件的样式、读屏和大字号支持只维护一份。
- 与 ADR-0005 同向：核心已经是状态和 ViewModel 的事实源，这里只是把“形状”也交过去。
- 出错的界面由核心按类别给，两端提示一致，不再靠两处人肉对齐。

## 代价与风险

- 描述格式可能长成半门语言。约束：没有表达式；组件种类进白名单要评审；出现第三种“条件渲染”的写法就收。
- 壳的灵活性降低。白名单外的页要手写，写之前先问“能不能用组件表达”；不能就进白名单并说明理由。
- 调试要跨语言：行不显示时先看核心出的树，再看渲染器。渲染器遇到不认识的组合要报出页 id 和组件 id，不静默。
- 页 id、字段名是动作路由的键，改名要一起改核心和测试。
- 首版的工作压在壳上：12 种组件两端各写一次，加契约测试，约等于写完这几页的现有实现；收益从第二个页面起才出现。
- 两端渲染器的细节仍可能漂移，靠第 7 节的同 fixture 测试和检查脚本压。

## 待验证

- 一份 fixture 同时驱动两端预览和一条两端 UI 测试，能否真的抓住渲染漂移（样板页做完后实测）。
- 应用商店这种带搜索、过滤和批量进展的页，用组件表达后滚动性能和输入手感是否与今天一致（模拟器实测）。
- 大页面（设置首页 + 商店）的树体积和首帧时间；有无必要把 section 分页下发。

## 兼容与迁移

- 没有外部用户，ViewModel 直接改，不写迁移（AGENTS.md“兼容”）。旧 ViewModel 类型在迁走那一页时删掉，
  不和新格式并存。
- 未知组件、未知动作、`requires_update` 的处理随格式一起进核心和两端，先于第一个迁移页。

## 现状盘点（2026-11-25 的 main，`225ea5cb`）

统计：`python3 scripts/adr-shell-pages.py --rev 225ea5cb --since 2026-10-25`（脚本在仓库里，口径见脚本头；
`--rev` 必填，只算该 rev 可达的历史）。两套口径都列：按提交 276 个、其中 141 个两端同改（51%）；
按（提交，页面）对 614 个、其中 251 对两端同改。
“iOS 提交 / Android 提交”两列两端各算一次（同一个提交两端都改时两列各记 1），所以两列之和大于对数。
下表就是脚本的输出；“核心给什么”是今天的现状。

| 页面 | iOS 行 | Android 行 | iOS 提交 | Android 提交 | （提交，页面）对 | 其中两端同改 | 核心给什么 | 结论 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 侧边栏与根导航 | 1548 | 2248 | 52 | 48 | 77 | 23 | 侧边栏条目、页面栈 | 手写 |
| 聊天房间（时间线） | 3051 | 2918 | 58 | 45 | 71 | 32 | 时间线增量、消息、卡片 | 手写 |
| 插件详情（含 Bot 资料、联系人、共享、密钥、权限） | 2902 | 3190 | 42 | 45 | 55 | 32 | 各节 ViewModel | 按节迁 |
| 插件网页 | 2088 | 2631 | 30 | 37 | 52 | 15 | 页面栈、主题、桥 | 手写 |
| 聊天面板 | 1172 | 1417 | 21 | 17 | 32 | 6 | 面板条目与状态 | 手写 |
| 应用首页 | 618 | 549 | 29 | 15 | 31 | 13 | 网格 ViewModel | 迁 |
| 聊天列表 | 574 | 802 | 22 | 20 | 31 | 11 | 会话列表 | 手写 |
| 输入框 | 1708 | 1701 | 24 | 17 | 31 | 10 | 发送、附件、草稿 | 手写 |
| 服务器、登录、邀请、配对 | 1125 | 1362 | 19 | 21 | 28 | 12 | 各自状态 ViewModel | 迁 |
| 问题与空态 | 779 | 558 | 17 | 15 | 21 | 11 | 问题类别 | 变组件 |
| 设置首页 | 292 | 290 | 16 | 11 | 19 | 8 | `SettingsViewModel` | 样板 |
| 应用商店 | 765 | 897 | 15 | 13 | 17 | 11 | `StoreListing`/`StoreSection`/`StoreItem` | 样板 |
| 确认卡片、待办 | 807 | 1056 | 13 | 13 | 17 | 9 | 卡片文字、可用按钮 | 迁 |
| 聊天备份 | 346 | 442 | 13 | 11 | 15 | 9 | 状态 ViewModel | 迁 |
| 好友 | 729 | 859 | 11 | 8 | 13 | 6 | `FriendsViewModel` | 迁 |
| 引导 | 653 | 845 | 8 | 11 | 13 | 6 | 步骤 ViewModel | 手写，最后 |
| 我的应用 | 361 | 691 | 8 | 7 | 11 | 4 | 设置 ViewModel | 迁 |
| 图片查看、文件播放 | 929 | 1405 | 4 | 9 | 11 | 2 | 媒体句柄 | 手写 |
| 通知 | 115 | 345 | 4 | 8 | 11 | 1 | 类别与状态 | 迁 |
| 资料、密钥 | 652 | 930 | 9 | 7 | 10 | 6 | `OwnerProfile*`、`OwnerSecretsView` | 迁 |
| 其他服务器 | 733 | 807 | 8 | 6 | 9 | 5 | `PeersViewModel` | 迁 |
| 设备验证 | 473 | 485 | 5 | 6 | 7 | 4 | 状态 ViewModel | 迁 |
| 核心升级 | 633 | 604 | 5 | 4 | 6 | 3 | 状态 ViewModel | 迁 |
| 镜像源 | 220 | 327 | 4 | 3 | 5 | 2 | 状态 ViewModel | 迁 |
| 插件设置（表单） | 550 | 610 | 4 | 2 | 4 | 2 | `SettingsForm` 字段描述 | 改字段组件 |
| 出网代理 | 234 | 339 | 4 | 2 | 4 | 2 | 状态 ViewModel | 迁 |
| 外观 | 349 | 379 | 3 | 3 | 4 | 2 | 选项 ViewModel | 迁 |
| 分享收进 | 349 | 534 | 3 | 2 | 4 | 1 | 状态 ViewModel | 迁 |
| 插件来源 | 251 | 269 | 2 | 2 | 3 | 1 | 列表 | 迁 |
| App 锁 | 253 | 353 | 1 | 1 | 1 | 1 | 状态 ViewModel | 迁 |
| 关于、隐私、许可 | 168 | 164 | 1 | 1 | 1 | 1 | 全文由核心给 | 迁 |

业内做法（看过原文的标了出处，只有摘要的写明）：

- Airbnb 的 Ghost Platform：响应按 screen/section 下发，每个 section 带 `sectionComponentType` 选渲染方式，
  每个平台有自己的客户端框架；同一种 section 数据可以换一种画法。
  <https://medium.com/airbnb-engineering/a-deep-dive-into-airbnbs-server-driven-ui-system-842244c5f5>
  （原文打不开，读的是俄文全文翻译 <https://habr.com/ru/companies/alconost/articles/568444/>）。
  Airbnb 自己说过响应体积的问题（转引，未核实原文）：<https://www.voqal.ai/blog/server-driven-ui-in-the-llm-era>
- DoorDash：通用组件 + 动作枚举，早期只能顺序排、后来补分组和按节刷新。
  <https://careersatdoordash.com/blog/improving-development-velocity-with-generic-server-driven-ui-components>
  （Cloudflare 挡了抓取，只有摘要，未核实原文。）
- Hinge Health：原生组件，服务端知道每个 App 版本能渲染什么、把不支持的省掉，设备端再校验一次，坏的跳过并上报；
  “服务端说布局该做什么、App 决定长什么样”。<https://rnd.hingehealth.com/blog/from-feature-to-platform-lessons-from-scaling-server-driven-ui>
- 已知失败模式：格式长成没有类型检查器和调试器的编程语言；动画和手势描述不了；旧版本兼容是长期税。
  <https://kenin.ch/blog/server-driven-ui>、<https://dezentech.com/blog/server-driven-ui>
- Shopify：remote rendering，UI 树经 RPC 序列化成 JSON 交给宿主重建，宿主有 JS、Kotlin、Swift 实现。
  <https://shopify.engineering/remote-rendering-ui-extensibility>
