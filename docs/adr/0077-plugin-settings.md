# ADR-0077 插件设置：清单带 JSON Schema，核心校验并存进记录，改了就只重建用到的服务

- 状态：已接受（2026-11-12）。主人 2026-11-12 定：放开 `boolean` + `const: true` 做必须同意的项；自由文本列表先用文本框；
  不让用户给插件加任意环境变量；整份配置文件的编辑另写一篇

## 背景

Actual 的预算选择等普通设置需要在安装确认页和插件详情页填写。`x-wonton.settings` 只指向包内文件，缺少校验、
存值和交付合同（#16）。应用自有设置仍由应用管理；出口代理走平台出网能力，不用普通插件设置绕过。

`settings` 已经写在规范里当预留，App 侧的“按 Schema 原生渲染”也一直在等一个能用的合同。没有合同就没法做界面：
值存哪、谁校验、改了什么时候生效、敏感项怎么办，每一件都决定另一边怎么写。

核心已有的两套做法可以对照：

- **输入密钥**（ADR-0064）：清单声明，装时在原生表单填，核心保管，不回显，改了重建用过它的服务。
- **渲染漂移**：容器的 `wonton.render` 摘要和这次渲染对不上就重建，卷和数据目录不动。

设置和输入密钥正好相反：值要读出来、要能反复改、不是凭据。所以照输入密钥的表单和“只重建用到的服务”，照渲染漂移
的“用摘要判断”，但值明文存进插件记录、随设置读接口返回。

## 决策

### 清单和文件

`x-wonton.settings` 照旧是插件包里的文件路径，内容是这份设置的 JSON Schema（Draft 2020-12）。文件随插件包固定、
进插件包摘要，最外层照插件包现有规则不超过 2 MiB。

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "x-wonton-order": ["budget", "months"],
  "properties": {
    "budget": {
      "type": "string",
      "enum": ["household", "personal"],
      "x-wonton-title": { "zh": "预算", "en": "Budget", "zh-Hant": "預算", "ja": "予算" },
      "x-wonton-description": {
        "zh": "选择要使用的预算", "en": "Choose the budget to use", "zh-Hant": "選擇要使用的預算", "ja": "使用する予算を選択"
      },
      "x-wonton-enum-titles": {
        "household": { "zh": "家庭", "en": "Household", "zh-Hant": "家庭", "ja": "家計" },
        "personal": { "zh": "个人", "en": "Personal", "zh-Hant": "個人", "ja": "個人" }
      }
    },
    "months": {
      "type": "integer",
      "minimum": 1,
      "maximum": 12,
      "default": 3,
      "x-wonton-title": { "zh": "显示几个月", "en": "Months shown", "zh-Hant": "顯示幾個月", "ja": "表示する月数" }
    }
  }
}
```

- 属性名是小写字母、数字和下划线，`^[a-z][a-z0-9_]*$`，最长 64；一份 Schema 最多 64 项。
- `x-wonton-title` 必填，`x-wonton-description` 可选；`x-wonton-enum-titles` 的每个标题也沿用清单的 `LocalizedText`。
  语言键是相同的 BCP 47 标签：`zh`（简体，`zh-Hans` 同样可协商）、`en`、`zh-Hant`、`ja`，不另设语言格式。
  至少写一种非空语言；示例列全 App 的四种语言。核心复用 `Languages::pick` / `wonton_l10n::choose_language`，
  按请求语言和文字系统协商，繁中优先 `zh-Hant`，缺时回退 `zh`，再回退 `en`，最后使用已有语言；不要求插件为缺失翻译改业务值。
  显示顺序按 `x-wonton-order`，没列的按属性名排在后面。
- Schema 只描述顶层设置项；App 画不出控件的 Schema 在预览阶段返回 400 `settings_schema_invalid`，
  类别为 `UnsupportedKeyword`，不装、不保存，也不提供 JSON 文本编辑器。
- `$schema` 必须是 `https://json-schema.org/draft/2020-12/schema`，不写或写成别的 draft 都算不合格。

原生控件子集如下，`type` 必须是单个字符串，不能写联合类型：

| 属性 Schema | 原生控件 | 可选约束 |
| --- | --- | --- |
| `type: boolean` | 开关 | `default` |
| `type: boolean`，`const: true` | 必须勾上的复选框（同意条款） | 无；必须在 `required` 里，不能写 `default` |
| `type: integer` | 整数输入 | `minimum`、`maximum`、`default` |
| `type: number` | 小数输入 | `minimum`、`maximum`、`default` |
| `type: string`，没有 `enum` | 文本框 | `minLength`、`maxLength`、`pattern`、`x-wonton-pattern-description`、`default` |
| `type: string`，`enum` 全为字符串 | 单选 | `default`、`x-wonton-enum-titles` |
| `type: array`，`items.type: string`，`items.enum` 全为字符串 | 多选 | `uniqueItems: true`（必需）、`default`、`x-wonton-enum-titles` |

根只用 `$schema`、`type`、`properties`、`required`、`additionalProperties: false` 和 `x-wonton-order`。
属性除表中的类型与约束，只能加 `x-wonton-title`、`x-wonton-description`；选项标题放在该属性的 `x-wonton-enum-titles`。
`object`、嵌套数组、无字符串枚举的数组、非字符串枚举、`null`、上表之外的 `const`、`oneOf`/`anyOf`/`allOf`、`$ref`、
`$dynamicRef` 和其他超出上表的写法均拒绝，不默默忽略。任何子 Schema 的敏感标记仍按敏感项拒绝。

- **必须同意的项**（Minecraft 的 EULA 这类）：只放开 `type: boolean` + `const: true` 这一种 `const`。不写 `default`，否则等于
  替主人勾上；不在 `required` 里也不合格，否则不勾也能装。核心照常用 `jsonschema` 校验，没勾的值过不了 `const`，安装和升级
  都被拦下。注入的值是 `true`。条款换了版本要主人重新同意的，插件换一个属性名：新版本新增的必填项没有默认值，按下面
  “校验时机”里升级预览的规则拦下，主人勾了才能升。
- **自由文本列表**（白名单、订阅地址）没有专门的控件，用文本框：`pattern` 限定写法，`x-wonton-description` 写明分隔符，
  如“用英文逗号分隔，例如 steve,alex”。插件自己拆。
- **`x-wonton-pattern-description`**：值不匹配 `pattern` 时给主人看的一句话，`LocalizedText`，只能写在有 `pattern` 的文本框上。
  不写时 App 用通用的“格式不对”，正则原文只进“详情”。

- 敏感项不留在设置里：属性带 `writeOnly: true`、`format: "password"` 或 `x-wonton-secret: true` 时算错误（见
  “敏感项”）。
- 属性名不得和 `x-wonton.secrets` 里的名字重复：同一个东西两头都能填，会分不清哪边生效。
- 整份生效值序列化后不超过 64 KiB，单个字符串不超过 8 KiB，一个字符串数组不超过 64 项；大小按 UTF-8 字节计。
  超过时返回 `settings_invalid`。
- 使用 `settings` 的清单至少写 `spec: "0.12"`；用到必须同意的项或 `x-wonton-pattern-description` 的至少写 `spec: "0.13"`，
  旧核心先看版本号提示升级，而不是报 Schema 不合格。

### 校验和库

Schema 自己在校验值之前先校验一遍：能解析、`$schema` 是 2020-12、根是 `type: object` + `additionalProperties: false`、
每个属性有标题、没有敏感标记、名字不和密钥重名、默认值自己过得了 Schema、枚举非空不重复、上下限不颠倒。
不合格时安装和升级在动资源之前就返回 400 `settings_schema_invalid`，带每一项的问题和位置。

值用 [`jsonschema` 0.61.1](https://docs.rs/jsonschema/0.61.1/jsonschema/)（MIT，最低 Rust 1.85，支持 Draft 2020-12）。
设 `default-features = false`，不取网络和宿主文件上的 Schema。关键字校验交给库，核心只检查插件字段规则和生成表单，
不另写一份 JSON Schema 校验器。

校验时机：

- 安装、升级请求在拉镜像、建网络之前按新 Schema 校验整份生效值。不通过返回 400 `settings_invalid`，带每个字段的
  路径、类别和一句人话，什么都不改。
- 安装请求只带主人新填的值，默认值由核心按 Schema 的 `default` 补。必填项（在 `required` 里）没有值、也没有默认值时
  不能提交。
- 升级预览把装着版本的值填进新 Schema 的表单。新 Schema 下已经不合法、或新 Schema 删掉了的属性：删掉的属性连值一起
  丢掉；不合法的留在表单里，带问题说明，主人改好才能确认升级。新版本新增的必填项没有默认值时同样标出来并被拦下，
  和 ADR-0064 对待新增密钥一样。
- 装好以后写设置，只校验新值叠在当前值上的结果；不通过时原值不动、不重建。
- `plugin check`（开发套件同一套）从 Schema 推出这个插件合法的 `WONTON_SETTING_*`：引用 Schema 里有的名字，裸写
  `${WONTON_SETTING_X}` 就算已定义，不要求 `:-` 默认值；引用 Schema 里没有的名字，带不带 `:-` 都报错。
- `additionalProperties` 只能写 `false`。请求里带 Schema 没有的属性也是 `settings_invalid`。

### 存哪

值存进插件记录（`InstalledRecord.settings`），一个属性一项，另记最后修改时间。记录本来就在核心数据目录、权限 0600。
值不是凭据，明文存、随读接口原样返回。

卸载时和记录里的其他东西一起按“保留数据”处理。清空一项时删掉存值、重新用默认值；无默认值的可选项注入空串。

### 怎么交给插件

核心提供 Compose 插值变量 `WONTON_SETTING_<属性名大写>`。字符串原样；布尔是 `true`/`false`；整数和数字是十进制；
多选值为 JSON 字符串数组文本，仅供插件程序读取，不让用户编辑。无值且无默认值的可选项注入空串，显式 `false`、`[]` 不算没值。

变量不会自动进入容器环境，插件在服务的 `environment`、命令或 `configs` 内容里显式引用。

**改了设置，按 `wonton.render` 摘要差异只重建最终渲染结果发生变化的服务。** 不给所有容器注入整份设置，因此无关服务
的摘要不变。同值写入不重建；未被引用的设置只存值。卷和数据目录不动。
重建失败时把值退回原样、按旧值再渲染一次，错误照原类别返回；退回也失败就把插件标成异常。

用户不能给插件加 Schema 以外的环境变量。要让主人改的东西，由插件作者声明成设置项。

### 敏感项

敏感项留在输入密钥（ADR-0064）：值不回显，走系统密码框。设置和密钥不重叠：Schema 任意位置出现敏感标记都拒绝，
不让嵌套对象或组合约束变成另一条明文存密钥的路径。未标记的秘密无法自动识别，规范明确禁止放进设置。

理由：Secret 走的是另一条存储和注入路径（`WONTON_SECRET_*`、不回显、改完必重建），如果设置也能存秘密，App 得按字段
逐项猜哪条路径、详情页要不要打码，值也可能被日志和调试接口带出去。

### 接口

设置页是原生页，形状由核心按受支持 Schema 子集给：字段、标题、说明、控件类型、范围、枚举、当前值、默认值、
用到它的服务和校验问题。壳只画控件、收值、提交。插件包给字段标题和说明，其余界面文字走本地化资源（ADR-0043）。
控件类型 `PluginSettingKind` 里，必须同意的项是单独一种 `Agreement`，文本框带上可选的 `pattern_description`。
新核心增加的控件或值类型落到 `Other` 时，该项显示“要更新 App 才能改”，禁止提交整份表单；
不能省略或清空该项后提交，也不给 JSON 编辑器。Schema 本身不受核心支持与旧 App 不认识新控件是两种不同情况。

- `GET /_wonton/api/plugins/{id}/settings` → `PluginSettingsView`。插件没装时 404 `not_installed`；装着的版本没有
  `settings` 时 400 `unsupported`。
- `POST /_wonton/api/plugins/{id}/settings/preview`：请求体是写请求的体，只校验、不保存，返回填上值的视图。
- `PUT /_wonton/api/plugins/{id}/settings`：`PluginSettingsUpdate`，保存并重建，返回重建后的视图。
- 视图和写请求都带 `schema_digest`（设置文件的 sha256）。插件在中间升级换了文件时对不上，返回 409 `changed`，App
  重新读。
- 安装预览的 `settings` 带表单和默认值；安装、升级请求和确认卡片的回答用 `settings` 带值。Bot 经 MCP 装插件时，
  主人在确认卡片里用同一套原生表单填，值不进 Matrix。上传插件包（请求体是包本身）安装、升级时，值放在
  `Wonton-Plugin-Settings` 请求头，编码同输入密钥的头（base64url 无填充的 JSON，ADR-0064），写法同 `settings`；
  插件开发套件的 `--setting` 走这条路。
- 核心只给已支持设置接口的版本置 `PluginDetail.has_settings`；接口阶段不凭 Schema 文件存在就开启入口。
- 只有主人能读写，路由按 `owner_only`（同 ADR-0045）。

接口只有值和字段形状，不回显 Schema 原文。

### 错误类别

| HTTP | `kind` | 情况 |
| --- | --- | --- |
| 400 | `settings_schema_invalid` | 包的 Schema 自己不合格，`problems` 带位置、类别和说明 |
| 400 | `settings_invalid` | 值过不了 Schema，`fields` 带字段、类别和说明 |
| 409 | `changed` | `schema_digest` 对不上 |
| 409 | `conflict` | 插件正在装、卸、重建 |
| 400 | `unsupported` | 装着的版本没有设置 |
| 404 | `not_installed` | 没装这个插件 |

## 理由

- **Schema 在包里**：设置是插件能力的一部分，跟版本走、进摘要，预览和升级能对出差异。让用户在插件网页里填、核心不
  认识，就还是今天的样子：没有原生页、升级丢了也不知道。
- **核心校验**：值最终要进容器，核心是渲染的人；把校验留给插件，装错的值会以“容器起不来”的样子出现，看不出是哪个
  字段。
- **只重建用到的服务**：改一个预算不该把整个 Compose 项目重启。渲染结果已经知道哪个服务引用了哪个变量，和输入密钥
  共用筛选，不用另写一套依赖分析。
- **存记录**：设置属于这个插件实例，和权限、局域网开关一样，卸载时跟着走。单独目录只有一份文件，没多出任何隔离。
- **敏感项走密钥**：见上，走已经定好的路径，App 只需按两种来源分别渲染。
- **必须同意只放开 `const: true`**：Minecraft 这类上游不同意条款就不启动，没有它插件只能写死“已同意”替主人点头。
  只放这一种，其余 `const` 照旧拒绝，子集不因此变宽。StartOS 用必须先完成的任务做同一件事。
- **列表先用文本框**：上游大多要逗号分隔的字符串，而多选注入的是 JSON 数组文本；专门的列表控件要连同注入格式一起定，
  等有足够多的插件需要再定。
- **未知类型要求更新 App**：不把用户不认识的大括号当原生控件的替代品，也不把解析不了的值改成空值。支持子集之外的 Schema 直接拒绝。

## 代价与风险

- 值明文可读，进日志和调试接口；放进去的就不该是凭据。
- 一份设置全插件共用，Bot 服务也一样。每个 Bot 一套设置这次不做。
- Schema 只支持原生控件子集；需要复杂对象、组合约束或其他数组的插件，应使用应用自己的设置页或调整普通设置的模型。
- 改设置会重启用到的服务，正在进行的任务会断。
- 校验库带来一棵依赖树；不收 `$ref` 后，公共的片段只能复制到每个属性里。
- 升级时删掉的属性丢值，降级回去只能重填。
- 自由文本列表靠文本框和说明，填错只能靠 `pattern` 挡，没有逐项增删的控件。
- 整份配置文件（如 Glance 的 `glance.yml`）不适合拆成设置项，它的编辑另写一篇 ADR。

## 没选的做法

- **核心不解释，交给插件命令校验**：每条设置都要插件实现一个命令和输出格式，核心拿不到字段的形状，App 也就画不出
  原生表单。
- **值放 `configs` 的配置文件**：改一次就要重写配置、重启全部挂它的服务；一份设置被几个服务用时还会各存一份。
- **敏感项也允许写在设置里**：多一条回显、多一套打码规则，详情页要按字段区分，存错位置的值会以明文露出来。
- **JSON 文本编辑器兜底**：把插件作者的 Schema 复杂度交给普通用户，不符合原生表单的目标；超出控件子集直接报包的设置 Schema 不合法。
- **只有插件网页，没有原生页**：就是现状；装的时候还要先起容器再碰网页，Bot 装的时候更没法填。
- **每改一项就重建全部服务**：简单，但一个预算选择会断掉目录同步和网页。
- **让用户给插件加任意环境变量**（Umbrel 的 customEnvironment、Portainer 改 stack 的环境变量）：绕过插件包摘要和 Schema 校验，
  升级时也没法对出差异；写错了就是“容器起不来”。
- **同意条款做成插件网页里的一步**：要先起容器才能同意，而上游不同意就不启动。

## 关联

- ADR-0015（小组件与标准接口）、ADR-0017（插件系统和接口）、ADR-0064（输入密钥）、ADR-0063（渲染漂移）、
  ADR-0045（主人-only 路由）、ADR-0043（界面文字）。
