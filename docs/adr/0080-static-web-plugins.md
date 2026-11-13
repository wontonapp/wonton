# ADR-0080 静态网页插件：文件交给静态托管插件，核心只存文件、转发和记账；外部网页只存一个地址

- 状态：已接受（2026-11-11，主人定，见“主人已定”）
- 补充 [ADR-0066](0066-plugin-forms.md) 第 1 节的运行形态表（那里写着“静态网页以后需要再加”），不取代它

## 背景

插件只有容器一种能跑网页的形态。纯静态的页面也要写一份 compose、起一个 nginx 容器。

2026-11-11 盘点一台开发主机上 Agent（Hermes）写的 33 个插件：

- 21 个是 nginx 加一个 `index.html`（如一个 17 KB 的贪吃蛇）。每个真实内存约 25 MiB，首次要拉约 92 MB 的镜像。
- 12 个是标准库 python 小后端，140–210 行，只读写 `/data/*.json`（待办、记账、习惯打卡）。没有一个出网、用密钥、用模型或定时。
- 容器的代价主要不在内存，而在每个插件一份健康检查、init、用户的样板，以及改一个字就要重建容器。

官方索引的 51 个条目全是容器，没有纯静态、外部网页条目。

业内（调研 2026-11-11，12 个项目加 MCP 官方注册表）：

- 家庭服务器一类（Umbrel、CasaOS、Runtipi、YunoHost、Cloudron、Home Assistant 的 apps）都是一个应用一组容器。
- “轻”只出现在不跑代码的静态资源上：Cloudflare Workers 命中静态资源时不运行 Worker 代码
  （[static assets](https://developers.cloudflare.com/workers/static-assets/)）。
- “页面加平台给的一小块存储”是通用做法：Telegram Mini App 的 CloudStorage（[文档](https://core.telegram.org/bots/webapps)）、
  Figma 的 `clientStorage`、Obsidian 的 `saveData`。
- Figma 的插件清单声明能访问的域名，没声明的被 CSP 拦下（[how plugins run](https://developers.figma.com/docs/plugins/how-plugins-run/)）。
- Home Assistant 的 apps 由 Supervisor 按清单算一个安全分，作者不能自己填
  （[security rating](https://contributors.home-assistant.io/apps/reference/security-rating/)）。

## 决策

### 1. 两种新形态

`x-wonton.runtime` 加两个取值：

| `runtime` | 核心做什么 | 谁执行 | 包里有什么 |
|---|---|---|---|
| `static_web` | 存文件、交给提供方、网关转发、记账 | 提供能力 `static_web` 的插件（静态托管插件） | 清单加一个静态文件目录 |
| `link` | 只存一个地址 | App 打开 | 只有清单 |

- 跑代码的插件照旧是 `compose`。
- `static_web` 是第一个“托管形态”（第 2 节）。
- 不做共用轻运行器，也不做 Python 模板、共享依赖仓库和 Pyodide 模板。主人的话：“不做，要跑脚本 agent 自己在容器跑不就行的吗？”
- 远端 MCP 紧接静态网页做，照 [ADR-0066](0066-plugin-forms.md)。
- 定时任务和 webhook 不做（主人 2026-11-11 定），有真实消费方时另写 ADR。

### 2. 托管形态：运行形态的名字就是一项能力

“托管形态”是核心不执行、交给一个能力提供方执行的运行形态。核心对它只做四件事：存文件、把目录交给提供方、网关转发、记副作用账本。
新加一种托管形态，只在能力表加一行、做一个提供方插件，核心不认识具体怎么执行（ADR-0015）。

- **能力表标记：** 能力表（ADR-0053）的 `Capability` 加一个字段 `hosted: Option<HostedForm>`。`runtime` 的取值必须是标了
  `hosted` 的能力名。`HostedForm` 写明这个形态在包里收哪个目录、提供方的挂载是只读还是读写。
- **隐含依赖：** `runtime: static_web` 等于 `requires: [static_web]`，清单里不用再写。没有提供方时，装、预览、商店的处理和
  缺 `requires` 完全一样：列出缺的能力和索引里提供它的插件，主人选了就一起装（ADR-0053 的“装”）。
- **提供方声明挂载点：** 提供方在 `provides.<能力名>` 里多写一个 `mount`（容器里的绝对路径）。核心建它的容器时，把
  `<核心数据目录>/hosted/<能力名>/` 挂到那里（`static_web` 只读）。以后装、升级、卸载托管插件只改这个目录里的内容，
  不重建提供方的容器：Docker 不能给运行中的容器加挂载（同 ADR-0066 的技能目录）。
- **一个插件一个子目录：** `<挂载点>/<插件 ID>/` 就是这个插件的根。插件 ID 只有小写字母、数字、连字符和点，每段非空
  （`PluginId::parse`），所以不会出现 `..`，也不会撞上核心用的 `.staging/`。
- **原子替换：** 核心先解包到 `.staging/<插件 ID>-<摘要>/`，再用 `renameat2(RENAME_EXCHANGE)` 和 `<插件 ID>/` 对换，最后删掉旧的。
  进行中的请求看到的要么全是旧版本，要么全是新版本。
- **转发：** 网关照旧按插件自己的子域名路由，目标从“插件的服务和端口”换成“提供方的服务和端口”。转发前删掉请求里所有
  `X-Wonton-*` 头，再写入 `X-Wonton-Plugin: <插件 ID>`（ADR-0053 的“访问”）。提供方只认这个头选子目录，不认请求的路径和 `Host`；
  不带这个头的请求回 400。
- **`fallback` 由网关补取：** 提供方回 404 的 `GET`、`HEAD`，网关按清单的 `fallback` 换路径再取一次。提供方因此只是普通的静态文件服务，
  不用读清单；换提供方时单页应用的行为不变。
- **访问控制在网关：** 谁能打开（主人、家人、好友共享）和容器插件的网页一样由网关判断（ADR-0033、ADR-0045、ADR-0051），
  提供方不做鉴权。每个插件仍是一个独立的源，浏览器的同源隔离不变。
- **账本：** 托管子目录和第 6 节的数据目录记进副作用账本（ADR-0066 第 7 节），卸载照账本撤。
- **状态：** 提供方在运行、子目录在，插件就是“运行中”。提供方没在运行时，插件标为异常，类别是 ADR-0053 的“依赖的插件没有在运行”，
  带上提供方的名字，不显示成插件自己坏了。主人停用一个托管插件，网关就不再转发它，提供方不动。
- **卸载提供方：** 有托管插件在用时照 ADR-0053 拦下，列出这些插件。

### 3. `static_web` 的清单

包格式和容器插件相同：仓库根的 compose 文件，只有 `x-wonton`，没有 `services`。

```yaml
x-wonton:
  spec: "<下一版规范>"
  id: io.github.alice.snake
  version: 1.0.0
  kind: app
  runtime: static_web
  title: {zh: 贪吃蛇, en: Snake}
  permissions: [network.egress]       # 写了 static_web.connect 才写
  static_web:
    root: web                         # 包里的目录，默认 web
    fallback: index.html              # 可选：找不到的路径回这个文件（单页应用）；不写回 404
    connect:                          # 可选：页面能连的外部来源，见第 4 节
      - https://api.open-meteo.com
      - https://*.tile.openstreetmap.org
  webui:
    path: /
  tabs: [...]                         # 照 ADR-0033
```

- **允许的字段：** `static_web`、`webui` 的展示字段（`path`、`cross_origin_isolated`；`display` 已由 [ADR-0095](0095-no-tab-bar.md) 删掉）、`tabs`、`requires`/`wants`
  （只能是第 6 节的 `web_storage`）、`permissions`（只能是 `network.egress`，第 4 节），以及标题、说明、图标这些商店字段。
- **装时报错的字段：** `services`、`webui.service`、`webui.port`、`webui.sso`、`webui.oidc`、`webui.proxy`、`webui.friends`、
  `secrets`、`settings`、`mcp`、`bot`、`bots`、`provides`、`lan`、`tunnel`、`setup`，以及 `network.egress` 以外的权限。
  不静默忽略，错误写明“静态网页没有服务端，用不上 X；要用就写成容器插件”。
  - 密钥不允许，因为静态文件对能打开页面的人全部公开。
  - `webui.friends` 第一步不收：好友能不能写第 6 节的数据，以后有需要时另定。
- **目录规则：**
  - `root` 下最多 1024 项、合计 64 MiB、单个文件 16 MiB。压缩包的上限照 ADR-0031 不变；静态目录的单文件上限比
    ADR-0031 引用文件的 2 MiB 宽，因为前端打包出的 JS 和 wasm 常超过 2 MiB。
  - 不能有软链接、`..`、以 `.` 开头的路径段；不能有 `_wonton/` 目录（第 6 节占用）。
  - 没有 `index.html` 时报错。
- **网关给静态页加的响应头**，盖掉提供方给的同名头：
  - `Content-Security-Policy`，见第 4 节；
  - `X-Content-Type-Options: nosniff`；
  - 声明了 `cross_origin_isolated` 时加跨源隔离要的头，同 ADR-0033 的“跨源隔离”。
- **能被 Agent 写：** Bot 经 `wonton__check_plugin` 和 `dir` 来源（ADR-0033）照样能写、能装这种插件。

### 4. 页面能连哪些外部来源：清单声明，网关下发 CSP

静态页只能连清单 `static_web.connect` 里写的来源。没写的，浏览器按 CSP 拦下。

- **写法：** 每项是 `https://<主机名>` 或 `wss://<主机名>`，主机名可以以 `*.` 开头，最多 32 项。
  - 不收 `http:`、路径、端口、IP 字面量、`localhost`、单段主机名，也不收单独的 `*`。
  - 要连局域网设备的页面写成容器插件，用 `network.lan`（ADR-0019）。
- **和权限对得上：** 写了 `connect` 就要在 `permissions` 里写 `network.egress`，反过来也一样。只写一边，清单不合法（同 ADR-0036 的规则）。
  确认页的这一行写“这个网页会从你的设备连接：<列表>”。
- **CSP 由核心算、网关加，不交给提供方：** 换一个提供方，或者提供方配错了，出网白名单照样生效。网关下发的是：

  ```
  default-src 'self'; script-src 'self' 'unsafe-inline' 'unsafe-eval' blob: <connect>;
  style-src 'self' 'unsafe-inline' <connect>; img-src 'self' data: blob: <connect>;
  font-src 'self' data: <connect>; media-src 'self' data: blob: <connect>;
  connect-src 'self' <connect>; worker-src 'self' blob:; frame-src <connect，没有时 'none'>;
  object-src 'none'; base-uri 'self'; form-action 'self'
  ```

  - 声明的来源对每一种取资源的指令都放行（取脚本的 CDN、地图瓦片、API）。分指令声明 Agent 难写对，也看不出多拦了什么。
  - 放行 `'unsafe-inline'` 和 `'unsafe-eval'`：这条 CSP 管的是出网，不是防 XSS。Agent 写的页面几乎都有内联脚本。
- **连接从主人的设备发出：** 页面在 App 的 WebView 里跑，外连从手机直接出去，不经家里服务器，也不走出网代理（ADR-0023、ADR-0039）。
  确认页的文字照这个写。
- **只管网页：** 这条 CSP 不影响 Agent 和容器插件上网。

### 5. `link`：外部网页只存一个地址

```yaml
x-wonton:
  runtime: link
  link:
    url: https://example.com/dashboard
```

- `url` 只收 `https`，不能带用户名和密码。清单里除了商店字段和 `link`，什么都不能有：没有权限、密钥、`webui`、`tabs`、`requires`。
- 核心不代理、不做单点登录、不检查地址通不通，只存记录。
- App 在“应用”列表里显示它，主人可以把它钉进 App 的侧边栏或面板。
- 打开时用系统自带的 App 内浏览器：iOS 用 `SFSafariViewController`，Android 用 Custom Tabs。顶上显示网址。
- 不用插件 WebView，不给任何和 App 说话的通道：没有桥，不能发消息、不能调工具（ADR-0033 的桥只给插件自己的网页）。
- 卸载只删记录。安全档位是 `minimal`（第 7 节）。

### 6. 第二步：每个静态插件一小块数据存储（WebDAV 子集）

静态网页做完紧接着做（主人 2026-11-11 定）。Hermes 那种“页面加一份 JSON”的小插件就不用容器了。

- **能力：** 能力表加 `web_storage`，`Credentials::None`，不是高权限。官方静态托管插件同时提供 `static_web` 和 `web_storage`，
  所以不另起容器；两项是两个能力，以后可以分开换提供方。
- **使用方：** 静态插件写 `requires: [web_storage]`（没有就用不了）或 `wants: [web_storage]`。
  - `requires`：没有提供方装不上；提供方停了插件标异常；有插件 `requires` 它时提供方卸不掉。
  - `wants`：没有提供方照装，数据目录照建，数据路径回 503；网页本身照常。
  - 只给静态网页：容器插件有自己的 `/data`，写了照旧报不支持。
- **访问路径：** 页面用相对地址 `fetch('/_wonton/data/todo.json')`。网关把插件子域名上 `/_wonton/data/` 开头的请求转给它选用的
  `web_storage` 提供方，去掉这个前缀，写入 `X-Wonton-Plugin`。没声明 `web_storage` 的插件请求这个路径回 404。
- **挂载：** `provides.web_storage.mount` 读写挂 `<核心数据目录>/hosted/web_storage/`，每个插件一个子目录，装的时候由核心建，
  属主是 `PUID:PGID`（提供方以这个用户写）。升级不动数据；新版本不再写 `web_storage` 时数据也留着，账本照样记着。
- **契约（RFC 4918 的子集，整文件读写）：**

  | 方法 | 支持的部分 |
  |---|---|
  | `GET`、`HEAD` | 取整个文件，带 `ETag` 和 `Content-Type` |
  | `PUT` | 整个文件覆盖或新建。`If-Match` 不符回 412；`If-None-Match: *` 而文件已在回 412（RFC 9110 的条件请求）。父目录不在回 409 |
  | `DELETE` | 删文件或空目录 |
  | `MKCOL` | 建目录 |
  | `PROPFIND` | 只收 `Depth: 0` 和 `1`，回 `getcontentlength`、`getetag`、`getlastmodified`、`resourcetype` |

  - 不支持：`LOCK`/`UNLOCK`、`MOVE`/`COPY`、`PROPPATCH`、带 `Content-Range` 的部分写入、`Depth: infinity`，一律回 405 或 501。
    整文件覆盖加 `ETag` 就够做“读、改、写回”；锁和部分写入是 Agent 写的页面用不上、又最难写对的部分。
  - 上限：每个插件 64 MiB、1024 个文件，单个文件 8 MiB。超了回 507，原文件不动。上限写在能力表的接口约定里，由提供方执行。
- **写要同源：** 网关对 `PUT`、`DELETE`、`MKCOL` 要求 `Origin` 等于插件自己的源，否则回 403。别的插件的页面借主人的登录态写不进来。
  - 除 `GET`、`HEAD`、`OPTIONS`、`PROPFIND` 以外的方法都按写处理，契约里不支持的方法也一样。
  - 浏览器标了 `Sec-Fetch-Site: same-origin` 的也算同源：App 的 WebView 经隧道打开时地址的写法可能和核心认的不完全一样。
  - 别的网页发来的请求在会话这一关就按没登录处理（插件网页的规则），回 401，到不了这一步。
  - 数据的响应也盖上第 4 节的 CSP，加 `Cache-Control: no-store`：存一个 HTML 再打开它，也连不出声明以外的地方。
- **谁能写：** 一个插件一份数据，能打开页面的人都能读写，同容器插件的 `/data`。每人一份分开的数据这次不做。
- **卸载：** 照“保留数据”处理，和容器插件的数据目录一样：默认留下，选“彻底删除”时删掉子目录。
  留着数据再装同 ID 的插件，照 ADR-0031 的“留下的数据”醒目提示。

### 7. 安全档位：核心按清单算，商店、预览、详情页都显示

| 档位 | 条件（取最高的一条） |
|---|---|
| `minimal` | `link`；没写 `connect` 的 `static_web` |
| `standard` | 不满足下面任何一条 |
| `high` | 声明了 ADR-0036 的“高”级权限，或者 `requires`/`wants` 一个高权限能力（ADR-0053） |
| `very_high` | 声明了“很高”级权限 |
| `highest` | 声明了“最高”级权限 |

- 声明了的都算，包括默认关的可选权限（ADR-0057）：档位说的是这个插件能拿到什么，不是现在开了什么。
- 只由核心按清单算，作者不能填，也不靠人工审（同 Home Assistant 的安全分）。
- 和来源标签（官方、未审核，ADR-0031）是两件事，并排显示，不合成一个分数。
- 协议：`wonton-protocol` 加 `SecurityTier`（上面五个取值加 `Other`），App 收到不认识的取值按 `highest` 画。
  `StoreItem.high_permissions: bool` 换成 `security_tier: SecurityTier`，`InstallPreview` 和 `PluginDetail` 也加这个字段。
  文字和图标由 App 按档位写（ADR-0043）。

### 8. 接口 PR 的内容

一个核心 PR 加一个 wonton-docs 的规范 PR，规范版本取下一版。

**清单（`x-wonton`）：**

| 字段 | 类型 | 说明 |
|---|---|---|
| `runtime` | 加 `static_web`、`link` | 第 1 节 |
| `static_web.root` | 字符串，默认 `web` | 包内相对路径 |
| `static_web.fallback` | 字符串，可选 | `root` 下的文件 |
| `static_web.connect` | 字符串数组，可选，最多 32 项 | 第 4 节的写法 |
| `link.url` | 字符串 | `https` |
| `provides.<能力>.mount` | 容器里的绝对路径 | 只在标了 `hosted` 的能力上收，对这些能力必填 |

`spec_floor` 表按这几行加。

**核心：**

- 能力表：`Capability.hosted: Option<HostedForm>`；`HostedForm { package_dir, mount: ReadOnly | ReadWrite }`，`package_dir` 是清单里
  取目录的字段（`static_web.root`），没有包内目录的为空。
- 加 `static_web`（只读挂载，能写在 `runtime` 里）和 `web_storage`（读写挂载，不能写在 `runtime` 里，只能被 `requires`/`wants`）。
- 网关的路由目标加一种“提供方 + 插件 ID”，以及 `/_wonton/data/` 前缀的分流和同源检查。
- 托管目录的解包、原子替换、删除，进副作用账本。

**协议（`wonton-protocol`）：**

- `PluginRuntime` 加 `StaticWeb`、`Link`；
- 加 `SecurityTier`，`StoreItem`、`InstallPreview`、`PluginDetail` 带上它，删 `StoreItem.high_permissions`；
- `PluginDetail` 加 `link_url: Option<String>`，只在 `runtime: link` 时有。

**错误类别**（都用现有的 `PluginFailure` 和 ADR-0053 的状态类别，不加新类别）：

| 情况 | HTTP | 类别 |
|---|---|---|
| 清单写了第 3、5 节不允许的字段；`connect` 写法不对；`connect` 和 `network.egress` 对不上；目录超限、有软链接、没有 `index.html` | 400 | `invalid` |
| `runtime` 是这份核心不认识的取值，或 `spec` 比核心新 | 400 | `unsupported`（同 ADR-0066：“先升级核心”） |
| 没有 `static_web` 或 `web_storage` 的提供方 | 预览的 `missing_requires` | 同 ADR-0053 |
| 卸载正被托管插件用着的提供方 | 409 | `conflict`，列出使用方（同 ADR-0053） |
| 提供方没在运行时打开页面 | 503 | 运行时状态“依赖的插件没有在运行”（ADR-0053） |
| 解包或原子替换失败 | 500 | `upstream`，原目录不动 |

**提供方回的状态：** 400（没有 `X-Wonton-Plugin` 或值不合法）、404（没有这个插件的子目录或文件）、405/501（不支持的方法）、
409、412、507（第 6 节）。网关原样转给页面。

### 9. 契约、提供方、消费方和契约测试

| | `static_web` | `web_storage` | `link` |
|---|---|---|---|
| 契约 | HTTP 静态文件语义（RFC 9110：`GET`/`HEAD`、按扩展名的 `Content-Type`、`ETag` 和条件请求、`Range`、404）+ 只读目录布局 + `X-Wonton-Plugin` | WebDAV（RFC 4918）的整文件子集 + 条件请求（RFC 9110）+ 同样的目录布局和头 | 一个 `https` 地址 |
| 提供方 | 官方静态托管系统插件：候选 static-web-server（MIT/Apache-2.0）或 Caddy 的 `file_server`（Apache-2.0），用上游官方镜像 | 同一个插件：Caddy 的 WebDAV 模块，或 rclone `serve webdav`（MIT） | 无，App 打开 |
| 真实消费方 | Agent 写的纯静态页（Hermes 写的贪吃蛇这类单个 `index.html`）；wonton-docs 指南里的 `static` 例子 | Hermes 写的待办、记账、习惯打卡页，改成静态页加 `/_wonton/data/*.json` | 主人自己的外部网页入口 |

**换提供方时消费方不用改：** 每项能力一组契约测试，照 `model_gateway_contract` 的样板，同一组断言跑在两个提供方上：

- 只在测试代码里的假提供方：一个几十行的 HTTP 服务，跑在默认测试里；
- `system.lock` 固定版本的官方提供方：要 Docker，标 `#[ignore]`。

`static_web_contract` 断言：
- 首页、子路径、`fallback` 指的文件都取得到，`Content-Type` 对；
- 缺文件 404；`..`、`%2e%2e`、`%2f` 编码的穿越被拒；
- 带 A 的头取不到 B 的文件；不带头回 400；
- 原子替换后下一个请求就是新内容，提供方没有重启；
- `HEAD`、`If-None-Match`、`Range` 行为对。

`web_storage_contract` 断言：
- `PUT` 后 `GET` 字节相同，`ETag` 变了；
- `If-Match` 不符回 412，`If-None-Match: *` 不覆盖；
- `PROPFIND` 的 `Depth: 1` 列出刚写的文件；`DELETE` 以后 404；
- 超上限回 507、原文件不动；A 读写不到 B 的目录；不支持的方法回 405 或 501。

网关一侧另有核心的行为测试，和提供方无关：
- 静态页响应带算出来的 CSP，盖掉提供方给的；
- 请求里自带的 `X-Wonton-Plugin` 被删掉重写；
- 提供方回 404 时按清单的 `fallback` 补取，没写 `fallback` 的照实 404；
- `/_wonton/data/` 的写请求没有同源 `Origin` 回 403；
- 提供方停了回 503，类别正确。

## 理由

- **托管交给能力插件：** 核心只存文件、转发、记账，执行在可替换的插件里（ADR-0015）。静态文件服务和 WebDAV 都有成熟的开源实现，
  不用自己写。
- **运行形态名就是能力名：** 新的托管形态不改核心的执行代码，只加一行表和一个插件；缺提供方、装在一起、卸载拦下全部复用 ADR-0053。
- **一个总目录、按头选子目录：** Docker 不能给运行中的容器加挂载。总目录挂一次，以后只改内容，和 ADR-0066 的技能目录同一个做法。
  子目录只认核心写的头，提供方不用懂路由，也就不会被请求里的 `Host` 骗到别的插件。
- **每插件一个源：** 共用一个托管进程，浏览器的隔离和现在一样（同 Sandstorm 的每应用一个主机名）。
- **CSP 由核心算：** 出网白名单是安全约束，放在提供方就要求每个提供方都写对；放在网关，一处实现、一组测试。
- **WebDAV 子集：** 是标准协议，有现成服务端，浏览器里直接 `fetch` 就能用；整文件读写和 Obsidian 的 `data.json`、
  Telegram 的 CloudStorage 一样，够 Agent 写的小页面用。
- **安全档位由核心算：** 确认页已经按权限给风险等级，档位只是取最高的一项，商店列表上就能看出来，不用多写文字。

## 代价与风险

- 静态托管插件停了，所有静态插件一起打不开。详情页如实写“静态托管插件没有就绪”。
- CSP 拦不住所有外泄：
  - 顶层跳转（`location = ...`）和 `window.open` 不归 CSP 管。App 的插件 WebView 把主框架跳到别的域名改成系统浏览器打开
    （ADR-0033），主人看得见，但拦不住。
  - WebRTC 和 DNS 预取 CSP 也管不到。
  - 所以确认页不写“不会外连”，只写“只能连这些地址”。
- 页面直接从手机外连：家里服务器配的代理对它不生效，被墙的 API 在手机上照样连不上。
- 一个插件一份数据，家人和主人共用同一份；以后要每人一份时，路径和配额都要改。
- 提供方看得到所有静态插件的文件和数据。静态文件本来就不含密钥；数据和容器插件的 `/data` 一样敏感，所以提供方只能是官方系统插件。
- 能力表多了“托管”标记和 `mount` 字段，`plugin check` 和官方索引的检查要跟上。
- Hermes 写过的静态插件要重写成新形态才省下容器；旧的照样能跑。

## 没选的做法

- **核心直接认各种形态：** 核心自己托管静态文件、提供 WebDAV。核心会变厚，换不掉实现，违反 ADR-0015。
- **CSP 由提供方按清单下发：** 每个提供方都要读清单、拼 CSP，一个写错就放开了出网；换提供方还要重测这一条。
- **每个静态插件一个 nginx 容器：** 就是现状。
- **静态网页用 Agent Plugins 1.0 的包格式：** 那个格式是给 MCP 和技能用的（ADR-0066 第 2 节），里面没有网页；静态页沿用 compose 文件里的
  `x-wonton`，Agent 不用学第二种写法。
- **WebDAV 全集或 S3：** 锁、移动、部分写入是 Agent 写的页面用不上、又最难写对的部分；S3 要签名，浏览器里直接用不方便。
- **作者自己标安全等级：** 作者说不危险就显示不危险，等于没有。
- **`link` 用插件 WebView 打开：** 外部网页拿到桥就能发消息、调工具，要另做一套授权；系统的 App 内浏览器还显示网址，主人看得出到了哪里。
- **共用轻运行器（以及 Python 模板、共享依赖仓库、Pyodide 模板）：** 不做。主人的话：“不做，要跑脚本 agent 自己在容器跑不就行的吗？”
  要跑脚本的 Agent 自己在容器里跑。

## 主人已定（2026-11-11）

1. 跑代码的用容器；静态网页交给提供这个能力的插件托管，核心只存文件、按 ADR-0053 转发、记账；外部网页只存一个地址。
2. 定时任务和 webhook 先不做。
3. 静态网页做完紧接着做每插件一小块数据存储（WebDAV 整文件读写的子集），不再单独问。
4. 静态网页按清单下发 CSP，只能连声明的域名（只管网页，不影响 Agent 上网）；商店显示核心按权限算出的安全档位。
5. 外部网页可以钉进 App 的侧边栏或面板，用系统自带的 App 内浏览器打开，顶上显示网址，不给和 App 说话的通道。
6. 本篇定的细节：CSP 由网关按清单算；第一步不开放给好友；数据存储每插件 64 MiB、单文件 8 MiB；安全档位五档。
7. 第三方仓库的插件 ID 带作者前缀，另写 ADR（会推翻 ADR-0031 “没选的方案”里的一条）。
8. 不做共用轻运行器、Python 模板、共享依赖仓库和 Pyodide 模板：“不做，要跑脚本 agent 自己在容器跑不就行的吗？”
9. 远端 MCP 紧接静态网页做，照 ADR-0066。

## 关联

- ADR-0015（小组件与标准接口）
- ADR-0019（局域网）
- ADR-0023、ADR-0039（出网和代理插件）
- ADR-0031（插件来源和包的上限）
- ADR-0033（插件页、跨源隔离、Agent 写插件）
- ADR-0036（权限的风险等级）
- ADR-0043（界面文字）
- ADR-0045、ADR-0051（家人、好友共享）
- ADR-0053（能力）
- ADR-0057（可选权限）
- ADR-0065（链接按类别打开）
- ADR-0066（插件的几种形态、副作用账本）
