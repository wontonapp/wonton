# ADR-0128 大清单一处定义：权限表先行，HostKind、Action、界面文字随后

- 状态：已接受（2026-11-27，主人在早会 T7 选 A：先做 ADR 和权限；当天协调定了遗留的三项，见“已定”）。第一期只实现权限表；
  HostKind、Action、界面文字写到“怎么做”，各自另提实现
- 补充：[ADR-0036](0036-plugin-privileges.md) 的风险分级、[ADR-0057](0057-optional-permissions.md) 的可选权限名单和
  注入变量，改为在权限表里各写一栏，分级和名单本身不变

## 背景

有几张清单几乎每个功能都要改，每加一项都要在好几处手工改，还得彼此对得上。2026-11-27 的公理设计分析把它们列为
“谁都要改”的内核词表：

| 清单 | 规模 | 同一项要改的地方 |
| --- | --- | --- |
| 插件权限 `Permission` | 24 种 | 协议里的枚举、解析、显示；核心的“已兑现”、风险级别（两份，确认页和清单限制各一份）、“高权限”、能否可选、注入变量名、说明和提示的消息键 |
| 网关目标 `HostKind` | 14 类 | 内部别名常量、枚举、`classify` 的分支、网关的分发、按权限给插件挂别名（`core_aliases`） |
| 客户端操作 `Action` | 19 个领域、188 个变体 | 领域子枚举、核心的分发；受 UniFFI 单个枚举 32768 字节元数据的上限约束（[ADR-0106](0106-action-domain-enums.md)） |
| 界面文字 | 四种语言 | iOS 的 `.xcstrings` 和 Android 的 `strings.xml` 各写一份同样的键和译文（[ADR-0043](0043-app-languages.md)），客户端核心和服务器另有 Fluent 目录 |

以权限为例：确认页的风险级别（`views_permissions::permission_risk`）和清单限制用的级别（`privileges::risk`）是两份手写的
match，“高权限”（`PermissionExt::is_high`）是第三份名单。三份现在一致，全靠人对齐。

## 决策

### 1. 每张清单只在一处定义，别处生成或查表

“一处”是一张表：每一项一行，一行写齐这一项在各处要用的属性。用到这些属性的代码读表或由表生成，不再各写一份 match。
表外只留每项各不相同、写不成数据的部分（比如带参数的权限怎么解析参数），并且也挂在这一行上。

按收益排顺序：权限 → HostKind → Action → 界面文字。

### 2. Rust 里的清单用声明宏生成；跨语言的产物用生成脚本

| 做法 | 用在哪 | 理由 |
| --- | --- | --- |
| 声明宏（`macro_rules!`），宏和表放在定义枚举的那个模块 | 权限、HostKind | 枚举和它的解析、显示、属性表在同一次编译里从同一份输入展开，漏写一栏编译不过；不加构建步骤、不加依赖，报错指到表里的那一行 |
| 独立生成脚本，产物提交进仓库，CI 重新生成后比对 | 界面文字 | 产物是 Swift、Kotlin 的资源文件，`build.rs` 只能管 Rust 的构建；Xcode 和 Gradle 要直接读到文件，评审也要看得到译文的改动 |

没选的：

- **`build.rs` 读数据文件（TOML、YAML）生成 Rust 代码。** 多一种格式和一个生成器，生成的代码在 `OUT_DIR` 里，读代码和跳转都要绕；
  报错指到生成的文件而不是数据行。对纯 Rust 的清单没有声明宏做不到的事。
- **过程宏或 `strum` 这类 derive 库。** 过程宏要单独一个 crate；`strum` 能生成名字和遍历，但风险、可选、消息键这些栏还得另写，
  仍然是两处。
- **手写枚举，旁边放一个按种类穷举的 `match` 当表。** 编译器能保证每种都有一行，但解析和显示的写法仍是各自的 match，
  要遍历全部种类时还得再列一份名单。

### 3. 权限表（本期实现）

表在 `crates/wonton-protocol/src/manifest/permission.rs`，宏和行类型在同目录的 `permission_table.rs`。每行写：

| 栏 | 含义 |
| --- | --- |
| 变体和字段 | `Permission` 的变体；带参数的写字段，如 `StorageShare { name, access }` |
| 写法 | 清单里的写法。带参数的是模板，占位符就是字段名：`storage.share.{name}:{access}`；显示用它当格式串，占位符和字段对不上编译不过 |
| `parse` | 只有带参数的行有：前缀和解析剩余部分的函数，外加一个合法的例子 |
| `risk` | 普通、高、很高、最高，或“按 capability 名字定”（`cap.*`，名单外按最高） |
| `optional` | 能否声明成可选、默认关（ADR-0057） |
| `env` | 可选项开着时注入的变量名 |
| `text`、`notice` | 说明和风险提示在核心 Fluent 目录里的键；提示可以是“实际级别是最高级时才写”的那句共用提示 |
| `supported` | 核心能不能兑现；规范先定义、核心还没做的写 `false`，安装时如实拒绝 |

宏生成 `Permission`、不带参数的 `PermissionKind`、按种类顺序的 `PERMISSIONS`，以及解析、显示、种类和参数（参数同时是 Fluent 变量）。
`Permission::risk()`、`is_high()`、`notice()` 在协议里按表算；核心的“已兑现”、可选名单、注入变量、确认页的说明和提示都直接读表，
原来的 `CapabilityRisk`、`privileges::risk`、`may_be_optional`、`permission_var`、`PermissionExt` 删掉。风险级别和
`capability_risk` 随表搬进协议，因为它们和写法一样是规范的一部分，核心和 App 看到的应当是同一份。

约束：

- 行的顺序就是 `Permission` 的 `Ord` 顺序，`optional_permissions` 这类 `BTreeMap` 按它遍历，不随手调整。
- 向前兼容不变：不认识的写法报 `UnknownPermission`；App 拿到不认识的风险级别落到 `PermissionRisk::Other`。
- 测试：表里每一行的例子都能解析、种类和参数对得上、显示和 serde 往返一致；说明和提示的键在四种语言里都有。
- 只有 `docker.socket`、`devices.gpu`、`devices.serial`、`devices.usb` 能可选（ADR-0057 的名单）。`devices.disk` 标不能可选，
  可选名单的校验对它单独说“硬盘不能可选”这句。
- 规范里给插件作者看的权限表（公开仓库 wonton-docs）以后从这张表生成，见“已定”。

### 4. HostKind（怎么做，本期不实现）

在 `crates/wonton-core/src/config/hosts.rs` 用同样的声明宏，每类目标一行：

- 变体；
- 怎么认：插件网络里的固定别名（`health.wonton.internal` 这类）、公网下的子域（`auth`、`matrix`）、或者要计算的
  （IP 地址、插件 slug、清单声明的能力），计算的那几类仍手写，表里标出来；
- 固定别名的主机名常量；
- 谁能拿到这个别名：某种权限（直接引用 `PermissionKind`，如 `plugins.health` → 健康状态），或者 SSO、网页、模型能力、只给核心私网
  （推送、文件钩子）。

生成枚举、主机名常量、`classify` 里按固定别名的分支、`core_aliases` 里按权限挂别名的那部分。网关的分发仍是穷举的 `match`：各目标的
处理函数参数不同，统一成一个签名得不偿失，穷举已经能让编译器提醒新加的目标。

### 5. Action（怎么做，本期不实现）

Action 不做成数据表。每个变体本来就只定义一次（领域子枚举里），Swift、Kotlin 的绑定由 UniFFI 从它生成；把 188 个带字段、带文档的
变体搬进宏，只会让 UniFFI 元数据和文档更难查，不会少写一处。约束是 [ADR-0106](0106-action-domain-enums.md) 的元数据上限，
它已经由 `scripts/check-uniffi-enum-sizes.sh` 按符号名检查，不需要名单。

要做的是让“改一个变体”在编译期就找全所有要跟着改的地方：

- 核心里每个领域的正式分发函数穷举匹配，不写 `_ =>`，在这些函数上打 `#[deny(clippy::wildcard_enum_match_arm)]`；
  `ui-fake` 的夹具有意只接一部分操作，不在此列。
- 顶层 `Action` 的领域列表就是枚举本身，加领域仍按 ADR-0106 的规矩。

### 6. 界面文字（怎么做，本期不实现，不改 ADR-0043）

ADR-0043 定的运行时格式不变：iOS 读 String Catalog 和生成的符号，Android 读 `strings.xml` 和 `R.string`，两个 Rust 核心读 Fluent。
变的只是这些文件从哪来：

- 壳的文字每组只写一份源，由 `scripts/` 下的生成脚本写出 `.xcstrings` 和四种语言的 `strings.xml`；产物照旧提交，CI 重新生成后比对，
  有差别就失败。键名、分组和按前缀落文件的规则沿用 `scripts/ui_text/groups.py`，完整性检查（`check-ui-text.py` 的规则三）改为检查源。
- 占位符和复数在生成时转换：源里写命名参数，iOS 写成带序号的 `%1$@`，Android 写成 `%1$s`；复数转成 String Catalog 的变体和
  Android 的 `<plurals>`。源里出现转换不了的写法，生成脚本报错停下，不猜。
- 服务器的 Fluent 目录（`crates/wonton-core/locales`）是另一批文字（只有服务器知道的），本身就只有一份，不并进来。
- 源用 Fluent，见“已定”。

### 7. 改这几张清单要带 ADR

增删变体、改写法或语义（风险级别、能否可选、谁能拿到别名、操作归哪个领域）必须带 ADR，写进 [AGENTS.md](../../AGENTS.md)。
新能力优先加数据行（清单字段、能力名、插件自己的工具和页面），不改这几个内核枚举。只改说明文字的措辞不算改语义。

## 理由

- 表里一行就是一项的全部事实，评审一眼能看出“这种权限是什么级别、能不能可选、开着注入什么”，不用在五个文件之间对。
- 风险级别原来手写了两份、“高权限”又是第三份，合成一栏以后不可能再不一致。
- 声明宏不引入依赖和构建步骤，Agent 改起来和改普通 Rust 一样，出错时编译器指到表里的那一行。
- 界面文字的产物是别的语言的资源，只有生成脚本能同时写给 Xcode 和 Gradle；运行时格式不动，ADR-0043 的检查和平台工具链照旧用。

## 代价与风险

- 声明宏的报错比普通代码难读，宏本身也要有人维护；表的格式（每行固定的几栏）改起来要动宏。宏的说明写在宏上面。
- 权限的风险级别从核心搬进协议，协议 crate 多知道一点“策略”；App 仍只拿 ViewModel 里的 `PermissionRisk`，不直接用这张表。
- 界面文字生成以后，直接改 `.xcstrings`、`strings.xml` 会被下一次生成覆盖；CI 的比对挡住这种改法，Xcode 的可视化编辑也不能再用。

## 已定（11-27）

协调会话 2026-11-27 定：

1. **界面文字的源用 Fluent。** 客户端核心和服务器已经在用，有现成的一致性检查（`wonton-l10n::check_catalog`），复数和选择器
   表达力够；生成脚本只接受能转成 String Catalog 变体和 `<plurals>` 的子集，其余报错停下。没选每组一个 TOML 或 JSON 目录：
   更简单，但要另写一套检查。
2. **规范里的权限表从这张表生成，后续做。** 规范在公开仓库 wonton-docs；下一次改权限规范时，由核心出一个按 `PERMISSIONS`
   打印 Markdown 表的命令，生成的表在那边提 PR。之前照旧手写。
3. **`devices.disk` 在表里标“不能可选”。** 原来标“可以可选”，只为了让设备校验报出“硬盘不能可选”这句更具体的话；这句话挪到
   可选名单的校验里。声明了硬盘槽位的清单报的仍是这句；没写硬盘槽位却把 `devices.disk` 写成可选的清单，原来能通过校验，
   现在一样报这句。

## 关联

- [ADR-0015](0015-small-components-standard-interfaces.md)：组件只经标准接口连接；本篇管的是组件内部的大清单
- [ADR-0036](0036-plugin-privileges.md)、[ADR-0057](0057-optional-permissions.md)：权限的分级和可选名单，本篇只改它们写在哪
- [ADR-0043](0043-app-languages.md)：界面文字的分工和运行时格式，本篇不改
- [ADR-0106](0106-action-domain-enums.md)：Action 的领域子枚举和 UniFFI 元数据上限
