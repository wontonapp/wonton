# ADR-0043 App 的多语言：文字放在各平台的本地化资源里，核心只给类别

- 状态：已接受（2026-11-06）
- 取代：[ADR-0028](0028-settings-page.md) 里外观选项的文案由核心列出；[ADR-0017](0017-plugin-ecosystem-and-store.md) 里插件文字挑语言的规则

## 背景

App 的界面文字写死成中文：iOS 和 Android 的壳里各约 600 处，客户端核心（`wonton-client`、`wonton-protocol`）里约 420 处，其中大半是错误原因（显示在“详情”里）和日志，小半直接显示（引导的标题、核心升级的说明、外观选项的名字）。家里的服务器给 App 的文字（插件文字、权限说明、确认卡片）有的已经按 `Accept-Language` 挑中文或英文，有的只有中文。

主人 2026-11-06 定了：

1. 支持简体中文、English、繁體中文、日本語。繁中按台湾和香港用户的用词写，不是简体逐字转换。以后加语言只加翻译文件。
2. 表述按各语言 App 的习惯写，不逐句直译。
3. 默认跟随系统语言，也能改。按平台惯例：iOS 跳到系统设置里本 App 的语言页；Android 13 起有系统的“应用语言”，App 里也放一个选择器，旧系统用 AppCompat 的官方做法。

AGENTS.md 已经定了分工：错误和状态的类别由核心给，文字按类别写在壳里。

## 决策

### 语言和回退

| 语言 | 标签 | 说明 |
| --- | --- | --- |
| English | `en` | 资源的基础语言。系统语言不在这四种里时用它 |
| 简体中文 | `zh-Hans` | 中国大陆、新加坡 |
| 繁體中文 | `zh-Hant` | 台湾、香港、澳门，用台湾的词 |
| 日本語 | `ja` | |

系统语言在四种里怎么对应由平台自己挑：iOS 按 App 的本地化，Android 按资源限定符（`values-b+zh+Hans` 这类带文字系统的写法），两端都把 `zh-TW`、`zh-HK`、`zh-MO` 归到繁中，`zh-CN`、`zh-SG` 归到简中。

App 名字：中文是“云吞”和“雲吞”，英文和日文是“Wonton”。

### 文字放在哪里

**壳：所有固定的界面文字。** 按钮、标题、空页面、按类别写的问题文字、通知文字、系统权限说明、App 名字。

- iOS 用 String Catalog：`Localizable.xcstrings` 放界面文字，`InfoPlist.xcstrings` 放 `CFBundleDisplayName` 和 `NS…UsageDescription`。打开 Xcode 的“生成 String Catalog 符号”（`STRING_CATALOG_GENERATE_SYMBOLS`），代码里写 `Text(.settingsLanguage)`、`String(localized: .settingsLanguage)`，键写错编译不过。
- Android 用资源文件：界面在 `shared/src/androidMain/res/values*/strings.xml`，App 名字和推送服务的文字在 `androidApp` 的资源里。代码里写 `stringResource(R.string.settings_language)`、`pluralStringResource`，Composable 外用 `Context.getString`。
- 不用 Compose Multiplatform 的资源：界面因为 UniFFI 绑定依赖 JNA 只能放在 `androidMain`（见 `shared/build.gradle.kts`），用不上它的跨平台好处，系统的应用语言和 Android Lint 的检查又是对 Android 资源做的。桌面端（M2）把界面搬进 `commonMain` 时一起换，两种资源都是同样的 `strings.xml` 写法。

**客户端核心：界面上的文字一律不出，只出“详情”。** ViewModel 给枚举、类别和原始值（数目、字节数、时间戳、名字），壳按它们挑文字和格式化。

- 已经在核心里写好的文字改成枚举：外观选项的 `label`、引导的步骤标题和提示、核心升级的说明、文件操作和聊天操作的“某某失败”、推送通知的正文（图片和文件的占位、邀请、不知道发送者）。这些是接口改动，按 AGENTS.md 先单独提改 `wonton-protocol` 的 PR。
- 默认收起的“详情”（`CoreError` 的原因、问题里的 `detail` 和 `message`）也按 App 的语言写。它们在核心深处的各条路径上生成，带着状态码、超时秒数这些参数，有几百种，一一变成枚举交给壳写不现实，所以由客户端核心用 Fluent 写：`crates/wonton-client/locales/<语言标签>/*.ftl`，语言取 `SetLanguages` 的第一项，也就是 App 实际在用的语言。
- 数据原样嵌在句子里，不翻：错误码、HTTP 状态、仓库名、镜像名、上游返回的原文。翻的是包在它们外面的说明。
- 连接失败的可读说明与原始诊断分开：`PairingStatus.Failed.message`、`HomeStatus.Disconnected.message` 和 `ConnectionDetails.last_failure` 按错误类别写本地化原因，不嵌入底层错误；配对失败和连接详情的可选 `diagnostic` 保留原始错误及其错误码，配对诊断也保留直连和中继地址，不含配对密钥。壳只在用户主动打开独立诊断入口后显示，并允许复制。配对的实际总耗时与每个交换阶段的限时分别换算为秒，不用一个“限时”暗示它是整次操作的预算。
- App 切换语言后，仍显示的配对失败说明根据保留的失败类别和计时事实重新生成；原始诊断不随翻译改写。配对进行中切换语言时，结果使用完成时的 App 语言。
- 配对码格式不合法或服务端拒绝时按已知失败类别本地化原因；解析器包含输入值的错误只记安全的字段和可确认不含敏感数据的原始原因，其余遮去。服务端原始拒绝说明仅在主动打开诊断后提供；诊断和日志不得含完整配对码、配对密钥或中继令牌，包括中继令牌的合法百分号编码形式。
- 日志写英文，不翻译：只有开发者看。
- 别人写的内容照原样传：聊天消息、Bot 的回复、插件和家里服务器挑好语言的文字。

**家里的服务器：只有它知道、旧 App 也要显示得出来的文字。** 插件清单里的文字、权限说明和提醒、引导的 Agent 名单和服务商、帮助文档，以及核心以自己的账号发的确认卡片和消息。这些按请求的 `Accept-Language`（在聊天里发的按主人的语言，按人取见下）挑语言。确认卡片发给谁就按谁最近一次请求的语言写（没有记录时按主人的）：家人的卡片推到他的设备、显示在他的 App 里，文字也该是他读得懂的。

- 核心自己的文字用 Fluent：`crates/wonton-core/locales/<语言标签>/*.ftl`，每种语言按领域分几个文件，构建时拼成一份（几批迁移同时加文字时不在同一个文件里冲突）。复数和数字用 Fluent 的选择器和 `NUMBER()`。现在散在代码里的 `("zh", …), ("en", …)` 对照搬进这些文件。
- 挑语言用 `fluent-langneg` 的协商（打开 `cldr` 特性，取 `Lookup` 策略），插件文字也换成它：按 CLDR 的可能子标签补全文字系统，所以 `zh-TW`、`zh-HK` 在有 `zh-Hant` 时选它，而不是排在前面的 `zh`。不开 `cldr` 时它没有中文的子标签，所有中文都落到列表里排第一的那种。
- 服务器给 App 的错误原因也按 `Accept-Language` 用这些文件写，同样只翻说明、不翻数据。
- 主机终端（`wonton pair` 等）和局域网状态页不在本篇范围。

### 键的写法

- 键按含义起名，两端用同一组名字：iOS 写 `settings.language`，Android 写 `settings_language`。前缀是界面或功能（`chat.`、`store.`、`problem.transport.`），多处共用的放 `common.`。同样的意思在两端说法不同时（“轻点”和“点按”），各写各的，键名照旧对得上。
- 文字按功能分文件、按前缀放：一个键落在哪个文件由它前缀决定（分组表在 `scripts/ui_text/groups.py`，`check-ui-text.py` 检查），前缀装不进 800 行就再拆细。iOS 的 common 组留在默认表 `Localizable.xcstrings`，其余表的生成符号多一层表名（`.Chat.chatReply`）；Fluent 在 `locales/<语言标签>/` 下按组分文件，`build.rs` 按语言拼成一份（同 `crates/wonton-core`）。
- 一条文字是一个完整的句子或短语，不在代码里拼。参数用位置占位符，每条都写给译者的注释：显示在哪里、参数是什么、有没有长度限制。
- 基础语言是英文：iOS 的 `sourceLanguage` 和 `CFBundleDevelopmentRegion` 是 `en`，Android 的 `values/` 是英文。四种语言缺一条，检查就失败，不靠运行时回退补洞。

### 语言设置

设置首页在“外观”“底栏”那一组加一行“语言”（`SettingsViewModel` 里的一项），右边显示当前的选择，名字和点了以后做什么由壳决定：iOS 是 App 在用的语言的自称；Android 是“跟随系统”或那种语言的自称。

- **iOS：** 点了打开 `UIApplication.openSettingsURLString`，也就是系统设置里本 App 的页面，系统在那里给“语言”。App 里不做选择器。系统改语言后会重启 App，这是系统的行为。
- **Android：** 点了打开 App 里的选择器：第一项“跟随系统”，下面是支持的语言，各用它自己的语言写名字（English、简体中文、繁體中文、日本語），选中即生效。选择器调 `AppCompatDelegate.setApplicationLocales`：Android 13 起它交给系统的 `LocaleManager`，和系统设置里的“应用语言”是同一个值；更早的系统由 AppCompat 存下、启动时套用。Android 12 及以前要 `AppCompatActivity` 才套用得上，所以 `MainActivity` 改成 `AppCompatActivity`，主题的父主题换成 AppCompat 的，清单里加 `AppLocalesMetadataHolderService`（`autoStoreLocales`）。
- 系统设置里的“应用语言”要 `locales_config`：AGP 的 `generateLocaleConfig` 只读 App 模块自己的资源，shared 库里的语言会漏掉，所以照 Mihon 的做法由 `androidApp/build.gradle.kts` 里的 `generateLocalesConfig` 任务生成，列出 App 和 shared 里每个带 `strings.xml` 的 `values-*` 目录（`values/` 是英文）。App 里的选择器也从它读语言列表。

### 语言怎么传给核心和插件页

- 壳在启动和语言变了以后发 `SetLanguages`：第一项是 App 实际在用的语言，后面接系统的语言偏好。iOS 取 `Bundle.main.preferredLocalizations.first`，不取 `Locale.preferredLanguages`：后者是用户要的语言，系统是法语时它是 `fr-FR`，App 显示的却是英文；Android 取每种语言的资源里写着自己标签的一条字符串，这样拿到的是资源实际落到的语言，而不是用户要的语言。
- 客户端核心把它写进每个请求的 `Accept-Language`。家里的服务器把**每个人**最近一次设备请求的语言记在这个人名下（`core/plugins/store/people-languages.json`，键是用户名），语言按人给，不只记主人的。插件按人取用走两个标准接口：OIDC 的 `locale` 声明（登录者本人，要 `profile` scope，原样的 BCP 47 标签）和核心 MCP 的 `wonton__user_locale` 工具（`locale` 和按偏好排的 `languages`；没有记录时是 `null` 和空数组，核心不替插件选默认语言）。Agent 经 MCP 调用、核心在聊天里发卡片时没有请求头，就用主人最近一次的语言；主人几台设备用不同语言时，以最近用的那台为准。
- 插件网页：WebView 自己的 `Accept-Language` 和 `navigator.language` 是用户要的语言，单独设了本 App 的语言就是它，没设就是系统的语言列表，插件有这种语言时比 App 用得更贴切；ADR-0033 的桥在 `hostContext.locale` 里给 App 实际在用的语言。
- 插件清单的多语言文字照旧至少写 `zh` 和 `en`，`zh` 是简体；可以加 `zh-Hant`、`ja`。没写的语言按上面的协商回退：繁中落到 `zh`，日文落到 `en`。插件规范在 wonton-docs 里同步改。

### 复数、日期和数字

- 核心不格式化给人看的数字、大小、时间。壳用平台的格式化器，取 App 当前的语言：iOS 用 `FormatStyle`（`Date.FormatStyle`、`Date.RelativeFormatStyle`、`ByteCountFormatStyle`、`ListFormatStyle`）；Android 用 `android.icu.text` 和 `java.time.format` 的本地化格式、`Formatter.formatShortFileSize`，语言取 `LocalConfiguration.current.locales[0]`。
- 列举（“A、B 或 key”）用列表格式化器，不在代码里写顿号和逗号。
- 复数用 String Catalog 的复数变体和 Android 的 `<plurals>`。中文和日文只有一种形式，英文的单复数照样要写。

### 术语和翻译

- 术语表在 `docs/glossary.md`：Wonton 自己的叫法（插件、商店、配对、家里的服务器、出网代理……）在四种语言里怎么说，和各语言的写法规矩（英文的大小写 iOS 和 Android 各按平台；日文的句子用敬体、按钮和标题用简短的名词或动词；繁中用台湾的词，如“伺服器”“設定”“裝置”）。参照各语言系统自带 App 和当地主流 App 的说法。
- 每批文字先出初稿，再用另一个模型按“像不像这个语言的原生 App”做母语审校。不写文案断言的测试。

### 以后加一种语言

1. 术语表加一列。
2. iOS 在两个 String Catalog 的每份表里加这种语言；Android 在每个 `values-<限定符>/` 下按同样的文件名各加一份；客户端核心和服务器在 `locales/<语言标签>/` 下各加一套 `.ftl`。
3. 代码不用改：Android 选择器的列表从 `locales_config` 读，设置行显示的语言名由平台按标签给出。

### 防止再写死

- `scripts/check-ui-text.py`，进 CI 和 `check.sh`：
  - 在 iOS 和 Android 的源码（不含测试）和 `wonton-client`、`wonton-protocol`（不含测试）里，含中文、日文假名或全角标点的字符串字面量一律报错，注释不算。
  - 两端的源码里，直接传给界面接口的字符串字面量像界面文字时也报错：`Text`、`Button`、`Label`、`navigationTitle`、`contentDescription` 这类系统接口，设计系统组件（名单从 `design/`、`Design/` 目录和 `Wonton` 前缀读出）的第一个参数，以及 `title`、`message`、`label`、`detail` 这类文字参数。迁移前按语言二选一的 `TabText.choose`、`tabText` 一律报错。
  - 四种语言的键要一样，同一条的占位符个数和类型要一样，没有翻译的条目报错。
  - 例外写进 `.ui-text-allowlist`，格式和 `.size-allowlist` 一样：`路径 | 原因 | 到期日 YYYY-MM-DD`。迁移期间现有文件都列进去，每抽完一批删掉对应的行。
- 客户端核心和服务器的 `.ftl` 由 Rust 测试检查：每种语言的消息一样，变量一样。

## 理由

**为什么不由核心统一出文字。** 之前算过一个方案：一份 Fluent 目录放在客户端核心，生成两端的访问函数，两端相同的约 220 条只写一处，切语言当场生效。主人定了语言设置按平台惯例走，这个方案的主要好处就没了：iOS 改语言系统会重启 App，Android 改语言会重建界面，不需要当场切换。剩下的都是平台的东西做得更好：

- 系统的应用语言、回退和复数规则直接可用，不用让核心去猜 iOS 选了哪种语言。
- 通知、权限说明、App 名字本来就只能走平台的资源，核心的目录只能管一半。
- Xcode 和 Android Studio 的编辑、检查和机器翻译都认这两种格式，键写错编译不过。

代价是两端各维护一份，相同的约 220 条写两遍。用同一组键名、同一份术语表和同一个检查脚本压住走样。

**参照的做法。** Android 的选择器照 Mihon：列表从 `locales_config` 读，第一项跟随系统，语言名用它自己的语言写，经 `AppCompatDelegate.setApplicationLocales` 生效。iOS 照 Ice Cubes：设置里一行跳到系统设置，String Catalog 用按含义起名的键、基础语言是英文。

**依赖。** `androidx.appcompat`（Apache-2.0）；`fluent-bundle`、`fluent-langneg`（Apache-2.0 或 MIT），核心和服务器都用。

**为什么服务器自己出一部分文字。** 新的权限、新的卡片是服务器升级带来的，App 不一定同时升级。服务器挑好语言给成品，旧 App 也显示得出来；在聊天里发的卡片存在 Matrix 里，所有设备看到的是同一条。这部分本来就按 `Accept-Language` 挑，本篇只把它从代码里的对照表换成翻译文件。

**为什么详情也翻译。** 主人 2026-11-06 定：用户点开详情时看到的也应该是自己的语言。要原样发给开发者的那部分（错误码、仓库名、上游原文）本来就是数据，嵌在句子里不变。

**为什么是 Fluent。** Mozilla 为 Firefox 做的本地化格式，Rust 实现（`fluent-bundle`、`fluent-langneg`）由 Project Fluent 维护。一条消息里能按数目、性别选不同说法，参数有名字，译者看得懂；一种语言一个文件，加语言只加文件。比起自己定一个 TOML 表加 `format!`，它多了复数和选择器；比起 gettext，它不以原文当键，原文改了译文不会全部失效。`fluent-langneg` 按 CLDR 补全文字系统，`zh-TW` 和 `zh-HK` 能挑到 `zh-Hant`，自己写的前缀匹配做不到这一点。

**为什么用 AppCompat。** Android 13 起系统自己管每个 App 的语言；更早的系统没有这个能力，Google 给的官方做法就是 `AppCompatDelegate.setApplicationLocales`，它在 13 及以上直接转给系统的 `LocaleManager`，两边是同一个值。自己存语言、启动时改 `Configuration` 也能做（Thunderbird 就是这样），但要自己处理每个窗口、服务和 WebView 重设语言的坑。代价是 `MainActivity` 要换成 `AppCompatActivity`、主题换成 AppCompat 的，界面是 Compose 画的，不受影响。

**为什么基础语言是英文。** 系统语言不在支持范围里的用户看英文；拿英文当基础，缺翻译时也不会冒出一句中文。

## 代价

- 繁中和日文的用户看插件文字时，插件没写这种语言就是简体或英文，要插件作者补。
- Android 多了 AppCompat 依赖，`MainActivity` 和主题要改成 AppCompat 的。
- 核心和服务器各有一份 Fluent 目录，加语言时要一起加。
- 主人几台设备用不同语言时，服务器在聊天里发的文字用最近那台的语言。

## 待验证

- iOS 的系统设置里出现本 App 的“语言”，`openSettingsURLString` 打开的是本 App 的页面（探针 App 没有权限说明，打开的是设置首页）。
- Android 系统语言是 `zh-HK`、`zh-MO`、`zh-SG` 时落到对的那种中文。
- Android 10 到 12 上用 AppCompat 改语言：第一次创建 WebView 后界面不退回系统语言；推送服务里取的文字也跟着 App 的语言。
- Android WebView 的 `Accept-Language` 和 `navigator.language` 跟着 App 的语言。
- `generateLocaleConfig` 读得到 KMP 库模块里的资源目录。

## 验证结果

iOS 26.5 模拟器、Xcode 27.1，用一个 XcodeGen 生成的探针 App（`developmentLanguage: en`，两个 String Catalog，四种语言）：

- `STRING_CATALOG_GENERATE_SYMBOLS: YES` 写在 target 的设置里就生效。生成的是 `LocalizedStringResource` 的静态属性和函数，参数没有标签（`.greeting("Ann")`、复数条目 `.pluginCount(3)`）；`Text(...)` 和 `String(localized:)` 都能用。
- XcodeGen 从 String Catalog 读出 `knownRegions`，构建出的 App 里有 `en`、`ja`、`zh-Hans`、`zh-Hant` 四个 `.lproj`，`CFBundleDevelopmentRegion` 是 `en`。
- 用 `-AppleLanguages` 换系统语言：`zh-HK`、`zh-TW`、`zh-MO` 落到 `zh-Hant`，`zh-CN`、`zh-SG` 落到 `zh-Hans`，`fr-FR` 落到 `en`。
- 在 App 的偏好里写 `AppleLanguages`（系统设置里单独设本 App 语言时写的就是它）后，`Bundle.main.preferredLocalizations`、`Locale.preferredLanguages`、`Locale.current` 和 WKWebView 的 `Accept-Language`、`navigator.languages` 都跟着变。
- 没单独设时，WKWebView 的 `Accept-Language` 是系统的语言列表：系统是 `fr-FR` 时它是 `fr-FR,fr;q=0.9`，App 显示英文。
