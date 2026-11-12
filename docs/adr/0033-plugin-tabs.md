# ADR-0033 底栏的插件页、全屏和 Agent 写插件：网页打底，App 管栏和主题，桥接用 MCP Apps 的词汇

- 状态：已接受（2026-11-04）。取舍由主人确认（见“已定”）
- 取代：[ADR-0014](0014-reserved-capabilities.md) 的“小程序 JS SDK”；[ADR-0028](0028-settings-page.md) 的“入口是底栏最后一项”（设置默认在栏上，可以拿掉）；[ADR-0016](0016-mcp-bus-and-confirmations.md) 的规则 4 和 [ADR-0017](0017-plugin-ecosystem-and-store.md) 的“谁能装”：Bot 能装官方插件和自己写的插件，高权限都经卡片；[ADR-0024](0024-confirmation-scope.md) 的判定：Bot 更新自己写的插件，权限没变宽时不发卡片

## 背景

主人要把底栏变成可插拔的槽位：插件提供一个页面，用户装上后固定到底栏，点图标就在 App 里浏览，用起来和聊天、设置一样。用户能调顺序和数量，有上限。社区做一个 RSS 插件，别的用户装上就能固定。Wonton 不内置动态、想法、目标这些页面，要的是通用机制。

主人给的参考是 Muse Bot（`0216zerotwo/muse-bot`，它的界面学 Meta Muse 的聊天、动态、想法、目标、资料库五栏）。读过它的代码（2026-11-04）：iPhone 版的底栏页是写死在 App 里的 SwiftUI 屏（`FeedScreen`、`GoalsScreen`），顺序存在本机；安卓版整个是 WebView 加原生能力。这解释了截图为什么顺滑，也说明它不是可参考的插件机制：页面是 App 自己的代码，第三方加不进来。我们要在“第三方页面”的前提下尽量接近这种感觉。

主人还要求插件生态对开发者（包括 Agent）足够简单和开放：在聊天里跟自己机器上的 Agent 说想要什么，Agent 写出插件、自己装到机器上跑起来，用户在 App 里直接用，App 的界面也能自己定义。所以“Agent 能不能一次写对”是选技术的硬指标：只看公开规范和一个模板就写得出来，不编译原生代码，不签名不上架，报错清楚到 Agent 能自己改。

现状（`96524ca` 加 `feat/apps-tab`）：

- 底栏写死聊天、应用、设置三项，iOS 是自摆的胶囊（ADR-0029），Android 是 `NavigationBar`。
- 插件网页盖在主界面上，WebView 经本地代理走隧道（`PluginWebView`），每个插件一个子域名，自带同源隔离，登录由网关和 OIDC 解决。
- 清单只有 `webui`：一个入口，没有底栏用的图标和标题。

## 别人怎么做

| 产品 | 页面是什么 | 入口和数量 | 主题和壳 | 用上的 |
| --- | --- | --- | --- | --- |
| Telegram Mini Apps | 网页，WebView | 附件菜单、侧边菜单、个人页按钮、直接链接，没有底栏槽位。附件和侧边菜单第一次要用户同意添加 | `themeParams` 和 `--tg-theme-*`、`themeChanged`；`ready()` 之前显示占位；原生顶栏和返回键由 `BackButton` 控制；`requestFullscreen()` 全屏，安全区分系统和 Telegram 自己的按钮两种 | 占位盖到页面说好了，令牌用 CSS 变量，安全区也用变量给，也分开给 |
| 微信小程序 | 自有语言，渲染层和逻辑层分开 | `tabBar.list` 最少 2 个、最多 5 个，由小程序自己定 | 宿主固定胶囊，页面藏不掉；`navigationStyle: custom` 去掉导航栏，胶囊还在，位置由接口查 | 数量上限；胶囊和“退出的路一直在”；自研框架的代价是整个生态，不学 |
| Home Assistant | 网页（Ingress）、自定义面板 | 清单写 `panel_icon`、`panel_title`，用户在侧边栏调顺序和隐藏 | App 和网页之间有 `externalBus`：`config/get` 报能力、`theme-update`、`haptic` | 清单写法；最像我们的处境。它的路线图 [#244](https://github.com/OpenHomeFoundation/roadmap/issues/244) 正要做一样的事：原生栏、每个标签放 WebView、内容还是网页，同样把“每个标签一个 WebView 的内存”列为要实测的风险 |
| Slack | Block Kit，JSON | App Home 的 Home 标签，每个 App 一个 | 宿主全画 | 声明式够用的范围就是列表、按钮、表单 |
| Discord Activities | 网页，iframe | App Launcher | Embedded App SDK，postMessage 握手 | 和 MCP Apps 同一种握手 |
| ChatGPT Apps SDK、Claude | MCP Apps，`ui://` 的 HTML，沙箱 iframe | 工具调用的结果里 | `hostContext`、`host-context-changed` | 见下 |
| Muse Bot | iPhone 原生屏，安卓 WebView；聊天里的卡片是生成的 HTML，放在不带凭据的 iframe 里，只能回传高度和发送文字 | 固定五项，顺序存本机 | 令牌传进 iframe | 页面只能把文字发进聊天，且要用户刚操作过 |

没有哪家把第三方页面放进底栏槽位：Telegram 和 Discord 都放在菜单里，微信是小程序内部的栏。Home Assistant 最接近，它的结论也是原生壳加网页内容。

让 Agent 写和装的做法：

| 产品 | 写什么、怎么交付 | 报错和迭代 | 用上的 |
| --- | --- | --- | --- |
| Home Assistant 的社区 MCP（[ha-mcp](https://github.com/homeassistant-ai/ha-mcp)、[Vibecode Agent](https://github.com/Coolver/home-assistant-vibecode-agent)） | 自动化、面板、主题，都是 YAML，经 MCP 写进去。ha-mcp 的 README 说官方 MCP 只能控制设备，建不了这些 | Agent 能读 Core 日志；Vibecode 改之前先提交到一个影子 git 仓库，能回滚 | Agent 要读得到日志；要能退回；配一份 Agent Skills 教它怎么写 |
| [Coolify 的社区 MCP](https://github.com/Thedurancode/Coolify-MCP) | 部署应用，`deploy_app` | `get_deployment_status`、`get_deployment_logs`，部署前查名字冲突和配额，写审计 | 工具就这么几个：部署、状态、日志 |
| [Claude Artifacts](https://claude.com/blog/build-artifacts) | 网页，托管发布，能调 MCP 连接器 | 对话里重写 | 模型写界面的主流交付物就是网页 |
| v0 | 全栈项目，平台构建和预览 | [社区反馈](https://community.vercel.com/t/v0-auto-diagnosis-automatically-editing-code-and-creating-new-versions-without-consent/36431)自动诊断会没完没了地改代码、白花额度 | 只有社区说法，算旁证：Agent 别在没有用户的回路里无限重试，让它看到准确的原因 |

给 Agent 看的说明有两个开放格式。[Agent Skills](https://agentskills.io/specification) 是一个 `SKILL.md` 目录，先读名字和描述，用到才读全文，Hermes 支持。[llms.txt](https://llmstxt.org/) 是站点根目录的一页索引，还是提案。经 MCP 发现 skill 的 [SEP-2640](https://modelcontextprotocol.io/extensions/skills/overview) 已定稿，文档写着 SDK 和宿主的支持还在实现。

## 四种渲染方式

| | 插件网页放 WebView（不处理） | MCP Apps（`ui://`） | 声明式原生（JSON 画控件） | 混合：网页打底，壳和主题归 App |
| --- | --- | --- | --- | --- |
| **像不像原生** | 不像：白屏、两套顶栏、颜色不跟 | 看作者；令牌标准，首屏是本地 HTML，不白屏 | 最像，就是原生控件；但只有组件库里的东西 | 适配过的页面接近原生；现成应用还是网页的样子，至少不白屏，栏是 App 的 |
| **深色和重点色** | 只有写了 `prefers-color-scheme` 的跟明暗，重点色跟不了 | 明暗和令牌都跟（`hostContext.styles`） | 自动跟 | 明暗和令牌都跟，重点色当场生效（实测见“验证”） |
| **滚动和手势** | 滚动原生，内容到不了栏下 | iframe 里滚动，手势归网页 | 原生 | 适配过的从栏下滚过，返回手势用 WebView 的 |
| **作者门槛** | 最低，现成应用不用改 | 要写 MCP 服务，把页面打成单个 HTML，走工具取数据 | 只写 JSON，但得按我们的组件想问题，表达力受限 | 现成应用不用改；想要原生感只加几个 CSS 变量，可选官方 SDK |
| **页面能拿到什么** | 插件自己的登录和数据 | 没有 Cookie，数据全走 `tools/call`，经确认卡片，审计最好 | 没有代码，最安全 | 同第一列，桥只多给主题和“把草稿放进聊天”，见“页面能拿到什么” |
| **断网** | 502 或白屏，要壳接住 | HTML 可缓存，数据要工具，断了也要壳接住 | 缓存上次的 JSON，最好 | 壳接住（ADR-0032）；已加载的留着 |
| **App Store** | 4.7：HTML5 小程序允许，开发者负责 | 同左 | 不下载代码，避开 2.5.2 的疑问，但插件内容仍要合规 | 同第一列，桥不暴露原生能力，4.7.2 没问题 |
| **内存** | 每个标签一个 WebView，用量未测 | 同左，再加一层 iframe | 几乎没有 | 同第一列，上限 3 个 |
| **两端代码** | 已有 | Rust 做 JSON-RPC，两端壳加一个 iframe 宿主页 | 每个组件两端各写一遍，组件越多越贵，还要一份编辑器和文档 | Rust 做 JSON-RPC 和注入脚本，两端壳加一个“标签页”容器 |
| **现成应用（Immich 等）** | 能放 | 放不了：规范的 `externalUrl` 还在“以后”，只收内联 HTML | 放不了 | 能放 |
| **Agent 能不能一次写对** | 能 | 能，但东西多：MCP 服务、工具、内联成单文件的页面，都要配套。我们没查到手机上的宿主可以试 | JSON 容易写合法，但这是 Wonton 专有的词汇；组件库里没有的控件做不出来 | 能，写普通网页，令牌是可选的几个变量。实测见“验证” |

这一行定了声明式出局。模型写网页的经验最多，浏览器也最宽容，写错一点通常页面还能显示。声明式的 JSON 不难写，A2UI 就是为模型生成设计的，难的是表达力：想要的控件组件库里没有，就要等我们改 App、过审核、用户更新。这正好和“用户想要什么，Agent 当场做出来”相反。RSS 要抓订阅源，所以无论页面怎么画，后端都要写，声明式省不掉这部分。原生代码更不行：iOS 不许下载后执行（2.5.2），而且要编译和签名。

MCP Apps 也能写，但没有比网页多给什么：底栏页本来就有自己的后端，不需要工具和单文件这一层。

### 查实的几点

- **MCP Apps 的规范**（`2026-01-26`，SEP-1865，SDK `@modelcontextprotocol/ext-apps` 2.0.4，MIT）是从工具调用开始的：工具的 `_meta.ui.resourceUri` 指向 `ui://` 资源，宿主渲染后把工具结果推给页面。规范没有“不经工具调用直接打开”的路径，MIME 只收 `text/html;profile=mcp-app`，`externalUrl`（嵌外部网页）和状态持久化都列在以后。宿主有 Claude、VS Code、ChatGPT、Goose 等，都是聊天里的卡片，我们没查到移动端原生宿主的实现或指南；规范只给了 `platform: "mobile"` 和 `safeAreaInsets`。
- **声明式：** Adaptive Cards 的移动端 SDK 还在发（CocoaPods 2.12.1），据一篇旧文，iOS 是 Objective-C 加 UIKit，样式要另外接我们的设计系统。A2UI 的 Compose 渲染器是 `1.0.0-alpha02`，规范 0.9，SwiftUI 的官方状态说法不一，现在押不起。两者都解决不了相册、影音这类内容。清单里的 `settings`（JSON Schema 表单）是声明式最合适的地方，已经留着，和这里分开。
- **WKWebView 的 Service Worker** 只有 App-Bound Domains（最多 10 个，写死在 `Info.plist`）或默认浏览器资格才能用，插件的域名是动态的，所以离线不能指望页面自己缓存。

## 决策

### 一个机制：底栏页面就是插件网页，栏和主题归 App

不新增渲染技术。一个底栏页面是插件 `webui` 下的一个路径，放在 App 的 WebView 容器里；顶部和底部是 App 自己的栏；首帧、主题、状态都由 App 负责。任何有 `webui` 的插件都能固定，作者想要更好的体验再适配。

`ui://` 不作为底栏入口。它的位置是以后聊天里的卡片：Bot 调工具，结果在聊天里显示一张交互卡片。这条路和这里共用桥和令牌，另写一篇。

### 清单：`tabs`

进公开规范，写法以 [wonton-docs](https://github.com/geraldpeng6/wonton-docs) 为准。

```yaml
x-wonton:
  spec: "0.5"
  webui:
    service: web
    port: 8080
  tabs:
    - id: feed
      title: { zh: 动态, en: Feed }
      icon: tabs/feed.svg
      icon_selected: tabs/feed-fill.svg
      path: /feed
      layout: edge-to-edge
```

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `id` | 是 | 插件内唯一，小写字母、数字和连字符。固定记录按 `<插件 id>/<id>` 存，升级时靠它对应，不要改 |
| `title` | 是 | 多语言，至少 `zh` 和 `en`。显示在底栏和页面标题上，建议不超过 4 个字或 8 个字母，超长截断 |
| `icon` | 是 | 单色图标，PNG 或 SVG。只取透明通道，颜色由 App 上色。SVG 规则同“图标和截图”。核心栅格化成 PNG 交给 App，壳不解析 SVG |
| `icon_selected` | 否 | 选中时的图标（实心）。没写时选中只靠底色和字色 |
| `path` | 是 | `webui` 下的路径，以 `/` 开头 |
| `layout` | 否 | `inset`（默认）：页面夹在两条栏中间，现成应用不会被挡。`edge-to-edge`：页面铺满，从栏下面滚过去，作者用 `--wonton-inset-*` 留出位置 |
| `display` | 否 | `standard`（默认）：打开时带 App 的栏。`fullscreen`：打开就全屏，见“全屏” |

- 要声明 `webui`。不申请新权限：页面能做的事没有变多。
- 一个插件最多 4 项，每项可以单独固定。多页面是真实需求（文件和影音分开），但底栏只有几个位置，所以页面内部的分段还是插件自己做。
- 声明了 `webui` 没声明 `tabs` 的插件也能固定，入口是 `webui.path`，标题和图标用插件自己的。彩色图标按圆角方形显示，不上色。这样已经有网页的第三方应用不用改就能放。

`webui` 也能写 `display`，给没声明 `tabs` 的插件用。

`wonton-core plugin check` 检查单色图标的透明通道和长度。

### 底栏

- **上限 5 项。** iOS 和 Material 的习惯都是 3 到 5 个，微信也是最多 5 个。没有“更多”标签：侧边栏的“全部应用”和应用网格页列出所有插件。
- **默认只有“聊天”，栏上只有它时整条底栏不显示**（2026-11-09 定，入口挪进侧边栏），聊天页铺到底。显不显示由核心算好交给壳（`TabsViewModel.show_bar`），壳不自己数。
- **聊天不能移除**，是主入口。“应用”网格页不再是栏上的一项，从侧边栏的“全部应用”进；没有配对时还是先到它（`initialTab` 不变）。
  主界面的可呈现页面由 `TabsViewModel.pages` 给出，壳用它创建页面，用 `items` 渲染底栏；移除底栏入口不等于移除页面。
- **设置不再默认在栏上**：入口在侧边栏底下；主人在侧边栏长按应用选“固定到底栏”，底栏才出现（聊天加上固定的项），设置也可以这样固定。
- **每项的顺序都能调**，包括聊天。第一项不决定启动页：启动还是回到上次的页面，没配对时到配对页（`initialTab` 不变）。

固定和调整的入口：

1. 插件详情页的“固定到底栏”，有多个页面时是一组开关。
2. “应用”页长按图标的菜单。
3. 装好后的完成页多一个按钮。
4. 设置里的“底栏”页：第一层只列栏上的项，可拖动排序、移除；“添加更多”进入二级页，列出能加的页面，使用插件自己的图标。底栏上长按也进这里。“显示文字”开关决定底栏是否只显示图标，和顺序一起存在 `tabs.json`。
5. 侧边栏“我的应用”里长按一行：固定到底栏，或从底栏拿掉。

满了再固定时，弹底部面板让用户选换掉哪一项，不静默失败。

配置（包括每个页面的全屏设置）每台设备各一份，和外观一样（ADR-0026）：手机屏幕和用途不同，也不依赖服务器，断网时能显示。放在 App 私有目录下单独的 `tabs.json`，写坏了不连累连接；读不了时退回默认（只有“聊天”）并如实提示，设置页说明原因。第一次配上服务器时重置为默认；换服务器（重新配对到别的服务器）时清空插件项。Bot 改不了底栏：配置只在这台手机上，核心和 Bot 都碰不到。Bot 装好插件后，确认卡片上有“放到底栏”按钮（见“Agent 写插件”）。

### 页面怎么显示

这些是 App 的事，插件不用写代码就有：

| 问题 | 做法 |
| --- | --- |
| 白屏 | 加载完之前 WebView 透明、背景设成 `canvas`（Android `setBackgroundColor`），盖一层 App 画的骨架。iOS 加载完改成不透明：WebKit 只在不透明时才在网页之外（全屏时 Home 条后面）画页面自己的背景色，透明时露出的是 `canvas`，内容在容器里滚的页面底下就是一条白（iOS 26.5 实测） |
| 骨架什么时候撤 | 收到 SDK 握手（`ui/notifications/initialized`）或页面加载完，以先到为准；没适配的页面靠后者，这和 Telegram 的 `ready()` 一样 |
| 第一帧的颜色 | 文档开始前注入脚本（iOS `WKUserScript`，Android `addDocumentStartJavaScript`），把令牌写到根元素，同时设 `color-scheme` 和根元素背景 |
| 切换明暗和重点色 | 不重新加载：换根元素上的变量，再发 `host-context-changed` |
| 字号（ADR-0028） | iOS 保持 `pageZoom = 1`，壳在文档开始时注入 `-webkit-text-size-adjust: <百分比> !important`，变更时原地更新；Android 用 `textZoom`。只放大文字，不缩小布局视口；安全区和栏高不除以文字比例。字体跟不了，页面用 `--font-sans` 的才跟 |
| 切标签 | WebView 常驻，切回来在原来的位置。第一次点才创建，不预加载：多数人不会每次都打开所有页面 |
| 进程被系统收掉 | iOS `webViewWebContentProcessDidTerminate`、Android `onRenderProcessGone`（Android 必须处理，否则整个 App 被杀）：重建，恢复地址，骨架盖着 |
| 顶栏 | 带栏时是 App 的标题栏，标题取清单，能后退时显示返回。没适配的页面从它下面开始，它不垫底色；适配过的页面内容从下面滚过，iOS 垫材质，Android 不透明（ADR-0029）。全屏时没有标题栏，见“全屏” |
| 网页框 | 带栏时没适配的页面限在标题栏和底栏之间，不铺到底栏后面：现成应用常有自己的底部按钮栏，铺过去会和我们的底栏叠在一起，被挡住或要页面另外让（主人 2026-11-06 定）。底栏周围是 App 底色，不取网页的颜色 |
| 返回 | iOS 边缘滑动走 WebView 历史，Android 返回键先退网页历史，到头再按平台习惯回到第一项。全屏时见“全屏” |
| 再点一次当前标签 | 回到页面最上面（文档滚动的页面由壳做）。有内层滚动的页面监听 `window` 上的 `wonton:tab-reselected` 自己处理 |
| 键盘 | 底栏和聊天一样让开 |
| 点输入框时放大 | iOS 上输入框字号小于 16px 时 WebKit 会在聚焦时放大页面，失焦不缩回。壳打开 `ignoresViewportScaleLimits`（和 Safari 一样，页面禁了缩放也能双指缩放），文档结束时给没写 `maximum-scale` 的 viewport 补上 `maximum-scale=1`：聚焦放大以它为上限，双指缩放不受它限制。没有 viewport 的桌面宽度页面不补，聚焦放大对它有用。Android 的 WebView 聚焦不放大（API 35、WebView 124 实测，13px 的输入框）。壳打开 `builtInZoomControls`（不显示缩放按钮），`DOMContentLoaded` 时去掉 viewport 的 `user-scalable=no` 和小于 5 的 `maximum-scale`，和 iOS 一样页面禁了缩放也能双指缩放 |
| 外部链接 | 主框架只能留在插件自己的域名、`auth.<基础域名>` 和核心的 `/_wonton/oidc/`（OIDC 登录要经过），别的地址用系统浏览器打开，桥也只对插件自己的主框架开放 |

令牌用 MCP Apps 标准名，加上它没有的几个：

| 变量 | 取值 |
| --- | --- |
| `--color-background-primary`、`-secondary` | `canvas`、`surfaceSubtle` |
| `--color-text-primary`、`-secondary` | `textPrimary`、`textSecondary` |
| `--color-border-primary` | `separator` |
| `--color-*-danger`、`-success`、`-warning` | 状态色 |
| `--font-sans`、`--border-radius-*` | 系统字体，`control`、`card` 等圆角 |
| `--wonton-accent-fill`、`--wonton-accent-ink`、`--wonton-accent-ink-on-fill` | 重点色三层，ADR-0026 的规则照用 |
| `--wonton-inset-top`、`--wonton-inset-bottom` | 栏占的高度，含安全区；全屏时是状态栏下沿（横屏为 0）和 Home 条，胶囊会自己藏起来，不算在内。`inset` 布局下是 0 |
| `--wonton-inset-left`、`--wonton-inset-right` | 左右安全区，横屏全屏时躲刘海和圆角用。`inset` 布局下是 0 |

`env(safe-area-inset-*)` 给的是系统安全区，不知道我们的标题栏和底栏；iOS 26 的 `obscuredContentInsets` 能改视口，但最低版本是 17，Android 也没有，所以用变量。

官方 SDK 严格校验 `hostContext.styles.variables`，出现标准以外的键，握手直接失败（实测）。所以 `--wonton-*` 只写到根元素，不进 `hostContext`；标准变量两边都有，只读 CSS 变量的页面和用 SDK 的页面拿到的一样。

`design/tokens.toml` 生成这张映射（两端和文档同一份）。这是公开接口，变量只增不改名。

### 全屏

有些应用自带底栏，放在 App 的底栏上面就是两行，又丑又难点（实测的 Actual Budget 就是这样）。参考微信小程序的胶囊：全屏时隐藏 App 的顶栏和底栏，只留右上角一个胶囊。状态栏竖屏照常显示，横屏由系统隐藏。

**怎么进：**

- 顶栏右边的展开按钮。
- 长按底栏图标，菜单里有“全屏打开”和“总是全屏”。
- 清单写了 `display: fullscreen`，或用户设了“总是全屏”，打开就是全屏。
- 适配过的页面调 `ui/request-display-mode`。

**胶囊：** 浮在页面右上角，顶边是状态栏下沿 + 8（横屏是屏幕顶 + 8），不为它让出一行。一层淡黑的底、白色图标带淡阴影，深浅页面上都看得出，不跟页面换颜色。

- **自动隐藏：** 进来、切回 App、加载完、转屏时出现，3 秒没碰就淡出。手指在网页上往下拖（往回看）24pt 叫出来，往上拖（往下读）立刻藏起。
- **只看手指，不看页面滚没滚：** 整页滚动、页面里的容器滚动、不能滚的单页和 canvas 都是同一条规则。壳只旁听拖动，不拦触摸；iOS 上旁听的手势挂在窗口上，页面自己处理触摸时也叫得出胶囊。
- **字号：** 胶囊和盖层标题栏右边那组按钮的字号封顶，最大的辅助字号下退出也不被挤出屏幕。
- **藏起来时不挡点按：** 透明度 0、不接收点按、读屏看不到，点到的是下面的页面（页面右上角常放头像和菜单，Immich 就是）。开着读屏时一直显示。
- 页面隐藏不了胶囊，用户往下拉一下就出来，所以不会困在里面。

**怎么退：** 胶囊左边是“···”，右边是退出全屏。

| 操作 | 结果 |
| --- | --- |
| 退出全屏，手动进的全屏 | 回到带栏的样子，还是这个页面，不重新加载 |
| 退出全屏，打开就全屏的页面 | 回到进来之前的标签（没有就回第一项）；固定的页面留在后台，未固定的页面释放，再打开重新加载 |
| “···” | 网页能后退时的“返回”、接管（声明了工具的插件）、重新加载、“总是全屏”开关、插件详情 |

未固定页与普通未固定页使用相同的关闭语义：多留一个隐藏 WebView 要多一套生命周期，不值。

**默认和用户设置：** 每个页面取值 `default`、`fullscreen`、`standard`，存在 `tabs.json`，默认跟清单。用户的选择优先于清单，两种方向都能改：作者写了全屏，用户也能关掉。

**安全区：** 谁躲开状态栏、Home 条，按页面的 `layout` 分两种，不看页面写了什么，两端规则相同。

| `layout` | 页面内容 | 内容够不到的边 |
| --- | --- | --- |
| `inset`（现成应用） | 从状态栏下沿开始，底边铺到屏幕底。`env(safe-area-inset-*)` 是 0 | 状态栏那一条是 App 底色，和页面的顶栏不连。底边：iOS 26 起用 `obscuredContentInsets` 让出 Home 条，页面画在后面，固定定位的元素和滚动的终点停在它上面；更早的 iOS 和 Android 停在 Home 条（导航栏）上面 |
| `edge-to-edge`（适配过的） | 铺满屏幕，作者按 `--wonton-inset-*` 留位置 | 没有 |

- **顶边为什么不用 `obscuredContentInsets`：** iOS 26.5 上一切换全屏，WebKit 就把顶边丢掉，页面钻到状态栏后面，试了几种复位都不稳。所以顶边由网页框定。
- **为什么接受状态栏那一条不连：** 带栏时网页上下都被栏限住；全屏时只剩系统状态栏，胶囊又会自己藏，基本挡不到页面（主人 2026-11-06）。想让页面画到状态栏底下，用 `edge-to-edge`，按 `--wonton-inset-top` 排自己的标题。
- **不替页面注入 `viewport-fit=cover`。** 没给的页面不知道要自己躲 Home 条，注入只会把按钮推进去。实测注入没有好处，见“验证”。网页框在安全区内时，`env(safe-area-inset-*)` 是 0，页面自己写了 `cover` 也不会多留一遍。
- **不按页面的 `viewport-fit` 自动切换：** iOS 的 WebKit 会按它把不写 `cover` 的页面留在安全区内，Android WebView 140 以下不会，`env()` 的值也不可靠（见“参考”），两端做不成同一条规则。所以要铺满的页面在清单里写 `edge-to-edge`，用 `--wonton-inset-*`。Android 上网页框的留白由壳按 `WindowInsets` 算。

**返回：**

| 操作 | 网页能后退 | 网页历史到头 |
| --- | --- | --- |
| iOS 左边缘滑动 | WKWebView 的返回手势 | 退出全屏，和点胶囊的退出全屏一样 |
| Android 返回键或手势 | 网页后退 | 退出全屏；再按时按平台习惯回到第一项 |
| 胶囊的退出全屏 | 不看历史，直接退出 | 同左 |

**全屏时照常有的：**

- 确认卡片的提示条（`PendingCardBanner`）盖在页面上。
- ADR-0032 的短状态浮在胶囊左边，不跟胶囊藏。确认卡片的提示条也不跟胶囊藏。
- “状态”一节的状态页。

### 桥

不发明 `window.wonton`，用 MCP Apps 的 JSON-RPC 方法名。作者可以直接用官方 SDK 的 `App` 类，也可以完全不用桥，只读 CSS 变量。

顶层页面的 `window.parent` 是它自己，SDK 的 `postMessage` 会发回给自己。注入脚本先接住页面发的 JSON-RPC 转给原生，回复再派发回来；不这样做，SDK 会把自己的请求当成对方的请求，答“方法不存在”，握手失败。

| 方法 | 第一版 | 说明 |
| --- | --- | --- |
| `ui/initialize`、`ui/notifications/initialized` | 有 | `hostContext`：`theme`、`styles`、`locale`、`timeZone`、`platform: mobile`、`displayMode`（带栏时 `inline`，全屏时 `fullscreen`）、`availableDisplayModes`、`safeAreaInsets` |
| `ui/notifications/size-changed` | 收下不处理 | SDK 默认会发；底栏页的大小由 App 定 |
| `ui/notifications/host-context-changed` | 有 | 明暗、重点色、字号、全屏状态变化时 |
| `ui/request-display-mode` | 有 | `inline` 或 `fullscreen`，只收主框架，回实际生效的模式。页面退不掉胶囊；用户把这个页面设成 `standard` 时不生效 |
| `ui/open-link` | 有 | 系统浏览器打开，只收 `https` |
| `ui/message` | 有 | 切到聊天，把文字放进输入框当草稿，**用户点发送才发**。发给哪个 Bot 由用户选的房间决定。这是动态页“讨论”按钮的做法 |
| `tools/call`、`resources/read` | 没有 | 页面有自己的后端，不需要。见下一节 |
| `ui/update-model-context` | 没有 | 没有模型上下文 |
| 媒体播放 | 有，不经 JSON-RPC | 页面用标准的 Media Session API 和 `<audio>`、`<video>`，插件不用为 App 改。iOS 由 WebKit 交给系统，App 声明后台音频。Android 的 WebView 不交给系统，所以注入脚本经 `wontonMedia` 通道把状态转给原生的媒体会话。有声音时开 `mediaPlayback` 前台服务，暂停后再留 60 秒盖住换曲的间隙，这段时间进程不被挂起、不被断网，隧道照常探测和重连（ADR-0032）；锁屏、通知栏和耳机的按钮调页面注册的 handler，拔耳机时暂停。插件页的媒体不要求用户点过才能播：换曲时页面在新的媒体元素上自动播放 |
| 其他原生能力（选文件、分享） | 没有 | ADR-0014 的 JS SDK 改由这张表扩展，每项单独过 4.7.2，见“审核” |

媒体播放的前台服务只管放声音，和 ADR-0003 的“保持连接”前台服务（#505）是两个服务：各自声明自己的类型、发自己的通知，谁也不启停谁。进程只要有一个前台服务在跑就不会被挂起，所以放音乐不依赖保持连接是否打开，关掉保持连接也不影响放音乐。

没有的方法回 JSON-RPC 错误，`hostCapabilities` 如实不声明，SDK 能据此判断。标准里没有的两件事（再点一次标签、连接状态）不进 JSON-RPC，用 `window` 上的 `wonton:*` 事件：SDK 校验严格，自定义消息容易被它拒绝，没用 SDK 的页面也能收到。

消息只在 Rust 里处理：壳把字符串交给核心，把回复注入回去，注入脚本也由核心给。两端不解析。
壳的通道、环境单位与调用顺序见 [客户端桥接入](../../crates/wonton-client/src/tabs/bridge/README.md)。

### 页面能拿到什么

和没固定时打开插件网页一样：插件自己的登录和数据，不多。

- 桥只给主题、语言、时区、栏高。不给主人身份（那是 `owner.identity` 的事）、设备信息、别的插件的东西。
- 页面碰不到核心的管理接口：网页发起的请求在核心被拒绝（ADR-0017、ADR-0024 的设备门）。
- **桥不能变成绕过设备门的路。** App 经隧道发请求时核心认作主人的设备。如果页面能让 App 代发，网页发起的请求就成了“主人亲自操作”。所以第一版没有 `tools/call`；以后要做（`ui://` 卡片需要）时，请求必须标明来自页面，只能调这个插件自己声明给页面用的工具（`visibility` 含 `app`），敏感工具照样走确认卡片，不算主人亲自操作。这条写成测试。
- 确认卡片的提示条照常盖在页面上（`PendingCardBanner`）。
- 未审核插件的页面和已审核的权限相同，靠商店和确认页的标签提示（ADR-0031）。固定时不再问，底栏上也不标，理由同 ADR-0031：是主人的选择。
- 草稿而不是直接发：网页内容对 Bot 是不可信输入（ADR-0011），用户看过再发。

### 跨源隔离

有的应用要跨源隔离才能用 `SharedArrayBuffer`，如 Actual Budget。iOS 的 WKWebView 看 COOP/COEP 响应头就给；Android WebView 没有站点进程隔离，一直不给（“验证”里 WebView 124 的实测）。WebView 156 起有官方开关 `Profile.setCrossOriginIsolatedAllowlist`：来源在允许列表里、响应带 `Document-Isolation-Policy: isolate-and-credentialless` 时才给。

- 插件在清单的 `webui` 上写 `cross_origin_isolated: true` 声明要它。没声明的插件不受影响。
- 核心给声明了的插件的网页响应加 `Document-Isolation-Policy: isolate-and-credentialless`。
- Android 壳只把声明了的插件的网页来源放进允许列表，不用 `*`。
- WebView 低于 156（`WebViewFeature.CROSS_ORIGIN_ISOLATED_ALLOWLIST` 不支持）时，打开前显示“这个应用在 Android App 里暂时打不开”，“详情”里写 WebView 版本，配“仍然打开”：用户能进去走应用自己的降级（Actual 有不用 `SharedArrayBuffer` 的模式）。不替应用打开降级。
- 详情页给声明了的插件加一条醒目提示，说它放宽了隔离。Agent 写的插件声明了这一项，安装和更新的确认卡片上也写明；从没声明变成声明算权限变宽，要发卡片。

### 状态

底栏项一直在，页面由 App 画状态，不露出 502，也不假装能用。状态来自核心的插件列表（`ViewModel`），壳按枚举显示文字。

| 情况 | 底栏 | 页面 |
| --- | --- | --- |
| 运行中，健康或健康未知 | 正常 | 加载 |
| 启动中、正在安装 | 正常 | 骨架加“正在启动…” |
| 升级中 | 正常 | 已加载的留着，顶部提示；升级完重新加载 |
| 已停止 | 图标变淡 | “xx 已停止”，按钮“启动” |
| 健康检查失败、异常 | 角标 | “xx 出了问题”，按钮“查看详情”（到详情页看原因）、“重试” |
| 新版本不再提供这个页面 | 角标 | “新版本不再提供这个页面”，按钮“从底栏移除” |
| 插件被卸载 | 移除，提示一次“xx 已卸载，已从底栏移除” | 正在看时回到“应用” |
| 页面返回 5xx、加载失败 | 正常 | “页面打不开”和状态码，按钮“重试”。错误页不用网页的，失败原因分清传输、插件未就绪和上游出错 |
| 插件没装好时冷启动（列表还没读到） | 用磁盘缓存（ADR-0032） | 骨架，等列表 |

“已停止”要求至少有一个常驻服务，且所有常驻容器都是 `exited` 或 `created`。部分运行、重启中、容器缺失、OOM 和运行中的健康失败仍是异常，保留原因。一次性服务不参与常驻服务统计，但它的失败不能被“已停止”掩盖。Docker 停止容器后可能保留旧的健康字段，不能用这个旧值把正常停止判成健康失败。

“启动”按依赖顺序启动原有常驻容器，不重建容器、不重跑已完成的一次性服务；已在运行时幂等返回。就绪检查失败要返回原因，不能提前报成功。不增加“停止”入口，也不向 Bot 暴露启停工具。操作沿用插件管理的设备门，插件网页不能代发。

在主屏点已固定的插件时切到它的标签，不开第二个 WebView。没固定的插件盖在主界面上打开，有顶栏，没有底栏，共用同一个容器（`inset` 布局），所以主题、状态和全屏只写一份。

### 和连接状态的配合（ADR-0032）

- 没连上时，页面不弹窗，也不加载：顶栏标题换成短状态（“连接中…”），和别的页面一样。已加载的页面留着，能看。
- 连上后，没加载成的页面自动加载；已加载的不刷新，让作者自己决定（`navigator.onLine` 看的是手机联网，不是家里的服务器，所以用 `wonton:connection` 事件）。
- 本地代理在隧道不通时不把 502 交给 WebView，壳认出来转成上面的状态。这和 ADR-0032 第 6 节是同一件事。
- 不做离线缓存：Service Worker 用不了（见上），在 Rust 代理里做 HTTP 缓存能做但复杂，等真有需求。

### 代码放哪

| 层 | 内容 |
| --- | --- |
| `wonton-protocol` | `InstalledPlugin.tabs`、`TabsViewModel`、底栏的 `Action`。选中哪一页和全屏退回到哪也在核心里算，壳只渲染。先单独提只改接口的 PR |
| `wonton-core` | 校验清单，栅格化图标，经插件列表给出。总线工具（`mcp/` 下单独一个模块）、`written_by`、Bot 的暂存、更新的判定 |
| `wonton-client` | `tabs.json`、上限和不变量、状态计算、注入脚本和令牌映射、桥的 JSON-RPC。行为测试：给定操作序列断言 `TabsViewModel` |
| iOS、Android 壳 | 按 `TabsViewModel` 渲染底栏；一个标签页容器（WebView 配置、骨架、顶栏、状态页）；排序页。估计每端各 300 到 500 行，比 `PluginWebView` 多的主要是容器 |

### Agent 写插件：从聊天到底栏

没有新的来源，也没有第二套安装：Agent 写的插件走 ADR-0031 的“上传”这条路，只是包从 Agent 的 outbox 来。

1. 主人在聊天里说想要什么。
2. Agent 调 `wonton__plugin_guide` 读说明，在 outbox 里写一个插件目录。
3. Agent 调 `wonton__check_plugin`，一次拿到全部问题，改到通过。这一步不打扰主人。
4. Agent 调 `wonton__install_plugin`，主人收到确认卡片，点允许。核心拉镜像、启动、探测网页。失败时回滚，错误里带容器日志，Agent 接着改（回到第 3 步）。
5. 卡片变成“已装好”，带“打开”和“放到底栏”两个按钮。
6. 主人看了说“标题太大了”。Agent 改目录，再调一次 `wonton__install_plugin`，这次是更新：页面自己刷新。

#### 工具

| 工具 | 敏感 | 参数 | 说明 |
| --- | --- | --- | --- |
| `wonton__plugin_guide` | 否 | | 这台核心的规范版本对应的说明和两个最小例子 |
| `wonton__check_plugin` | 否 | `dir` | 和 `wonton-core plugin check` 同一份代码，返回 `problems`、`warnings`、`files`、`digest`。不连 Docker，不联网 |
| `wonton__install_plugin` | 是 | `id` 或 `dir` | `id` 还是官方索引。`dir` 是 outbox 里的目录，清单的 `id` 没装过时是安装，装过且是这个 Bot 写的时是更新 |
| `wonton__plugin_status` | 否 | `id` | 状态、健康、网页探测的状态码。这个 Bot 写的插件还带每个服务最近的日志 |

**包经 outbox 交给核心，不放进工具参数。** 工具参数是模型一个字一个字生成的，把 tar 的 base64 写进参数既贵又容易错，还要另外考虑总线的请求上限。Agent 的容器没有 Docker，也没有 `wonton-core`（ADR-0011），打包和校验都由核心做。outbox 已经挂给 Hermes（ADR-0027），路径规则照用：相对路径，逐级不跟软链接，只收普通文件。和 `wonton__save_file` 不同的是，核心读完不删目录，Agent 接着在上面改。核心只取 compose 和它引用的文件，算摘要后暂存，所以 Agent 之后再改也影响不到正在确认的那一份。

**镜像。** 核心不构建（`build` 不在 Compose 子集里），Agent 也构建不了。所以写法只有一种：公开镜像加 `configs`，代码在插件包里，受插件包的大小限制。静态页用 `nginx`，带后端的用 `python` 等语言镜像。依赖尽量用标准库；要安装依赖时用一次性服务装进数据目录，并申请 `network.egress`。要自己发镜像的，是开发者在电脑上的路（模板的 `scripts/publish.sh`）。

#### 和 ADR-0031 对齐

- 复用 `preview_upload`、`install_source`、`preview_upgrade(UpgradeTo::Upload)`、`upgrade_source`，不另写。
- `trust` 是 `Local`，App 上显示未审核。`origin` 是 `None`，检查更新不会去问 GitHub。
- `provider` 不用：它的含义是能核实的代码位置，Bot 的名字核实不了。`InstalledRecord` 和 `InstallPreview` 新增 `written_by`，记 Bot 的身份，显示时写“你的 Agent · Hermes”。
- 暂存分开：App 的上传暂存只有 8 个，先进先出，Agent 改得快时会挤掉主人待确认的包。Bot 按（Bot，插件 id）留一份，新提交替换旧的，旧卡片更新成“已被新版本取代”。

#### 确认

| 操作 | 卡片 | 理由 |
| --- | --- | --- |
| 第一次装 | 每次发。写明未审核、你的 Agent · Hermes、全部权限和镜像，高权限醒目 | ADR-0016 不变 |
| 高权限（`network.lan`、`devices.gpu`） | 同上，能批 | 卡片就在主人的 App 里，点的是主人本人，和详情页的确认没有本质差别。这是主人的选择，提示风险，不在底层拦。更高的两级和 Bot 写的插件能申请什么，见 [ADR-0036](0036-plugin-privileges.md) |
| 更新，权限没变宽 | 不发，写审计，Agent 在聊天里说一声 | 卡片看不出代码改了什么，改一个颜色点一次只是在教主人闭眼点允许，这和 ADR-0024 的背景一样 |
| 更新，权限变宽 | 发，只列新增的项 | 和 ADR-0031 升级的权限确认相同 |
| 卸载 | 每次发 | ADR-0024 不变 |

**高权限的规则只写在这里：** 不管是官方插件（`id`）还是 Bot 自己写的（`dir`），都经卡片，主人点了就装（主人 2026-11-04 定）。

- “权限没变宽”指安装确认列出的项没有增加：`permissions`、`lan.ports`、GPU 的服务和共享文件夹的挂载。换镜像不算：镜像提不了权，代码本来就是 Agent 写的。
- 只有写这个插件的 Bot 能更新。`dir` 里的 `id` 已被官方、社区装的、别的 Bot 写的插件或系统插件占用时，直接报 `invalid_input`，不发卡片。这条写成测试。
- 主人在 App 里上传装的插件归主人，但 Agent 拿同一个 id 的包来更新时发卡片请主人点头，主人同意才接手：装上 Agent 交来的这份（数据保留，失败回滚），归属记成这个 Bot，以后的更新按上面的规矩走。摘要和装着的相同时也发这张卡——接手本来就要主人过目。放宽占用规则的理由：每次接手都要主人在卡片上点同意，没有绕过主人的静默路径；回绝或没理之后有 10 分钟冷却，同一个 Bot 对同一个插件不再刷卡（#817）。
- 第一次的卡片写明“以后 Hermes 改这个插件不再问，除非要更多权限”。插件详情页有“每次都问”，默认关，打开后每次更新都发卡片。
- 主人在 App 里用文件升级或卸载后，`written_by` 清掉，这个插件不再归 Agent；Agent 之后再拿包来更新，就按上一条经卡片接手。

#### 迭代

- 更新走 #237 的升级：数据保留，失败回滚。ADR-0031 允许版本号不变、摘要变了反复传，所以 Agent 不用改版本号。摘要没变时报“没有变化”。
- 更新要重建容器，页面这段时间回 503。App 留着已加载的页面，升级完才刷新，主人看到的是提示，不是错误页。说明和例子都要写 `init: true`：程序作为 1 号进程不响应 SIGTERM，每次更新都要等满 10 秒的停止时限（实测见“验证”）。
- App 在前台且已连接时，对当前插件页或覆盖层每 10 秒检查已安装列表，切入页面时立即检查；内容修订标识（`revision`）变化才自动重新加载已打开的页面，升级回滚后内容没变则不刷新，停止后启动仍按“状态”重新加载。
- 改坏时：`check_plugin` 不过，旧版不受影响；起不来就回滚，旧版继续跑，日志给 Agent；能跑但不对，主人说一声 Agent 再改。
- 核心不留历史版本。说明里教 Agent 在工作目录用 git，要退回就再提交一次旧版（Vibecode 的做法，放在 Agent 这边）。

#### 预览

- 不做单独的预览环境。Agent 这条路上，`check_plugin` 通过后装上就是预览：页面已经在“应用”里，放不放到底栏由主人决定。第二个环境意味着第二份网关、登录和隧道，不如真的装上。
- 电脑上有 Docker 的开发者（或电脑上的 Agent）用模板的 `scripts/dev.sh`，这是现成的。
- Agent 看不到页面渲染得怎么样，只有探测的状态码和日志。这一步靠主人看一眼，见“代价与风险”。

#### 开发套件

| 东西 | 放哪 | 内容 |
| --- | --- | --- |
| 说明 | wonton-docs 的 `skills/wonton-plugin/`，Agent Skills 格式；`llms.txt` 指向它和规范。核心镜像带一份，经 `wonton__plugin_guide` 给 | 从需求到装好的步骤、Compose 子集的禁区、常见报错怎么改。还要回答实测里 Agent 靠猜的几处（见“验证”）。随核心走，版本对得上，机器没外网也能用 |
| 最小例子 | 说明的 `assets/`，模板仓库同一份 | 静态页（nginx 加 configs）；带后端的页面（python 加 configs）。都带 `tabs` 和令牌样式表 |
| 校验 | `wonton__check_plugin`；`wonton-core plugin check`；模板的 `scripts/check.sh` | 同一份代码 |
| 引导语 | Hermes 插件仓库的 [wonton/__init__.py](https://github.com/geraldpeng6/wonton-plugin-hermes/blob/main/wonton/__init__.py) | 一句话：主人想要新页面或应用时，先调 `wonton__plugin_guide`。别的 Agent 插件同样写一句 |

Skills over MCP 等宿主支持了再加，内容是同一个目录。

#### 底栏

卡片上的“放到底栏”是壳的按钮，点了发 `Action::PinTab`，只改这台手机，不回核心。有多个页面时列出来逐个选。Bot 没有改底栏的工具：配置只在设备上，手机也可能不在线。

主人能自己定义的是：底栏放哪些页面、什么顺序，以及页面本身（说一句让 Agent 写）。聊天和“应用”留在栏上，那是回到其余一切的路。

### 审核

- **App Store 4.7：** 允许“不在二进制里的 HTML5 和 JavaScript 小程序”，开发者负责合规。4.7.2 不许向它们暴露原生平台接口：桥第一版只有主题、打开链接和草稿，不算。4.7.3 不许未经同意把数据或权限给它们：不给。**4.7.1（过滤、举报、拉黑）、4.7.4（要有索引和通用链接）、4.7.5（年龄）是风险**，见下。
- **2.5.2** 不许下载改变功能的代码，4.7 是它的例外，页面只在 WebKit 里跑。
- **Google Play：** 不许下载 dex、JAR、`.so`，“JavaScript in a webview” 明确不受限。
- **4.2（最低功能）：** App 有原生聊天和管理，不算套壳网站。

这一篇没有增加新的审核面：商店（ADR-0017、ADR-0031）和插件网页已经在了。底栏只是让它更显眼。Agent 写的页面也是网页，只在主人自己的服务器上，商店里没有。

### 验收

验收是一条端到端冒烟：真实模型的 Hermes 收到一句需求，写出插件，主人批准，页面能用；再改一次，数据还在，页面自己刷新。

## 理由

- **网页打底：** 只有它能放进现成应用，插件作者也不用学新东西。Home Assistant 和 Telegram 走到同一个结论。
- **栏和主题归 App：** “感觉不原生”主要来自白屏、两套顶栏和颜色不跟，这些都能在壳里解决，作者不用写代码。不能解决的是页面本身的控件，那取决于作者，令牌和样板给他们一条容易的路。
- **用 MCP Apps 的词汇，不用它的加载方式：** 规范的加载方式解决的是聊天里的第三方卡片，要求内联 HTML、数据全走工具；底栏页要长驻、要看大图和视频，放在自己的域名上更合适。词汇（方法名、`hostContext`、CSS 变量）照搬：作者能用官方 SDK，以后 `ui://` 卡片共用一套，也不用再设计一种协议（不造轮子）。
- **状态在 Rust：** 底栏顺序、上限和桥的回复都能写行为测试，两端不会各说各话（ADR-0005）。
- **Agent 写插件复用现有的路：** 包的校验、暂存、确认页、升级回滚都是现成的，要加的只有入口（outbox）和判定（谁写的）。新东西越少，规范和说明越短，Agent 越容易写对。

## 没选的方案

- **只用 MCP Apps：** 放不了现成应用，规范也没有不经工具调用的打开方式。看图看视频要经工具和 Base64 或另开 URL，不如直接是网页。
- **声明式原生：** 感觉最好，但只够列表和表单，RSS 能做，相册做不了。组件库和两端渲染器是长期成本，也违背“不造轮子”。A2UI 成熟后可以作为 `ui://` 之外的一种卡片再评估。
- **自己的小程序框架（微信的路子）：** 体验可以很好，代价是一整套语言、工具和生态，和“优先用成熟开源组件”相反。
- **系统的 `TabView` 和 `UITabBarController`：** ADR-0029 选了自摆，这里不动。
- **“更多”标签：** “应用”页已经是全部插件的入口，再加一个“更多”是两个同样的列表。
- **固定时要页面先就绪：** 插件暂时没起来就不能固定，用户会以为固定失败。状态放到页面里说清楚。
- **`tabs` 必须声明才能固定：** 主人否了：这样接入已有网页的第三方应用就慢了。代价是图标不统一。
- **替现成应用注入 `viewport-fit=cover`：** 实测没好处，还会让不知情的页面躲不开 Home 条。
- **`env(safe-area-inset-*)` 作为给页面的接口：** iOS 只有 `cover` 时才有值，Android WebView 140 以下不可靠。
- **全屏时顶部那一条用 `theme-color`：** 只是一个颜色，常和页面顶栏对不上（主人在 Navidrome 上看到一整块浅蓝）。
- **全屏时取网页贴边的像素，拉伸铺满留白：** 每秒取一次，页面换颜色时那一条约 1 秒后才跟上，中间是一条错色，主人 2026-11-06 否了。
- **胶囊常驻，为它让出一行：** 主人否了。页面右上角常放头像和菜单，所以胶囊改成会自己藏、藏起来不挡点按。
- **按页面的滚动距离决定胶囊显示：** 页面里的容器滚动、单页和 canvas 都要另想办法；看手指一条规则全包了。
- **胶囊按页面的像素换深浅：** 主人要透明、不换。
- **带栏时也去掉标题栏，用同一个胶囊：** 做了原型，主人 2026-11-06 看了说带栏时保留标题栏，胶囊只用在全屏。
- **带栏时网页铺到底栏后面（iOS 26 用 `obscuredContentInsets` 让出底栏）：** 做了原型，主人 2026-11-06 否了：现成应用自己的底部按钮栏会和我们的底栏叠在一起，透过去反而出问题。Android 的 WebView 也没有对应的接口（Chromium `android_webview/docs/insets.md`：只认真的和它重叠的系统栏、挖孔和键盘）。
- **往页面里注 CSS 让出栏：** 页面的布局不归我们，改了可能坏得更隐蔽。
- **工具参数传 tar 的 base64：** 上面说过，模型要把它逐字写出来。
- **给 Agent 开 Docker socket，让它自己 `docker compose up`：** 绕过核心的校验和确认，也违反 ADR-0011。
- **核心替插件构建镜像：** 要在核心里放一条构建链，占磁盘，构建的代码也不受 Compose 子集约束。公开镜像加 `configs` 够写页面和小后端。
- **Agent 写的插件算单独的一种来源：** 除了显示的文字，和上传没有差别，规则也都一样。
- **Bot 直接改底栏：** 配置只在设备上。要做就得把配置搬到服务器，多设备和离线都变麻烦，换来的只是省一次点按。

## 代价与风险

- **现成应用不会变原生。** Immich 这样的页面有自己的顶栏和底栏，放进来是两套，截图上和 Muse Bot 差得远。只有适配过的页面才接近。这是通用机制的代价，主人看过对比图后接受了；两套底栏的问题靠全屏解决。
- **全屏时看不到底栏。** 要切页面得先退出。打开就全屏的页面点关闭回到进来之前的标签，这条路要一眼能看懂。
- **`inset` 的页面全屏时不是真的画到屏幕边。** 状态栏那一条是 App 底色，和页面的顶栏不连；iOS 26 以前和 Android 的底边也停在 Home 条（导航栏）上面。要真正铺满，作者写 `edge-to-edge`。
- **胶囊出现时盖住页面右上角一小块。** 最多 3 秒，用户碰页面就能让它藏起来。
- **页面里的控件还是网页。** 长按菜单、文字选择、列表惯性有差别。作者用得好可以很接近，但不会和原生一样。
- **内存。** 最多 3 个固定的插件 WebView，未固定页另用 overlay。Android 上 WebView 可能共享渲染进程，一个崩了影响全部。模拟器的三页主进程采样见“Android 标签页容器”，不能代替低端真机评估。如果低端机扛不住，退路是只留最近用的一到两个，其余按地址恢复。
- **App Store。** 4.7.1 和 4.7.4、4.7.5 针对的是“在 App 里提供软件”的开发者。自托管、主人自己装的情形没有明确说法。社区插件进底栏后，审核员更容易看到这一点。提审前单独评估，必要时提审版本只显示官方索引。这不是这一篇引入的，但这一篇让它更显眼。
- **规范在动。** MCP Apps 还是扩展，SDK 已到 2.x，草案在继续改。注入脚本依赖 SDK 用 `event.source` 校验来源，键名校验也很严（见“验证”），SDK 升级可能打破，要有一条跟着新版 SDK 跑的测试。CSS 变量是最稳的接口，模板也优先教这个。
- **Agent 能悄悄改已批准的插件。** 不发卡片的更新，让被提示注入的 Bot 能把自己写的插件改成别的样子，在已批准的权限内做事，比如把它能读的共享文件夹发出去（如果也批了 `network.egress`）。范围只限它写的插件和批过的权限，每次写审计，详情页能打开“每次都问”。主人接受了这个取舍。
- **Agent 写的代码没人审。** 只标未审核，不做扫描。没有 `network.egress` 的插件出不了内部网络，这是最实在的限制，所以说明要教 Agent 只申请用得到的权限。
- **Agent 看不到渲染结果。** 页面只有主人看得到。网页探测和日志能发现起不来，发现不了布局错。以后可以让 Agent 经浏览器插件看自己的页面，要先想好网关登录怎么给。
- **跨源隔离放宽了 Android 上的安全保障。** 允许列表里的页面和别的网页可能在同一个渲染进程里，能用 `SharedArrayBuffer` 和高精度计时器读同进程的内存（Spectre 一类）。只对声明了的插件开，详情页和卡片上写明，主人知道这个代价。
- **公开接口变多：** CSS 变量、`tabs` 字段和 `wonton:*` 事件都是作者依赖的，以后不能随便改。

## 已定

主人 2026-11-04 定了：

- **渲染路线：** 网页打底，栏和主题由 App 管。
- **设置：** 默认在底栏，可以拿掉。
- **没适配的现成应用也能固定：** 按原样显示。看过对比图后定的，理由是这样能很快接入已经有网页的第三方应用。
- **全屏：** 同时要加，参考微信小程序的胶囊，解决两行底栏。
- **Agent 更新自己写的插件：** 权限没变宽就不问，变宽才发卡片。详情页能打开“每次都问”。

主人 2026-11-06 定了：

- **Android 上的跨源隔离：** 声明了的插件在 WebView 156 及以上放宽隔离，更低的版本如实提示并给“仍然打开”。知道代价是安全保障变弱。
- **栏和胶囊（看了两版原型的录屏）：** 带栏时保留标题栏，网页限在标题栏和底栏之间；全屏时用会自动隐藏的胶囊，往下拉出现、往上推藏起、3 秒淡出；全屏时状态栏那一条是 App 底色。

## 验证

### iOS 文字缩放

iOS 26.5 模拟器、402pt 宽、Debug 壳，真实代理与隧道；Notes、手写 JSON-RPC 探针和 Actual Server 26.11.0 欢迎页。
`pageZoom` 在 `extra_large` 下让视口变成约 301 CSS px、文档仍宽 402 CSS px，出现横向滚动。
保持 `pageZoom = 1`，壳注入根元素 `-webkit-text-size-adjust`，标准 → 特大 → 标准时：

| 页面 | 标准 / 特大的标题字号（CSS px） | 两档视口 / 文档宽（CSS px） |
| --- | --- | --- |
| Notes | 28 / 37.33 | 402 / 402 |
| JSON-RPC 探针 | 28 / 37.33 | 402 / 402 |
| Actual 欢迎页 | 40 / 53.33 | 402 / 402 |

切换期间每页只有首次打开的一次 `didFinish`；探针的 `loads` 保持 1，文字重新换行，栏高变量保持原 CSS px。
Notes 的“加上”“删掉”不再需要横向滚动。Actual 欢迎页的固定底部内容在特大字号下与表单纵向重叠，
这项验证只证明文字缩放与横向宽度，不证明 Actual 全部页面的大字号可用性；壳没有改上游布局。

### 原型（不合并）

iOS 26.5 模拟器，`@modelcontextprotocol/ext-apps` 2.0.4，本机服务器让页面晚 3 秒返回，模拟慢的隧道。没有走真实隧道和网关，也不是真机。Mac 当时负载很高，这里不记流畅度。

| 问题 | 结果 |
| --- | --- |
| 不处理时深色模式下白不白 | 白：加载期间整块白屏，页面的颜色也不跟 App，重点色的按钮看不见 |
| 不处理时能不能用官方 SDK | 不能：`ProtocolError: Method not found`，就是 SDK 把自己的请求当成对方的请求 |
| 不透明关掉、盖骨架、文档开始前注入 | 加载期间是骨架，没有白屏；页面第一屏就是深色 |
| 官方 SDK 经注入脚本连到原生 | 通：`app.connect()` 成功，拿到 `theme`、`platform`、`safeAreaInsets` |
| `styles.variables` 带 `--wonton-*` | SDK 拒绝，握手失败：`Unrecognized keys`。去掉后正常，只给部分标准键可以 |
| 改重点色 | 页面当场变，没有重新加载（全程只有一次 `didFinish`），SDK 收到 `host-context-changed` |
| `ui/message` | SDK 的 `sendMessage` 到达原生，切到聊天，文字成为草稿 |
| 切标签再回来 | 同一个 WebView，没有重新加载 |
| 内容从栏下面滚过 | `edge-to-edge` 的页面按 `--wonton-inset-*` 留白，内容穿过胶囊的材质 |
| 没适配的 Immich（`inset`，官方镜像 v3.3.5） | 能放，登录态由原型的转发代替网关的单点登录。深色和浅色都跟（它自己写了 `prefers-color-scheme`），重点色不跟。App 的顶栏下面是 Immich 自己的顶栏，两层标题；底栏不挡内容，也没有内容从栏下滚过 |

这个 iOS 原型没有验证：深色切浅色的当场变化（启动时选浅色或深色的路径相同，但没有实际切过）、3 个 WebView 的内存、`ui/open-link`。Android 容器见下一节；全屏另列。

### Android 标签页容器

验证条件：API 35 x86_64 模拟器、390×844、Android System WebView 124.0.6367.219、targetSdk 36、
Debug 壳加 release Rust 核心，真实配对后经本地代理、iroh 隧道和 Docker 网关访问插件；不是物理手机。

- App 浅色→深色→浅色和更换重点色，探针的文档 UUID 与 `loads=1` 不变，`prefers-color-scheme` 和桥上下文同步变化。
  页面没有 `color-scheme` meta。`ContextThemeWrapper` 随 Compose 外观更新，不靠 Activity 换主题或重载网页。
  配置了 `DARK_STRATEGY_WEB_THEME_DARKENING_ONLY`；targetSdk ≥33 的 WebView 忽略旧 force-dark API，
  同时关闭算法反色，不能仅凭调用成功宣称旧策略生效。
- Jellyfin 官方镜像 `sha256:fea863d0cd9ec445985d9c75fc6ed64e38a1d2766cf0c250a33d553f4fc1b771`
  的初始向导在 App 明暗切换时保留自己的深色内容，文字可读；根元素虽收到 App 的颜色，未观察到内容被强行反色。
  这里只覆盖初始向导，不代表全部播放器页面。Actual 官方镜像
  `sha256:06cf186569cf58d93e12659ff1a517b65b5b0ebee7050a3991d4bd8a1d398da6` 报 `SharedArrayBuffer`
  不可用，未用它宣称主题兼容。
- 手写 JSON-RPC 页面完成握手并撤骨架；`ui/message` 进入聊天只填草稿，`ui/open-link` 打开系统浏览器。
  切回保留滚动位置，重选回顶部；`inset` 留在栏内，`edge-to-edge` 收到顶部 112 CSS px、底部 104 CSS px。
- 主框架 HTTP 503 显示原生错误页，状态码放在折叠详情；重试恢复。断连保留已加载内容并通知 `connected=false`，
  恢复后同一文档收到 `true`。主动终止 WebView 渲染进程后 App PID 不变，容器重建并恢复地址。
- 三个已加载探针页常驻时，`adb shell dumpsys meminfo app.wonton.android` 报告 `WebViews: 3`、
  主进程 PSS 227983 KiB、RSS 376256 KiB。数字含 Rust/Matrix 和调试壳，不含独立 WebView 渲染进程，
  不是三张任意复杂网页的内存上限。
- 停止页与淡化图标用临时 Android 调试夹具输入 `TabState::Stopped` / `dimmed=true` 验证，按钮调用真实
  Rust 核心。`c5206ad` 的 `StartPlugin` 返回 `NotReady`，原始原因完整显示。夹具不进入交付构建；
  这不是容器成功启动的端到端证明。

### iOS 全屏壳

iOS 26.5 模拟器、402pt 宽、标准字号、Debug 壳，经真实隧道访问 Actual Server 26.11.0 欢迎页和手写 JSON-RPC 探针。
顶栏手动进入与胶囊关闭恢复带栏、状态栏与底栏隐藏、胶囊菜单打开插件详情已验证。
Actual 的顶部色带来自 WebKit `themeColor`；探针 `edge-to-edge` 时顶部变量为 118 CSS px，底部为 34 CSS px。
桥 `ui/request-display-mode` 的 inline/fullscreen 往返有效，同一文档 `loads` 不变，只有 context 计数增加。

同一条件下，用 Actual 欢迎页和 `edge-to-edge` 探针补验：

| 场景 | 结果 |
| --- | --- |
| 用户的“总是全屏” | 两种页面打开、关闭开关均生效；自动全屏关闭返回先前的聊天标签，手动全屏关闭回到同一页的带栏样式 |
| 作者声明全屏、用户关闭 | 两种页面首次自动全屏；关闭“总是全屏”后再次打开是带栏样式，用户选择覆盖清单 |
| 固定与未固定页面 | 固定探针往返保持 `loads=1`、同一 `performance.timeOrigin`；未固定自动全屏探针关闭后再打开，`timeOrigin` 改变，证明是新文档 |
| 横屏 | 探针宽度与文档宽均为 874 CSS px，顶部变量 56px、底部 20px；关闭后恢复竖屏 402px，`loads=1`。Maestro 旋转后的可访问性树漏掉底部元素，恢复以截图和文档尺寸为证 |
| 键盘 | 网页输入框可输入，键盘弹出时胶囊可操作，关闭能恢复带栏样式 |
| 页面视频全屏 | WebKit 系统播放器可进入，下滑关闭后回到同一全屏网页，`loads=1`、`timeOrigin` 不变；播放器控件隐藏时 Maestro 的按钮断言未通过，不算按钮自动化通过 |
| 确认卡片 | 经真实 MCP 总线申请卸载测试插件，全屏页显示提示条；点后进入 Wonton 房间，取消后核心返回“主人拒绝了，没有执行” |
| 设备被拒绝 | 只撤销该模拟器的配对：全屏页显示原因和“重新配对”，胶囊可用，不显示原始 502；重新配对并重启 App 后网页恢复 |
| 临时断连自动恢复 | 全屏停在 Edge，只 `docker stop wonton-adr33-dev`：显示连接短状态，胶囊菜单可操作，无原始 502；`docker start` 后等核心 healthy，连接提示自动消失，同一网页 `loads=1`，随后成功加载 `/second`。未重新配对、未重启 App，未操作其他容器 |

顶栏改用 Phosphor 2.1.1 的 `corners-out`（语义名 `expand`）后，Actual Fullscreen 长标题与按钮分离；图标仍以“全屏打开”作为无障碍标签，点击进入和胶囊关闭通过模拟器实测。

客户端基线 `7c1ab2b6`（含 #398、#399）、iOS-sim Debug 壳接入 `TabLoadSucceeded`，与核心 `f43c04f5` 联调，补验结果：

| 场景 | 结果 |
| --- | --- |
| 最终图标回归 | Actual Fullscreen 长标题和 Edge 的图标入口均能进入全屏、关闭恢复；Edge 同一文档 `loads=1` |
| 503 后返回历史 | `/` → `/second` → `/fail` 显示原生错误页；原生返回回到 `/`，失败状态清掉，`loads=1`、`timeOrigin=1793886909880` 均不变 |
| 首次导航就是 503 | 直接打开错误标签页显示错误和重试，没有历史返回入口；进入全屏再退出仍保留错误页，不显示上游响应体 |
| 停止与启动 | 只停止 `app.a7.fullscreen` 的 `web` 容器；刷新应用列表后 Edge 图标变淡，带栏与全屏均显示“已停止”和“启动”。全屏里点启动后容器 healthy，页面恢复、图标恢复，`timeOrigin` 改变，证明文档重新加载 |

外部 `docker stop` 不是推送来的：页面开着时客户端每 10 秒查一次。App `ab100a31`（含 #403）、核心 `470c9404` 上实测：停留在 Edge 页不操作，20 秒内显示“已停止”和“启动”，点启动后文档重新加载。同一条件下真实模型 Hermes 更新 Notes，停留在该页不操作，不到 30 秒显示新内容，数据还在，审计 `decision: allowed`、没有卡片。

客户端再基于 `0d550c50`（含 #402）重建并安装后，与同一核心复跑图标全屏往返、503 后历史恢复和首次 503 保留状态页，全部通过；历史恢复仍为 `loads=1`。

#### 边缘手势实测（负责人做）

条件：iOS 26.5 模拟器 fileapp-iphone，A7 工作区的构建，Edge 探针页（edge-to-edge，已固定）。输入走模拟器的系统边缘手势（从屏幕左缘 4pt 内起手的触摸路径），不是真机。

| 操作 | 结果 |
| --- | --- |
| 全屏，网页无历史，左缘右滑 | 退出全屏，回到带栏的样子，同一文档（`loads=1`，`origin` 不变，`mode=inline`） |
| 全屏，进过 `/second` 有历史，左缘右滑（10 个采样点，约 150ms） | WKWebView 后退到 `/`，仍是全屏，文档从历史恢复（`loads=1`，`origin` 回到原值） |
| 同上，但用 0.5 秒的慢速滑动 | 没有后退，也没有退出。只出现一次，没有查原因，可能是模拟器注入的速度 |
| 全屏，页面内横滑（起手 x=300，非边缘） | 轮播换到第二张，不退出全屏；再滑回第一张 |
| 全屏，无历史，从轮播所在高度的左缘右滑 | 退出全屏，不是滑动轮播 |

没验证：真机的手感，Immich 查看器。

### 全屏（原型，不合并）

同样的条件。[Actual Budget](https://github.com/actualbudget/actual) 26.11.0 的手机版自带底栏，写了 `viewport-fit=cover` 和 `theme-color`；Immich v3.3.5 没有写这两项。

| 问题 | 结果 |
| --- | --- |
| 带栏时 | Actual 两行底栏，主人说的问题 |
| 网页框留在安全区内（选的做法） | Actual 只剩自己的底栏，在 Home 条上方，没有多留一遍。胶囊在顶部那一条里，没盖住 Immich 右上角的头像，也没盖住 Immich 查看器的工具栏 |
| 网页框铺满，不注入 | 底栏同样在 Home 条上方。Actual 按自己的 `cover` 把顶部多推了一截 |
| 网页框铺满，注入 `cover` | 底栏位置没有变化，没有好处 |
| 顶部那一条取 `theme-color` | Actual 深色和浅色都和页面的顶栏连成一片；Immich 没写，用 `canvas`，也不突兀 |
| 展开按钮进、胶囊的关闭退 | 都生效，页面不重新加载，还停在同一张照片 |

原型未覆盖的作者默认全屏、用户覆盖、横屏、键盘和视频，由上面的“iOS 全屏壳”实测补充。
Android 全屏不在这次 iOS 验证范围内。

### Android 全屏

验证条件：API 35 x86_64 模拟器、390×844、Android System WebView 124.0.6367.219、targetSdk 36，
Debug 壳和 release Rust 核心，经真实配对、本地代理、iroh 隧道与 Docker 网关访问插件。

- `inset` 全屏网页框为 `[0,56]–[390,820]`；`edge-to-edge` 为 `[0,0]–[390,844]`，
  `--wonton-inset-top=56px`、`--wonton-inset-bottom=24px`。状态栏隐藏，胶囊保持原生层，底部保留系统导航区。
- 顶栏与底栏长按菜单进入、胶囊关闭、用户“总是全屏”、清单 `display: fullscreen` 和用户关闭覆盖均已操作。
  手动关闭回带栏页面，打开即全屏的关闭回进入前标签；探针 UUID、`loads=1` 和 `performance.timeOrigin` 不变。
  默认页面的 `ui/request-display-mode` 可以进入和退出，用户选择标准模式后网页请求不能覆盖该选择。
- `theme-color` 从 `#204060` 变到 `#663399` 时顶部条同步，移除 meta 后回到 `canvas`，文档未重载。
  横屏和键盘弹出时胶囊可见，输入保留；这不是物理手机刘海与手势导航的验证。
- 确认卡片横幅显示在网页之上、胶囊之下；断连时顶栏换成“连接中…”，页面保留，恢复后同一文档收到
  连接状态，全程不露 502。
- Swing Music 官方镜像（自带手机底栏）全屏进入退出不重载，它的底栏贴近 Home 条，没有和 App 底栏
  叠成两层；Jellyfin 12.4 手机网页用顶栏加抽屉，没有底栏，不能代替这项验收。Actual 在此 WebView 上
  报 `SharedArrayBuffer` 不可用，未能用它验收：COOP/COEP 响应头经直连与隧道都完整到达，同一镜像的
  Chrome 能建立跨源隔离而 WebView 124 不能，是平台限制而非代理或网关问题。
- 停止与启动走真实核心：docker 停掉插件容器后核心把页面转 `Stopped`，打开显示“xx 已停止”和“启动”，
  点启动后容器恢复健康、页面重新加载；固定到底栏的图标在停止时按 0.4 alpha 变淡（截图像素核对）。
- **网页 5xx 走壳的失败状态页**：插件自返 503（带 HTML body 或空 body）都显示“xx 出了问题”类状态页和
  重试，上游响应体不露出。WebView 的 `onReceivedHttpError` 早于 `onPageStarted` 到达（实测日志），页面开始
  回调曾把失败标记清掉再报成功；壳现在按失败 URL 保留失败标记，重试同一 URL 成功、从失败页返回历史、
  首次导航即 503 三种路径都实测恢复。网关错误路径（停插件、断连）同页显示。
- 网页视频全屏：DevTools 以用户手势调 `video.requestFullscreen()` 后 `onShowCustomView` 托管视频视图，
  胶囊浮在视频上；返回键先退视频（页面保持全屏）、再按走网页历史，胶囊关闭回带栏。真实手指未验证。

### 点输入框时放大（iOS）

iOS 26.5 模拟器 iPhone 17 Pro，2026-11-06：小费插件点“账单金额”输入框（#442），原来页面放大约 1.4 倍、收起键盘不缩回；改后不放大，双指放大、缩小照常。

### 栏和全屏胶囊（iOS）

iOS 26.5 模拟器 iPhone Air（灵动岛），开发栈配探针插件（长页、内部滚动、单页，`inset`），2026-11-07。

| 情形 | 结果 |
| --- | --- |
| 底栏里的长页 | 有标题栏、没有胶囊；网页框在标题栏和底栏之间，滚到底最后一段完整停在底栏上沿，底栏周围是 App 底色 |
| 底栏里的内部滚动页 | 同上，底栏后面不露页面的颜色 |
| 标题栏的“全屏打开” | 进全屏，状态栏照常显示，胶囊出现、3 秒后藏起；网页铺到屏幕底 |
| 全屏时往下拖一下、点退出全屏 | 胶囊出来，退出后回到带栏，同一位置，不重新加载 |
| 应用页打开单页（overlay） | 标题栏上有全屏打开和关闭，点关闭回到应用页 |

胶囊藏起时点按穿透、往上推藏起、菜单、读屏时一直显示，用的是原型阶段同一套代码，原型录屏里已经验证过，这次没有重测。

### Android 的三键导航

三键导航横屏、浅色主题、深色页面时，右侧的三个键是深色的：App 已经请求浅色图标（SystemUI 也记着），但 API 35 的 Launcher 任务栏按自己的主题重画按钮颜色（Launcher3 `NavbarButtonsViewController`）。公开 API 管不到，不为它加不透明的底条。

### Agent 能不能一次写对

AI 编程代理，只给公开规范（v0.4）和模板仓库，不能联网，也没有 Docker，和机器上的 Agent 处境相同。需求是主人的一句话：“我想在 App 里有个动态页，看我订阅的 RSS。能加订阅、删订阅，点开文章能看。”校验和安装用本地构建的核心（2026-11-04，`wonton-core:dev`），它没有校验 `tabs`，也没有走 MCP 工具（工具还不存在），由人代为调用同样的校验和升级接口。这只是一个模型、一个需求的一次结果。

| 问题 | 结果 |
| --- | --- |
| 写出来了吗 | 约 5 分钟：`python:3.13-alpine` 加 `configs`，`server.py` 318 行只用标准库，`index.html` 297 行 |
| `plugin check` | 第一次通过，只提醒镜像没按摘要固定 |
| 装得上吗 | 第一次装上，3 秒后 `running healthy`（镜像已在本地） |
| 功能 | 加订阅、文章列表、正文都能用。读 GitHub 的订阅源偶尔超时（15 秒），这台主机直连 GitHub 不稳，开发环境也没配出网代理，不算插件的问题。手机尺寸的深色下页面可用，没有明显的布局问题 |
| 写错时的报错 | `ports`、`build`、错字段、缺文件、错权限，都有一句话说清原因，错字段还列出可选的。能照着改 |
| 改一次（升级接口，版本号不变） | 成功，页面变了，订阅还在。12 秒，全程 503 |
| 加 `init: true` 后 | 同样的更新 2 到 4 秒。之前的 10 秒是在等 1 号进程响应 SIGTERM。第一次写的 Agent 不知道这一点 |
| 改坏（程序启动就退出） | 12.6 秒后回滚，旧版继续跑。错误是 `unhealthy`，带 Python 的完整报错，够 Agent 自己改 |

它靠猜的地方，说明要回答：

- `user` 写 `${PUID}:${PGID}` 时，`configs` 的属主是插值前还是插值后算的。
- 健康检查用 `python3 -c` 时代理怎么处理（规范只写了 wget 和 shell）。
- `sso: none` 又没有自带登录的页面，是否就靠网关。
- `network.egress` 能访问局域网，用户填的地址要不要拦。

### 实现时要验证

- 两端模拟器，浅色和深色，默认和别的重点色，Immich（现成应用）和 RSS（适配过的）各走一遍；切明暗时页面不重载。
- 全屏：Actual（自带底栏）和适配过的 `edge-to-edge` 页面，两端走一遍上面“没有验证到”的几项。
- 断网、插件停止、卸载、升级，页面状态符合上表，全程不露 502。
- 打开 3 个页面后的内存，两端低端设备各测一次，结果写在这里。
- Android WebView 124 / API 35 的主题验证见“Android 标签页容器”。旧 force-dark API 与 targetSdk ≥33 的行为不同，其他 WebView 版本和低端真机仍需验证。

## 参考

- MCP Apps：[规范 2026-01-26](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/2026-01-26/apps.mdx)、[概述](https://modelcontextprotocol.io/docs/extensions/apps)、[客户端支持](https://modelcontextprotocol.io/extensions/client-matrix)、[SDK 的 PostMessageTransport](https://github.com/modelcontextprotocol/ext-apps/blob/main/src/message-transport.ts)
- [OpenAI Apps SDK 的 ChatGPT UI](https://developers.openai.com/apps-sdk/build/chatgpt-ui)
- [Telegram Mini Apps](https://core.telegram.org/bots/webapps)、[附件和侧边菜单](https://core.telegram.org/api/bots/attach)
- Home Assistant：[App 配置](https://contributors.home-assistant.io/apps/reference/configuration/)、[外部总线](https://contributors.home-assistant.io/frontend/reference/external-bus/)、[原生导航路线图 #244](https://github.com/OpenHomeFoundation/roadmap/issues/244)
- [微信小程序全局配置](https://developers.weixin.qq.com/miniprogram/dev/reference/configuration/app.html#tabBar)
- [Slack App Home](https://docs.slack.dev/surfaces/app-home/)、[Block Kit](https://docs.slack.dev/block-kit/)、[Discord Activities](https://discord.com/developers/docs/activities/how-activities-work)
- [Muse Bot](https://github.com/0216zerotwo/muse-bot)、[Meta Muse 界面介绍](https://smartscope.blog/en/blog/meta-muse-screen-guide-features-main-chat-feed-2026/)
- [App Store 审核指南](https://developer.apple.com/app-store/review/guidelines/)（2.5.2、4.2、4.7）、[Google Play 设备和网络滥用政策](https://support.google.com/googleplay/android-developer/answer/9888379)
- WebKit：[App-Bound Domains](https://webkit.org/blog/10882/app-bound-domains/)、[Service Worker 的限制](https://github.com/mdn/browser-compat-data/pull/30146)、[Safari 26 的 obscuredContentInsets](https://webkit.org/blog/17333/webkit-features-in-safari-26-0/)
- Android：[WebView 的深色](https://developer.android.com/develop/ui/views/layout/webapps/dark-theme)、[WebViewCompat](https://developer.android.com/reference/androidx/webkit/WebViewCompat)、[处理渲染进程结束](https://developer.android.com/develop/ui/views/layout/webapps/handle-termination)
- 全屏：[Telegram 的 `requestFullscreen` 和两种安全区](https://core.telegram.org/bots/webapps)、[微信小程序的 `navigationStyle`](https://developers.weixin.qq.com/miniprogram/dev/reference/configuration/app.html)、[胶囊的位置](https://developers.weixin.qq.com/miniprogram/dev/api/ui/menu/wx.getMenuButtonBoundingClientRect.html)、[Capacitor 关于 WebView 140 的说明](https://capawesome.io/blog/capacitor-edge-to-edge-and-safe-areas-guide/)、[WKWebView 里 `env()` 和 `viewport-fit` 的关系（社区对 WebKit 的解读）](https://github.com/ilkyeong-gn/learn-safe-area/issues/3)
- [A2UI](https://a2ui.org/introduction/what-is-a2ui/)、[A2UI Compose](https://developer.android.com/jetpack/androidx/releases/a2ui-compose)、[Adaptive Cards 渲染](https://learn.microsoft.com/en-us/adaptive-cards/rendering-cards/getting-started)

## 关联

ADR-0005（厚核心）、ADR-0006（插件系统）、ADR-0011（安全模型）、ADR-0014（JS SDK）、ADR-0016、ADR-0017、ADR-0024（设备门）、ADR-0026、ADR-0028、ADR-0029（外观）、ADR-0027（outbox）、ADR-0031（来源）、ADR-0032（连接状态）。
