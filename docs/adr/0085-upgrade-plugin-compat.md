# ADR-0085 核心升级前列出会失效的插件：目标镜像自证的兼容检查

- 状态：已接受（2026-11-12）

## 背景

ADR-0025 的升级页只说核心自己的事：版本号、会不会迁移数据、回滚。已装插件不在里面。新核心可能读不懂已装插件（#233）：

- 清单的 `spec` 写得比新核心支持的高（`store_info.rs` 的 `SPEC_VERSION`）；
- 清单用了新核心不认识的字段或权限（`Manifest` 按表解析、`deny_unknown_fields`）；
- `requires`/`wants`/`provides` 的能力名从新核心的能力表里没了（ADR-0053）；
- 清单引用的注入变量新核心不再注入，渲染不过。

现在这些要等升级后新核心恢复插件时才暴露：记录解析不了就标异常（`manager.rs` 的 `restore` → `Entry::broken` →
`Degraded`），主人只能回滚整台核心。Nextcloud 的 `occ upgrade` 直接停用不兼容应用、提前检查被关成不做，用户反复踩
（#233 引用的两个 issue），我们不能走到那一步。

底子已经有两样：

- 已装插件的记录里存着整份插件包（compose 原文和引用文件，`InstalledRecord`），当前核心不启动容器就能重新解析它；
- 跨版本问目标镜像有先例：镜像标签（`wonton.core.migrations`、`wonton.core.components`）和目标镜像里的只读子命令
  （`wonton-core compose-images`，升级器跑在目标镜像里，ADR-0025）。

## 决策

### 已定

主人 2026-11-12 验收定的三条（原 PR 待定项，都按 PR 里的建议）：compat 查不成时一键升级照常可发起；确认页只列
"新失效"的插件，既有异常的不重复列；原因类别维持四个。

### 目标镜像用子命令说清自己的支持范围

两个候选：

- **镜像标签**（如 `wonton.core.spec`、`wonton.core.capabilities`）：读起来最便宜，镜像 inspect 就行，版本号和迁移
  标签就是这么读的。但标签只能装下规范版本号和能力名列表；权限表、注入变量、compose 渲染规则装不下。拿着标签，当前
  核心只能拿自己的旧规则去猜新核心认不认，"字段或权限不认识"这类猜不出来。标签和解析器是两处声明，要人肉保持一致，
  会漂。
- **目标镜像里的只读子命令**：目标核心自己的解析和渲染代码裁决，答案和升级后新核心的实际行为同源，不存在两处声明。
  代价是查的时候跑一个一次性容器。

选子命令。新子命令 `wonton-core plugin compat <插件包目录>…`，JSON 写标准输出，一次说清三件事：这份核心支持的规范
上限（`spec_supported`）、它认识的能力表（`capabilities`，ADR-0053）、逐个插件包的裁决。不另加镜像标签。

这不是启动新核心：一次性容器 `--network none`、只读挂载、用完即删；镜像已经拉到本机，之后就要整体运行它，信任面和
升级器跑 `compose-images`、`init` 一样。

### compat 的契约（缝一：目标镜像 → 兼容裁决）

```text
wonton-core plugin compat <插件包目录>…     # 每个目录一份完整插件包
标准输出：
  {"spec_supported":"0.12",
   "capabilities":["models","egress_proxy","torrent_client","jellyfin"],
   "plugins":[
    {"id":"app.immich.photos","verdict":"ok"},
    {"id":"x.y.z","verdict":"breaking",
     "reasons":[{"category":"unknown_permission","raw":"files.home"}]}]}
退出码：0＝裁决完成（有失效的插件也是 0）；1＝命令自身出错；2＝用法不对
```

- `category` 四个，字符串固定：`spec_unsupported`（清单 `spec` 超过 `spec_supported`）、`unknown_feature`（字段或
  权限不认识）、`capability_missing`（能力名不在目标的能力表里，也不是合法的 `x-` 第三方名字）、`other`（渲染不过：
  引用了不再注入的变量、运行时不支持等）。
- `raw` 是清单里的原值：`spec` 的值、字段路径（如 `x-wonton.tabs`）、权限名、能力名、变量名。JSON 里没有人话：类别
  文字由壳按类别写（ADR-0043：核心只给类别和原始值），`raw` 原样进详情。
- 插件包解析不了也是一条裁决（`breaking`），不当命令失败：一个坏包不挡别的包。
- 检查范围和安装同一套：解析清单（含未知字段拒绝）、`Manifest::validate`（权限表、能力表、spec 版本）、`dry_run`
  渲染。不含 `plugin check` 的打包质量部分（图标、截图、健康检查提醒）——那些和核心版本无关；也不含主机要求
  （`requirements`）——主机不随核心版本变。

### 当前核心怎么拿：下载完成后、确认页之前跑一次

- 时机：查新版本下载好镜像、读出版本号和迁移标签之后（`upgrade/check.rs` 读标签那一步）跑 compat。结果跟着 digest
  走；digest 变了重跑（发起时 digest 不符本来就回 409 `changed`）。
- 当前核心把每份已装记录物化到 `<数据目录>/core/upgrade/compat/<digest>/`（记录里本来就有 compose 原文和引用文件；
  数据目录容器内外路径相同，ADR-0025 的挂载约定；放 `upgrade/` 下顺带被备份排除），挂进一次性目标镜像容器跑 compat，
  读 JSON，删目录。一个容器查所有包，每次检查开始先删上一次的目录。
- **只报新失效**：目标裁决 `breaking`、且当前核心解析同一份记录不报错（内存里对照，不用第二份物化）的才进列表。现在
  就异常的（当前核心都读不懂的）不重复列——插件页已经显示它们，升级不改变它们的处境。
- 系统插件不查：它们跟着核心镜像走，新核心放行后自己会升级它们（ADR-0025 的"升级没结束时核心不动插件"）。
- 限时 60 秒。超时或命令失败，这次检查如实给出"没查成"和原因，升级照常可以发起：兼容检查是给主人看的建议信息，不拦
  升级——让一个查不了的附加检查挡住主人升级，就成了 Nextcloud 的反面。
- 商店、第三方索引、上传和 Agent 写的插件都在已装记录里，一律查（`check-index` 只保官方索引，compat 保每台主机实际
  装着的）。

### 接口（缝二：升级状态 → App）

`CoreUpgradeOffer` 加一项：

```rust
/// 目标版本下已装插件的兼容性。旧核心没有这个字段＝Unavailable。
pub enum CoreUpgradeCompat {
    /// 没查：核心没实现这个检查，或目标镜像太老、没有 compat 子命令。
    Unavailable,
    /// 查了，没有会新失效的已装插件。
    Ok,
    /// 查了，这些已装插件在目标版本下会被标异常。
    Breaking { plugins: Vec<CoreUpgradeBreakingPlugin> },
    /// 查了但没查成：容器没起来、命令超时、输出读不懂。附原因。
    Failed { problem: CoreUpgradeProblem },
    /// 比这个 App 新的核心给的状态。
    #[serde(other)]
    Other,
}

pub struct CoreUpgradeBreakingPlugin {
    pub id: String,
    pub reasons: Vec<CoreUpgradeBreakingReason>, // { category, raw }
}
```

- 挂在 `CoreUpgradeCheck::Ready { offer }` 的 offer 上：确认页是主人做决定的地方。字段 `#[serde(default)]`，缺了按
  `Unavailable`：旧核心没查过，不等于"没有会坏的"。
- 类别只有四个，文字按类别写在壳里（ADR-0043），`raw` 放详情。未来核心加了类别，旧 App 落到 `Other`，按"查不了"
  显示，不伪装成 `Ok`。
- 状态机和 `CoreUpgradeViewModel` 的角标逻辑不动：这个字段只影响确认页多列一段。

### 确认页的三个选择

版本号和迁移提示下面列出会失效的插件（名字、类别一句话、详情），三个选择：

1. **照样升级**：照常发起。升级后这些插件由现有恢复规则标为异常（`Degraded`，原因如实给出）：容器不动，网页和 MCP
   路由不再提供，不静默停用、不删除，卸载永远是主人自己的操作。这是现有行为，本 ADR 不新增规则，只是把"会这样"
   提前到升级前说清楚。
2. **先升级插件**：导航去现有的插件更新流。升级页留着，插件升级后回来重开升级页重查（清单变了，结果跟着变）。
3. **不升级**：不动；跳过版本的现有逻辑照旧（记在本机，渠道上出新版本重新提示）。

### 插件侧不加"最高支持核心版本"

- 装的方向已经覆盖：插件要用新写法就必须把 `spec` 写到那一版，旧核心装它时直接说"先升级核心"
  （`ManifestError::NewerSpec`）。`spec` 就是"最低核心版本"。
- 反方向（新核心对旧插件）由 compat 实测裁决，不靠作者声明。声明的上限会过时：作者保守地写了旧版本，新核心其实兼容；
  按声明拦升级（Cloudron 的 `maxBoxVersion` 拒绝升级，#233）是把判断交给一份会过时的表，还和实测裁决打架——以谁为
  准说不清。
- Obsidian 的 `versions.json` 解决"旧 App 从商店挑能装的最新旧版"，前提是插件发版和 App 发版完全解耦的大生态（#233）。
  Wonton 的商店装最新版，还没有这个需求。真出现了（同一插件要给旧核心留旧版），加在商店索引条目上（ADR-0031 的
  来源侧），不是清单上——清单说的是这份包自己。

### 契约测试（新能力检查单）

| 缝 | 契约 | 提供方 | 消费方 | 换提供方时消费方不用改的证明 |
| --- | --- | --- | --- | --- |
| 目标镜像 → 兼容裁决 | compat 的 stdout JSON：schema、四个类别字符串、退出码语义 | 目标核心镜像（子命令） | 当前核心的升级检查（发起、解析） | 契约测试拿两个提供方喂同一个消费方：真实子命令进程（对本版本二进制跑 fixture 包）和 fake_host 的替身输出（`upgrade/fake_host.rs` 的模式），断言解析成同一份裁决、类别字符串逐字相同。子命令的实现换了，消费方只认 schema |
| 升级状态 → App | `CoreUpgradeCompat` 的 serde 形状：`tag = "state"`、snake_case、缺字段＝Unavailable、未知取值＝Other | 核心管理接口 | wonton-client 的升级状态机和两端壳 | `core_upgrade.rs` 现有的 json! 对比测试模式加四条：旧核心（无字段）→ Unavailable；`Ok`/`Breaking` round-trip；未来类别 → Other；Unavailable 不渲染成"没有会坏的" |

第三条防漂移：契约测试拿官方索引跑 compat（`check-index` 已保证这批清单本核心读得懂），断言没有一条 `breaking`——
compat 和安装路径用同一套解析，结果必须一致。

## 理由与没选的做法

- **镜像标签声明规范版本和能力表**：最便宜，但表达不了权限表、注入变量和渲染规则，"字段不认识"只能靠猜；两处声明会
  漂。选了子命令，标签一个不加。
- **复用 `plugin check` 加 `--json`**：check 的范围是打包质量（图标、截图、收录要求），Report 是人话列表，没有机器
  类别，混着两类问题。compat 只做版本兼容子集，输出就是类别加原值。
- **升级器动手前再查、不对就停**：决策发生在确认页之后，主人已经确认过一轮；Nextcloud 的事故正是检查和执行离得远。
  检查必须在 offer 生成前完成。
- **新核心起来后自己报，App 弹窗**：就是现状，主人已经升完。
- **交给商店或目录服务算兼容性**：要多一个知道所有人装了什么的服务，和 ADR-0015 的组件独立相悖。
- **按能力依赖传播（提供方失效，连使用方一起列）**：compat 只裁决每份清单本身。提供方坏了连累使用方是运行期的事，装
  时查 `requires` 有没有提供方的现有规则兜住（ADR-0053）；传播会让确认页出现主人没法单独处理的条目。
- **插件清单加 `maxCoreVersion`、商店加 `versions.json`**：见上，不选。

## 代价与风险

- 每次检查多跑一个一次性容器（秒级）；插件包物化到数据目录，用完即删。
- compat 是新的跨版本接口，补进 ADR-0025 的"版本之间的约定"：目标镜像没有它（太老，跳版本升级时可能）就 `Unavailable`，
  如实显示；答了但读不懂就 `Failed`。
- 类别只有四个，`other` 会兜住一些说不清的：`raw` 给原值，详情由客户端核心按 App 的语言写（ADR-0043）。再细分等
  真实案例。
- App 端按类别写的文案进四语言本地化资源，是实现 PR 的活。

## 验证结果

本 ADR 只做设计，未实现，无验证结果。
