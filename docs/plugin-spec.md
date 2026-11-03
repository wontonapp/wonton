# 插件规范 `x-wonton`（草案 v0.3）

> 草案，M1 前定稿。字段可能调整，调整时同步更新本文和清单类型。清单类型在
> `crates/wonton-core/src/plugin/manifest.rs`，迁到 `crates/wonton-protocol` 见“实现计划”第 1 项。
> v0.1 补上了 M0-4 实测 Hermes 和 magpie 时发现的缺口，没落地的部分见末尾的“实现计划”。三处取舍已由
> 主人确认（2026-10-28）：插件默认不能出网，每个 Bot 一组容器，模型凭据默认可以用网关里的所有模型。
> v0.2 加上规范版本（`spec`）、商店信息和插件的发布与来源（ADR-0017）。
> v0.3 加上权限 `owner.identity`（ADR-0022）。

一个插件就是一个 `docker-compose.yml`，在顶层加一个 `x-wonton` 扩展字段描述它在 Wonton 里的身份、
入口、能力和权限。compose 引用的配置文件、图标和设置表单与它放在同一个目录里，合称插件包。
没有 `x-wonton` 的 compose 文件可以由用户手动导入，但必须先补全权限声明。一个插件一个 Git 仓库，仓库根目录
就是插件包，发布就是打 tag（见“发布与来源”）。可运行的示例见
[`plugins/examples/`](../plugins/examples/README.md)；默认 Agent 和模型网关见
[`plugins/hermes/`](../plugins/hermes/docker-compose.yml) 和
[`plugins/magpie/`](../plugins/magpie/docker-compose.yml)。

## 示例

```yaml
name: immich
services:
  server:
    image: ghcr.io/immich-app/immich-server:v2
    volumes:
      - ${WONTON_APP_DATA}/upload:/usr/src/app/upload
      - ${WONTON_SHARE_PHOTOS}:/photos:ro
    healthcheck:
      test: ["CMD", "immich-healthcheck"]
  mcp:
    image: ghcr.io/example/immich-mcp:1
    healthcheck:
      test: ["CMD", "wget", "-q", "--spider", "http://127.0.0.1:8080/healthz"]

x-wonton:
  spec: "0.2"
  id: app.immich.photos
  version: 2.0.0
  kind: app
  title: { zh: 相册, en: Photos }
  description: { zh: 照片备份、人脸与语义搜索, en: Photo backup with face and semantic search }
  icon: icon.png
  category: photos
  screenshots: [screenshots/timeline.png, screenshots/search.png]
  author: { name: Example, url: https://example.com }
  homepage: https://immich.app
  license: AGPL-3.0-only
  tags: [immich, 照片, 备份]
  about:
    zh: |
      手机里的照片自动备份到家里的服务器。

      按人脸、地点和画面内容搜索照片。
    en: |
      Back up the photos on your phone to the server at home.

      Search photos by face, place and what is in them.
  release_notes: { zh: 升级到 Immich 2.0。, en: Upgrades to Immich 2.0. }
  webui:
    service: server
    port: 2283
    sso: oidc
    oidc:
      redirect_paths: [/auth/login, /user-settings]
  mcp:
    service: mcp
    port: 8080
    transport: streamable-http
  permissions:
    - storage.share.photos:read
  settings: settings.schema.json
  requirements:
    arch: [amd64, arm64]
    memory: 2G
```

## 字段

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `spec` | 否 | 清单按哪一版规范写成，见“商店信息”。 |
| `id` | 是 | 反向域名形式，全局唯一，安装后不可修改。至少两段，每段由小写字母、数字和单个连字符组成，总长不超过 63。 |
| `version` | 是 | 插件包版本（语义化版本），和镜像版本无关。 |
| `kind` | 是 | `app`（普通应用）、`agent`（Agent 运行时）、`channel`（第三方 IM 渠道）、`model`（模型服务）、`system`（系统插件，仅官方，只能由安装器预装）。 |
| `title`、`description` | 是 | 多语言文本，至少提供 `zh` 和 `en`。 |
| `icon` | 否 | 插件包里的 PNG 或 SVG，见“商店信息”。官方索引要求必填。 |
| `category` | 否 | 商店分类，取“商店信息”里的一项。官方索引要求必填。 |
| `screenshots`、`author`、`homepage`、`license`、`tags`、`about`、`release_notes` | 否 | 商店信息，见“商店信息”。官方索引要求有 `author` 和 `license`。 |
| `webui` | 否 | 作为小程序打开的入口：`service`、`port`、`path`（默认 `/`）、`sso`（`oidc`、`proxy`、`none`，默认 `none`）、`oidc`、`proxy`（见“网页入口与单点登录”）。 |
| `mcp` | 否 | 给 Agent 的工具入口：`service`、`port`、`path`（默认 `/mcp`）、`transport`（`streamable-http`），可选 `tool_idle_seconds`、`tool_total_seconds`，见“工具与 MCP 总线”。 |
| `bot` | 否 | Bot 的显示名（`display_name`，多语言）和头像（`avatar`，插件包里的文件）。只能和 `matrix.bot` 权限一起声明。 |
| `bots` | 否 | 一个插件多个 Bot：`services` 列出每个 Bot 一份的服务（见“一个插件多个 Bot”）。 |
| `agent` | `kind=agent` 时必填 | `protocol`：`matrix`（Agent 自己登录 Matrix，须声明 `matrix.bot`）或 `acp`（经 ACP 桥接入）。 |
| `provides` | 否 | 向其他插件提供的能力，v0.1 只有 `models`（见“模型网关”）。只有 `kind: system` 的插件可以声明。 |
| `gpu` | 否 | 拿到显卡的服务（`services`）。要和 `devices.gpu` 权限一起声明，见“显卡”。 |
| `lan` | 否 | 在局域网上开放的端口（`ports`）。要和 `network.lan` 权限一起声明，见“局域网”。 |
| `secrets` | 否 | 插件自己的密钥：核心生成后注入，或插件运行中生成、交给核心保管（见“插件密钥”）。 |
| `permissions` | 是 | 安装时由主人确认的权限，见下表。没有权限时写空列表。 |
| `settings` | 否 | 插件包里的 JSON Schema 文件，留给 App 用原生表单渲染（ADR-0017 的“自定义接口”，预留）；在那之前插件用自己的网页做设置。 |
| `requirements` | 否 | `arch`（安装时与宿主架构比对，不符则拒绝）、`memory`、`gpu`（`required`、`optional`，见“显卡”）。 |

`x-wonton` 里出现未知字段时安装失败；`spec` 比核心支持的版本新时，先报规范版本（见“商店信息”）。

## 插件包

插件包是一个目录：`docker-compose.yml`，加上它引用的文件——`icon`、`settings`、`bot.avatar` 和
顶层 `configs` 的 `file`。`screenshots` 不算在内（见“商店信息”）。

- 引用的路径相对 compose 文件所在的目录，不能是绝对路径、不能含 `..`、不能用变量，也不能经过软链接；
  单个文件不超过 1 MiB，整个插件包（文件大小相加）不超过 2 MiB。引用的文件不存在时安装失败。
- 安装接口收 compose 文本，或者 `Content-Type: application/x-tar` 的整个插件包目录（如
  `tar -C <目录> -cf - .`）。compose 引用了包里的文件时只能传归档；归档里只能有普通文件和目录，软链接、硬链接和
  设备一律拒绝，最多 256 项。
- 核心只保存被引用的文件。安装后整个包（compose 原文和这些文件）存在核心自己的数据目录里，供审计、
  恢复和升级比对；插件的容器访问不到这个目录。
- 卷不能用相对路径挂载包里的文件（见“Compose 子集”）；容器要用的包内文件一律经 `configs` 交给它。

### 配置文件（`configs`）

写法和 compose 相同：

```yaml
services:
  app:
    configs:
      - source: app-config            # 长语法；短语法只写名字，放到 /<名字>
        target: /etc/app/config.yaml
        uid: "1000"
        gid: "1000"
        mode: 0440
configs:
  app-config:
    file: ./config.yaml               # 插件包里的文件
  motd:
    content: |                        # 内联内容，按注入的变量插值
      model=${WONTON_MODEL_DEFAULT}
```

- 顶层 `configs` 只支持 `file` 和 `content`；`external`、`environment`、`name` 报错。
- 核心在创建容器之后、启动之前，用 Docker 的归档接口把文件写进容器，不用绑定挂载。所以 `uid`、
  `gid`、`mode` 对 `file` 也生效（docker compose 对 `file` 用绑定挂载，这三项只对 `content` 生效）。
  每次创建容器都重新写入；应用要自己改写的配置，用下面的一次性服务复制进数据目录。
- `target` 是容器里的绝对路径，不能和这个服务的挂载点（卷、绑定挂载、`tmpfs`）重叠：写进挂载点的文件
  会留在卷或数据目录里，`tmpfs` 则在启动时把它盖住。`read_only: true` 的服务不能用 `configs`，Docker
  不往只读的根文件系统写文件。
- 没写 `uid`、`gid` 时，属主取服务的 `user`：数字形式的 `uid:gid` 就是它，只有 uid 时组是 0，没写
  `user` 或写的是用户名时是 root。`mode` 按八进制理解（`0440`、`"0440"`、`0o440` 都是 0440），默认
  `0444`，写权限位被忽略，不能超过 `0777`。默认所有用户可读，收紧了 `mode` 才需要关心属主。

### 一次性服务与首次写配置

被其他服务以 `condition: service_completed_successfully` 依赖的服务是一次性服务：

- 必须写 `restart: "no"`，没写或写了别的取值时安装失败：否则 Docker 会在它退出后反复重启它。退出码为 0
  算成功；非 0 时安装失败，错误里附上它的日志。
- 核心每次创建插件的容器（安装、升级、重建）时按依赖顺序先运行它，所以它必须是幂等的。
- 安装后、核心重启后检查状态时，“已退出、退出码 0”是它的正常状态（正被重新运行着也算正常），以其他退出码
  退出或容器不见了时插件标为异常。
- 不需要网络时写 `network_mode: none`。

应用会自己改写配置文件时（如 Hermes 的 `config.yaml`），由一次性服务在首次启动前把包里的配置复制进
数据目录，已存在就不覆盖。这和 Kubernetes 的 init 容器是同一种做法：

```yaml
services:
  init:
    image: *image
    entrypoint: ["/bin/sh", "-c", "test -f /data/config.yaml || cp /seed/config.yaml /data/config.yaml"]
    volumes:
      - ${WONTON_APP_DATA}:/data
    configs:
      - source: config
        target: /seed/config.yaml
    network_mode: none
    restart: "no"
  app:
    image: *image
    depends_on:
      init:
        condition: service_completed_successfully
```

## 商店信息

商店、安装确认页和 App 主屏用到的信息写在清单里，随插件包按提交固定（ADR-0017）。多语言的字段同 `title`，至少有
`zh` 和 `en`。

| 字段 | 说明 |
| --- | --- |
| `spec` | 清单按哪一版规范写成，字符串，如 `"0.2"`（YAML 里要加引号，不然读成数字）。没写时按 `"0.1"`。核心先读它：比核心支持的版本新时安装失败（`unsupported`），提示先升级核心，不再逐项报未知字段。用到 v0.2 的字段时写 `"0.2"`，用到 v0.3 的写 `"0.3"`。 |
| `icon` | 图标，见下节。 |
| `screenshots` | 截图：仓库里的 PNG 或 JPEG（路径规则同插件包里的文件），1 到 6 张，按顺序显示。截图不算插件包的一部分：不参与插件包摘要，安装时不取，上传插件包时忽略；商店的详情页要显示时，核心按同一个提交去取。 |
| `author` | 插件包的作者：`name`（必填）、`url`（HTTPS，可选）。不一定是应用的作者。 |
| `homepage` | 应用的主页，HTTPS。 |
| `license` | 应用的许可证，SPDX 表达式，如 `AGPL-3.0-only`、`MIT OR Apache-2.0`。插件包自己的许可证写在仓库的 LICENSE 里。 |
| `category` | 分类，取下表的一项。 |
| `tags` | 搜索用的关键词，最多 8 个，每个 1 到 24 个字符，不分语言。 |
| `about` | 详细介绍：多语言，纯文本，空行分段，每种语言不超过 4000 个字符。没写时详情页只显示 `description`。 |
| `release_notes` | 这个版本的更新说明：多语言，纯文本，每种语言不超过 2000 个字符，升级提示里显示。数据迁移不可逆时要写明。 |

| `category` | 中文名 | 例子 |
| --- | --- | --- |
| `ai` | AI 与模型 | Ollama |
| `agent` | 助手 | Hermes |
| `photos` | 照片 | Immich |
| `media` | 影音 | Navidrome、Jellyfin |
| `files` | 文件 | Syncthing、Samba、文件搜索 |
| `documents` | 文档 | Paperless-ngx |
| `productivity` | 效率 | 笔记、待办、日历 |
| `communication` | 通讯 | 第三方 IM 渠道 |
| `home` | 智能家居 | Home Assistant |
| `network` | 网络 | DNS、代理 |
| `security` | 安全 | 密码管理 |
| `developer` | 开发 | Git 服务 |
| `downloads` | 下载 | BT 下载 |
| `tools` | 工具 | 不属于以上各类的 |
| `system` | 系统 | 只给系统插件（`kind: system`） |

### 图标和截图

- 图标是插件包里的 PNG 或 SVG，正方形：
  - PNG 至少 256×256，建议 512×512，不超过 512 KiB；SVG 的 `viewBox` 是正方形，不超过 128 KiB。
  - 满幅：画面铺满整个正方形，不留透明边，不画圆角和阴影。App 统一裁成圆角方形，四角会被裁掉，重要内容放在中间 80%
    的范围里。
  - SVG 里的文字不渲染（要用文字就转成路径），外部资源（图片、字体、样式表）和内嵌的位图都不加载，脚本和动画不执行。
  - SVG 要是 UTF-8 文本，根元素是 SVG 命名空间里的 `<svg>`，不能有 `<!DOCTYPE>`，不收压缩过的 SVG（svgz）。
  - `<use>` 只有一个 `href` 或 `xlink:href`，写成 `#` 加 id，id 里没有空白；展开以后不超过 20000 个元素、64 层，不能循环
    引用。
  - `<style>` 里只用不带组合符的选择器：`.cls-1`、`#a`、`rect.a` 可以，`g rect`、`g > rect`、`a + b` 不行；最多 256 条
    规则。渲染器匹配后代选择器时逐层回溯，几百字节的样式表就能算上几分钟。
  - 不能用 `<pattern>`、`<marker>` 和 `<filter>`（阴影和模糊本来就不该画）；`<clipPath>` 的 `clipPathUnits` 只能是
    `userSpaceOnUse`（默认值），`<mask>` 要写 `maskUnits="userSpaceOnUse"`；每个渐变最多 64 个 `<stop>`。渲染器会按引用的
    次数复制这些功能的内容，不加限制时几 KB 的文件就能占掉几 GB 内存。
  - 不接受 HTTPS 地址：图标随提交固定，App 也不向第三方取图。
- 截图是 PNG 或 JPEG，每张不超过 1 MiB，建议长边至少 1280，横屏和竖屏的截图不要混在一个插件里。
- 核心把图片解码后重新编码再交给 App：图标是 192×192 和 512×512 的 PNG，截图是长边不超过 1280 的 JPEG，都不带原文件的
  元数据。解码和渲染在单独的进程里做，每张图最多用 512 MiB 内存、10 秒（很细的虚线这类写法渲染起来很慢）。解码不了的、
  解码后超过 4096×4096 像素的、处理时超出内存或时间的图片，和不是正方形的图标都不合法：图标不合法时安装失败，截图不合法
  时详情页里不显示它。`wonton-core plugin check` 按同样的规则、同样在单独的进程里检查。
- `bot.avatar` 的格式不变（见“Matrix Bot”）。

## 网页入口与单点登录

插件网页的地址是 `<scheme>://<slug>.<基础域名>[:端口]`，`slug` 是把 `id` 里的 `.` 换成 `-`
（`app.immich.photos` → `app-immich-photos`）。所有请求经核心网关，网关先校验主人的会话，未登录的请求
不会到达插件；转发时去掉网关自己的 Cookie 和客户端伪造的 `X-Forwarded-*`，重新设置 `X-Forwarded-For`、
`X-Forwarded-Host`、`X-Forwarded-Proto`，保留原始 `Host`，支持 WebSocket。

`webui.sso` 决定用户怎样登进应用（ADR-0022）：

| 写法 | 什么时候用 |
| --- | --- |
| `none` | 默认，新插件都从它开始。网关只校验会话，应用用自己的登录，或者本来不需要登录（下一节）。 |
| `oidc` | 应用官方支持 OIDC，由核心的身份提供方登录（见“OIDC”）。 |
| `proxy` | 应用官方支持反向代理传来的用户名，或者认一个固定的密钥，由网关代为带上（见“网关代登录”）。 |

声明 `oidc` 或 `proxy` 要同时满足：应用官方支持这种接法；接上后打开网页不会再看到应用的登录页，角色也对；不用脚本
往应用里预建账号，不用改应用的数据，不用每次启动核对。做不到就用 `none`。

### 应用自带的登录（`sso: none`）

主人装好后第一次打开网页时，在应用里自己注册管理员。网关挡在前面，只有主人打得开注册页。

- 不要带默认账号和默认密码。应用非要预建管理员时，用 `secrets` 生成密码，不要写死。
- 注册页只在 `webui` 端口上。`lan.ports` 列出的别的端口装好就开在局域网上，不能提供未认证的管理员注册。`webui` 的端口列进 `lan.ports` 时默认不开，确认页和插件设置里有开关 `lan_webui`，默认关，旁边写明“管理员注册之前，局域网里的设备和这台主机上能出网的插件可能抢先注册”（ADR-0023）。
- 应用能关自助注册时，默认关掉第一个账号以外的注册。
- 应用的登录页关不掉就留着，局域网浏览器里主人登两次。不要为了去掉它去改应用。
- 插件自己的服务要访问应用（比如 MCP 服务调应用的接口）时，用 `secrets` 里的密钥，不要用主人的账号。
- 要主人的用户名或邮箱时（比如拿用户名建系统账号），申请 `owner.identity`，不要为此声明 `sso: proxy`。
- 忘了密码按应用自己的办法找回。不删数据重装还是原来的账号，删了数据就要重新注册。

### OIDC

`sso: oidc` 的插件由核心的身份提供方（Ory Hydra，见 ADR-0009 验证结果）登录，用户不会看到第二个
登录页。清单要写明回调路径：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `webui.oidc.redirect_paths` | 是 | 应用登录回调的路径（以 `/` 开头，相对插件网页地址），核心据此登记客户端的回调地址。 |
| `webui.oidc.token_endpoint_auth_method` | 否 | 应用在令牌端点出示密钥的方式：`client_secret_basic`（默认）或 `client_secret_post`。 |

客户端可申请的 scope：`openid`、`profile`（`name`、`preferred_username`）、`email`（`email`、
`email_verified`）、`groups`（主人为 `["owner"]`，可用来映射应用内角色）、`offline_access`。
客户端 ID 就是插件 `id`，密钥在安装时生成，卸载时删除客户端。

### 网关代登录（`sso: proxy`）

网关认出主人后，在转发给插件的请求上加上清单声明的请求头或 Cookie。应用认反向代理传来的用户名时
（Navidrome、Paperless-ngx、Grafana 的 auth proxy 等）：

```yaml
webui:
  service: navidrome
  port: 4533
  sso: proxy
  proxy:
    headers:
      Remote-User: "{user}"
```

应用用自己的固定密钥登录时，比如 magpie 的网页认名为 `magpie_web_<端口>` 的 Cookie，值就是网页密钥：

```yaml
webui:
  service: magpie
  port: 3430
  sso: proxy
  proxy:
    cookies:
      magpie_web_3430: "{secret:web_key}"
```

| 字段 | 说明 |
| --- | --- |
| `webui.proxy.headers` | 请求头名 → 值模板。不能是 `Host`、`Cookie`、`Forwarded`、`X-Real-IP`、`X-Forwarded-*` 和逐跳头部。 |
| `webui.proxy.cookies` | Cookie 名 → 值模板。名字不能以 `wonton_` 开头。和浏览器发来的其他 Cookie 合并成一个 `Cookie` 头。 |

`sso: proxy` 至少写一项头或 Cookie。
值模板里可以用 `{user}`（用户名，同 OIDC 的 `preferred_username`）、`{name}`、`{email}`，以及
`{secret:<名字>}`，其余文字原样保留，如 `"Bearer {secret:api_token}"`；`{`、`}` 只能用来写占位符。主人没配邮箱时，
用了 `{email}` 的插件装不上。

- `{secret:…}` 只能引用插件自己的 `secrets` 里由核心生成（`generate`）、至少 16 字节的一项：引用不到别的插件的
  密钥，也引用不到插件交给核心保管（`collect`）的秘密。网关按值从响应里找出并去掉它（见下），值太短会误伤。
- Cookie 的值只能用 `{secret:…}` 和固定文字，文字里不能有空白、引号、逗号、分号和反斜杠：用户名、显示名和邮箱
  可能有 Cookie 里不能出现的字符。请求头的值里不能有控制字符。不合规的清单在预览和安装时就被拒绝。
- 只有网关认出主人的请求才加凭据，规则和其他插件网页一样：局域网浏览器要有这个插件子域名的会话，经隧道的
  已配对设备看设备身份。没认出主人的请求不转发：打开页面跳到核心登录，其余请求 401。
- 插件子域名彼此同站，一个插件的页面能让浏览器带着会话向另一个插件发请求。所以网关按 Fetch Metadata
  的资源隔离策略，只认这些请求的登录状态：`Sec-Fetch-Site` 为 `same-origin` 或 `none`；顶层导航
  （`Sec-Fetch-Mode: navigate`，方法为 `GET` 或 `HEAD`）；没有 `Sec-Fetch-Site` 时，没有 `Origin`
  或 `Origin` 就是插件地址。其余请求按没登录处理，同样不转发。凭据只加在这个插件自己的子域名上。
- 网关先删掉客户端发来的同名请求头和 Cookie（名字不分大小写）再加上声明的值，客户端无法伪造或覆盖。
- 凭据不回到浏览器：插件响应里设置这些 Cookie 的 `Set-Cookie`（名字不分大小写，包括没写名字、浏览器会读成
  同名的）由网关去掉；值里带着 `{secret:…}` 原文的响应头（比如插件把密钥写回别的 Cookie 或跳转地址）也去掉。
  网关不改响应体：插件不能把凭据写进页面。
- 主人的显示名这类值写不进请求头时（比如带换行），网关如实报错（500 `proxy_credentials`），不转发。
- 用请求头认用户的应用，“信任的代理地址”只写核心的地址，不写 `0.0.0.0/0` 和整个网段：
  出网插件的容器和接在 `--lan` 网络上的服务，宿主上的进程和经宿主代理转发的连接都到得了，别的插件能借此冒充主人（ADR-0023）。

  ```yaml
  environment:
    ND_REVERSEPROXYUSERHEADER: Remote-User
    ND_REVERSEPROXYWHITELIST: ${WONTON_CORE_ADDRESS}/32
  ```

  这条只管 `proxy.headers` 的模板用了 `{user}`、`{name}` 或 `{email}` 的插件。只信任核心，指 `webui.service` 的环境变量引用了 `${WONTON_CORE_ADDRESS}`，值里也没有 `0.0.0.0/0`、`::/0`。做不到时（应用没有这个选项，或者作者放宽了），按 `webui.service` 怎么开：

  | 插件 | `plugin check` | 核心 |
  | --- | --- | --- |
  | 出网 | `--official` 判不合格，改用 OIDC 或 `{secret:…}` | `webui.service` 的局域网端口同下一行（第三方插件） |
  | 不出网，开了局域网端口 | 提醒，不判不合格 | `webui.service` 的这些端口默认不开，服务也不接 `--lan` 网络。确认页和插件设置里有开关 `lan_webui`，默认关，旁边写明“局域网里的设备，以及这台主机上能出网的插件，不用密码就能以你的身份打开它”，主人打开后才开 |
  | 不出网，也不开局域网端口 | 在索引的审核里登记后通过：它的网络里只有核心和本插件的服务，宿主也连不上 | 不提示 |

  不走单点登录的局域网服务（比如 Ollama 的 API）不适用这一节的任何规则，用不用密钥由主人决定（ADR-0023）。
- 用 `{secret:…}` 的插件分不出是谁在用：每个能打开这个小程序的人都以同一个应用身份登录。

## 权限

| 权限 | 含义 |
| --- | --- |
| `storage.share.<名称>:read` / `:write` | 读或写某个共享文件夹，如 `photos`、`documents`、`music`、`downloads`。设备上传的文件在 `transfers` 里（ADR-0020）。 |
| `storage.home:read` / `:write` | 读或写当前用户的私人空间。 |
| `models` | 访问模型网关（magpie），见“模型网关”。 |
| `tools` | 经核心的 MCP 总线调用核心和其他插件的工具，见“工具与 MCP 总线”。 |
| `matrix.bot` | 以 Bot 身份收发消息，见“Matrix Bot”。 |
| `notify` | 给主人发通知。 |
| `owner.identity` | 知道主人的用户名和邮箱，见“安装时注入的变量”。 |
| `network.egress` | 能访问外网，也能访问主人的局域网，安装确认按这句写。见“网络”。 |
| `network.lan` | 在局域网上开放 `lan.ports` 列出的端口，见“局域网”。高权限。 |
| `network.host` | 使用宿主网络。高权限，不开放（ADR-0019）。 |
| `devices.gpu` | 使用显卡，见“显卡”。高权限。 |
| `devices.<路径>` | 访问宿主设备，如 USB。高权限。 |
| `docker.socket` | 访问 Docker socket。最高权限，默认拒绝，需要单独确认。 |

高权限在安装确认时单独醒目展示。插件不声明的能力一律不给。安装请求必须逐项确认清单里的全部权限，
少一项或多一项都拒绝安装。

核心兑现 `storage.share.*`、`network.egress`（见“网络”）、`network.lan`（见“局域网”）、`models`（见“模型网关”）、
`matrix.bot`（见“Matrix Bot”）、`tools`（见“工具与 MCP 总线”）、`owner.identity` 和 `devices.gpu`（见“显卡”）。共享文件夹在数据目录的 `shares/<名称>` 下，
只授权读时必须以 `:ro` 挂载，而且只能整个挂载，不能挂其中的子路径（ADR-0011）。其余权限清单里可以声明，
但安装时如实报“尚未支持”。

## 安装时注入的变量

| 变量 | 何时注入 | 说明 |
| --- | --- | --- |
| `WONTON_APP_ID` | 总是 | 插件 `id`。 |
| `WONTON_APP_DATA` | 总是 | 插件私有数据目录（宿主路径）。绑定挂载的子目录由核心逐级预先创建，属主见下文；路径上有软链接或不是目录时拒绝安装（ADR-0011）。 |
| `WONTON_SHARE_<名称>` | 授权了该共享文件夹 | 共享文件夹路径（大写，`-` 换成 `_`，如 `WONTON_SHARE_PHOTOS`）。 |
| `TZ`、`PUID`、`PGID` | 总是 | 时区与运行用户。`TZ` 是 IANA 时区名，如 `Asia/Shanghai`、`UTC`。 |
| `WONTON_APP_URL` | 声明了 `webui` | 插件网页的公开地址，不带末尾 `/`。应用生成回调地址、外部链接时使用。 |
| `WONTON_OIDC_ISSUER`、`WONTON_OIDC_CLIENT_ID`、`WONTON_OIDC_CLIENT_SECRET` | `webui.sso: oidc` | OIDC 客户端配置。 |
| `WONTON_OIDC_AUTH_URL`、`WONTON_OIDC_TOKEN_URL`、`WONTON_OIDC_USERINFO_URL` | `webui.sso: oidc` | 各端点地址，给不支持发现文档的应用（如 Grafana）。 |
| `WONTON_CORE_ADDRESS` | `webui.sso: proxy` | 核心在这个插件网络里的地址（网段的最后一个可用地址，如 `172.18.255.254`），核心容器重建后不变。写进应用的“信任的代理地址”，见“网关代登录”。 |
| `WONTON_OWNER_NAME` | `owner.identity`；`spec` 低于 `0.3` 的清单，`webui.sso` 是 `oidc`、`proxy` 时也给 | 主人的用户名，装好后不变，和 OIDC 的 `sub`、`preferred_username` 以及代登录模板的 `{user}` 同值，见下文。 |
| `WONTON_OWNER_EMAIL` | 同上，且主人配了邮箱 | 主人的邮箱，和 OIDC 的 `email`、代登录模板的 `{email}` 同值。值是建容器时的快照，见下文。 |
| `WONTON_MODEL_BASE_URL` | `models` | 模型网关 OpenAI 兼容接口的地址，以 `/v1` 结尾，如 `http://models.wonton.internal:8080/v1`。 |
| `WONTON_MODEL_ANTHROPIC_BASE_URL`、`WONTON_MODEL_GEMINI_BASE_URL` | `models`，且网关提供该接口 | Anthropic Messages、Gemini 接口的地址，按各自 SDK 的约定不带 `/v1`。 |
| `WONTON_MODEL_API_KEY` | `models` | 只属于本插件（多 Bot 时只属于这个 Bot）的网关凭据。 |
| `WONTON_MODEL_DEFAULT` | `models` | 默认模型名，由网关决定（magpie 是路由组 `group/wonton-default`）。 |
| `WONTON_MCP_URL` | `tools` | MCP 总线的地址（Streamable HTTP）：核心在插件网络里的别名，如 `http://mcp.wonton.internal:8080/mcp`。 |
| `WONTON_MATRIX_HOMESERVER` | `matrix.bot` | Matrix 客户端接口的地址：核心在插件网络里的别名，如 `http://matrix.wonton.internal:8080`。 |
| `WONTON_MATRIX_USER_ID`、`WONTON_MATRIX_ACCESS_TOKEN` | `matrix.bot` | Bot 的 Matrix 用户 ID 和访问令牌。 |
| `WONTON_MATRIX_OWNER` | `matrix.bot` | Bot 主人的 Matrix 用户 ID，用作白名单：私人 Bot 只接受主人的指令。 |
| `WONTON_MATRIX_HOME_ROOM` | `matrix.bot` | Bot 的主房间：核心替主人建好的、和 Bot 的加密私聊，定时任务的结果发到这里。 |
| `WONTON_SECRET_<名字>` | `secrets` 里由核心生成的项 | 插件密钥，名字转成大写，如 `WONTON_SECRET_WEB_KEY`。 |
| `WONTON_BOT_NAME`、`WONTON_BOT_DATA` | `bots.services` 里的服务 | Bot 名和这个 Bot 的数据目录（宿主路径）。 |
| `WONTON_LAN_ADDRESSES` | `network.lan` | 局域网端口开在哪些地址上，逗号分隔，如 `192.168.0.193`。地址变了核心重建开端口的容器，见“局域网”。 |
| `WONTON_PROXY_URL` | `network.egress` | 出口容器的地址 `http://egress.wonton.internal:3128`，主人没配代理时是空串。给有自己代理设置、不读 `HTTP_PROXY` 的程序用，见“网络”。 |

- 这些变量只用于 compose 插值（`${VAR}`、`${VAR:-默认值}` 等，`$$` 表示字面量 `$`），不会自动进入
  容器环境；需要的变量在对应服务的 `environment` 里显式引用，密钥只给用得到它的容器。
- 引用了没有注入的变量且没有默认值时，安装失败，而不是像 docker compose 那样替换成空串。例如没有声明
  `sso: oidc` 却引用 `WONTON_OIDC_CLIENT_SECRET`，或者模型网关不提供 Anthropic 接口却引用
  `WONTON_MODEL_ANTHROPIC_BASE_URL`。核心不把自己的环境变量传给插件。
- 地址和令牌以注入值为准，插件不要自己拼：主机名、端口和格式都可能变。
- 主人的用户名和邮箱同样以注入值为准，不要写死或按规则拼。没申请 `owner.identity` 的插件拿不到它们，安装不多给一份
  个人信息，引用了就装不上，错误写明“插件没申请 `owner.identity`”。规范版本升到 `0.3` 的插件，靠 `sso` 拿身份变量的要补上
  这项权限。应用登录时从 OIDC 或代登录模板拿到的身份不归这项权限管，预览怎么列见 ADR-0022。
- 用户名是固定 ID，装好后不变，插件不用处理它变化的情况。它只含小写字母、数字、`_` 和 `-`，以字母开头，最长 32 个
  字符，不是常见的系统用户名，可以直接用来建系统账号；镜像里自带的用户（`postgres`、`node`）可能重名，要插件自己
  处理。应用按用户名或 OIDC 的 `sub` 认人，不要按邮箱认：邮箱会变。
- 主人没配邮箱时不注入 `WONTON_OWNER_EMAIL`，OIDC 也没有 `email` 声明。应用非要邮箱时写 `${WONTON_OWNER_EMAIL:?}`，
  插件这时装不上，错误写明“主人还没配邮箱”和怎么配：OIDC 登录要 `email` 声明的应用也这样写，不要等到登录时才失败。
  能没有邮箱的写 `${WONTON_OWNER_EMAIL:-}`。已装的插件升级或重建时遇到这种情况怎么显示，见 ADR-0022。
- 主人改了邮箱或显示名（怎么改见 ADR-0022）之后，OIDC 声明和代登录的请求头马上换成新值，容器里的 `WONTON_OWNER_EMAIL`
  还是旧值：核心重启只核对插件容器，不重建。升级或重装（可以不删数据）时新建的容器才拿到新值；局域网地址变化只重建
  `lan.ports` 里的服务，同一个插件的其他服务还是旧值。应用里已有账号的资料由主人在应用里改，插件不写对账代码。
- 核心不注入 `WONTON_OWNER_ID`：OIDC 的 `sub` 就是主人的用户名，另起一个变量只是同一个值。

**数据目录属主**：核心预先创建的绑定挂载目录（`WONTON_APP_DATA` 下）交给谁，看挂载它的服务的
`user`：

- 没写 `user`，或 `user` 是 root（`root`、`0`、`0:0`）：交给 `PUID:PGID`。以 root 启动、再按
  `PUID`/`PGID` 降权的镜像（如 LinuxServer.io 的镜像）就是这种情况。
- `user` 是数字形式的 `uid:gid`：交给这个 uid:gid。镜像固定以某个 uid 运行、不认 `PUID`/`PGID` 时
  （如 magpie 的 65532），就写上它：`user: "65532:65532"`。
- `user` 是 root 以外的用户名，或只有 uid：安装失败，提示改成 `uid:gid`。几个服务以不同的属主挂载同一个
  目录时也安装失败。

应用数据目录本身和路径上的中间各级目录交给 `PUID:PGID`。核心只改这些目录本身的属主，不递归改里面已有的
文件。

能以任意 uid 运行的镜像优先写 `user: "${PUID}:${PGID}"`，宿主上的文件就归主人的用户。这里不另设清单
字段：compose 的 `user` 已经说明了容器以谁运行。

## 预装插件

核心镜像里带着两类插件包，核心启动后在后台装上，授予清单声明的全部权限：装核心就是主人对它们的确认。目录里
每个子目录一个插件包，一个装完再装下一个；先装系统插件，再装默认插件（默认 Agent 要用模型网关）。

| | 目录 | 什么时候装 | 主人卸掉以后 |
| --- | --- | --- | --- |
| 系统插件（`kind: system`，如模型网关） | `WONTON_SYSTEM_PLUGINS` | 每次启动，没装就装 | 下次启动装回来 |
| 默认插件（如默认 Agent，ADR-0007） | `WONTON_DEFAULT_PLUGINS` | 只装一次 | 不再自动装 |

- 系统插件只能这样装，API 装不了。默认插件是普通插件（不能是 `kind: system`），主人可以照常卸载、重装。
- 已经装着的照常核对，不重装。系统插件目录里的版本比装着的新时，等核心自己的升级结束（ADR-0025）后，按“升级”的步骤升级它（ADR-0017），装核心就是对它
  的确认。默认插件只在第一次从目录装，之后和别的官方插件一样由官方索引提示升级、主人确认，核心不随镜像升级它。
- 默认插件装好、或者主人卸掉它以后，核心在自己的数据目录里记下（`core/plugins/preinstalled/<id>.json`），此后不再
  自动装：主人换掉默认 Agent，核心不会把它装回来。
- 装不上时如实标为异常、写明原因，不影响核心启动：
  - 插件包本身的问题（`invalid_input`、`unsupported`，如不支持本机架构）和本机没有可用的显卡（`gpu_unavailable`）
    重试也一样，不重试，下次启动再试；
  - 其余原因（Docker、身份提供方、Matrix、模型网关还没就绪，核心处理图片的子进程起不来，拉镜像失败，没通过健康检查）
    自动重试，间隔逐次加长，最长 10 分钟；等待时原因里写着试了几次、会自动重试。拉不下镜像时原因写成“拉取镜像失败
    （镜像）：Docker 的原话”（错误类别 `image_pull_failed`）；
  - 等待重试时主人卸掉这个异常的插件，就不再重试；默认插件同样记下，不再自动装。卸载开始了就算，到点时卸载还没
    做完也不再重试。
- 局域网状态页逐个列出这些插件里没在运行的（安装中，或者异常和原因的第一行）：它们每台主机都有，列出来不泄露主人
  装了什么。原因后面的容器日志可能有聊天内容，只在主人登录后的插件列表里给。其他插件只给个数。

## 模型网关

模型请求只经模型网关（ADR-0008，默认 magpie）。网关是一个声明了 `provides.models` 的系统插件（见“预装插件”）；
核心把声明了 `models` 的插件接到它上面，给每个插件单独的凭据。

### 使用方

- 地址指向核心在插件网络里的别名 `models.wonton.internal`，由核心转发给网关（见“网络”），如
  `http://models.wonton.internal:8080/v1`。插件和网关不在同一个网络里，只能经这个别名访问；网关按来源地址认出
  请求来自哪个插件网络，没申请 `models` 的插件得到 403 `permission_denied`，网关没在运行时得到 503
  `models_unavailable`。请求和响应都流式转发，不缓冲，也不设总超时。主机名一定带点：
  Hermes 把不带点的主机名当成本机模型服务，会去探测 Ollama 接口，而 magpie 正好兼容这组接口，结果每个
  请求都被加上 Ollama 专用的参数（ADR-0008 实测）。
- 请求 `WONTON_MODEL_DEFAULT`，不写死 provider 的模型名：换 provider 或模型只在网关里改。默认模型还没
  配好时，签发照常成功，请求得到网关自己的错误，插件如实显示，不换用别的模型。
- 凭据默认可以用网关里的所有模型：插件和网关在同一台机器上，能访问就该能用。凭据由核心自动签发和注入，
  插件不用配置，作用是按插件记账和单独吊销。要限定某个插件能用的模型、设额度或限定账号，主人在网关自己的
  网页里改；核心不同步这些设置。
- 网关没装或没就绪时，安装失败（`models_unavailable`）并说明是网关的原因，不注入半套变量。

> **已定：凭据默认可以用所有模型**（主人 2026-10-28 确认）。没选的方案是只允许默认模型：最小权限、花费
> 可控，但需要特定模型（如视觉模型）的插件要主人去网关里放开。

### 提供方（`provides.models`）

```yaml
x-wonton:
  kind: system
  provides:
    models:
      service: magpie
      port: 3425
      apis:                           # 接口 → 基础路径
        openai: /v1
        anthropic: /
        gemini: /
      default_model: group/wonton-default
      credentials:
        issue: [sh, /wonton/credentials.sh, issue, "{name}"]
        revoke: [sh, /wonton/credentials.sh, revoke, "{name}"]
```

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `service`、`port` | 是 | 网关监听的服务和端口。核心只转发到这个端口，网关容器的其他端口（如网页）使用方访问不到。 |
| `apis` | 是 | 提供的接口及其基础路径。必须有 `openai`，可选 `anthropic`、`gemini`；决定注入哪些地址变量，注入的地址都不带末尾的 `/`。 |
| `default_model` | 是 | 注入给使用方的 `WONTON_MODEL_DEFAULT`。网关插件自己保证它存在，核心不认识网关的模型（magpie 的做法见 ADR-0008）。 |
| `credentials.issue`、`credentials.revoke` | 是 | 签发和吊销凭据的命令（argv，不经 shell），见下节。 |
| `credentials.service` | 否 | 在哪个服务的容器里执行命令，默认是 `service`。 |

同一台服务器同时只能有一个插件提供 `models`，装第二个时安装失败（`conflict`）。提供方看得到全部模型请求和
provider 的 key，所以只有 `kind: system` 的官方插件可以声明。

### 凭据的签发与吊销

参照 Docker 凭据助手（credential helper）的约定：核心在网关的容器里执行声明的命令，秘密值走标准输入，
以退出码判断结果，不解析输出。magpie 的实现见
[`plugins/magpie/credentials.sh`](../plugins/magpie/credentials.sh)。

- 占位符：`{name}` 是凭据名。没有 `bots` 时只有插件级的凭据，名字是插件 `id`；有 `bots` 时 Bot 服务用这个 Bot 的
  凭据（`<插件 id>/<Bot 名>`，M1 只有 `default`），还有全插件一份的服务时再签发一个插件级的。核心逐项替换 argv，
  不经 shell，在 `credentials.service`（默认 `service`）的容器里以容器的默认用户执行。
- `issue`：标准输入给出凭据的值（一行）。网关里没有这个名字的凭据时，新建一个，不限定模型；
  已有时不改动它（主人可能调整过）。退出码 0 表示成功。
- `revoke`：删除这个名字的凭据；本来就没有也算成功。
- 凭据的值由核心生成（32 字节随机数的十六进制）并存在密钥库。网关的数据丢了或重装了，核心用同一个值
  重新签发，使用方不用重启。
- 时机：
  - 安装使用方时签发，再用这个凭据请求一次 `GET <openai 基础地址>/models`，返回 200 才算成功。返回 401
    说明网关里的同名凭据不是这个值，核心先 `revoke` 再 `issue` 一次；还是不行就报错。
  - 网关每次启动后（安装、升级、核心重启后的恢复），核心按同样的步骤为所有使用方对一次账；对账失败的使用方
    标为异常、写明原因。
  - 卸载使用方（删不删数据都一样）或删掉一个 Bot 时吊销，再从密钥库删除；重装是一个新值。网关已经卸载了就没有
    可吊销的；网关装着但没在运行时吊销不了，卸载如实失败。
- 命令超时 30 秒。失败时错误里附上命令的标准错误输出，并区分“网关未就绪”和“网关拒绝”。

## Matrix Bot

Bot 是一个 Matrix 身份加上驱动它的插件（ADR-0007）。核心没有接入 Matrix 时，声明了 `matrix.bot` 的插件
安装失败（`matrix_unavailable`）。安装时核心：

1. 分配 Bot 的用户名：插件 `id` 的最后一段（`agent.echo` 是 `echo`）。这个名字的账号已经存在（家人、别的插件的
   Bot、停用的账号），或者和主人、核心的管理账号重名时，依次换成 `<名字>-2`、`<名字>-3`……JWT 登录对已有的账号
   是直接登录进去，所以核心不登录任何已有的账号，只有这个插件记录在案的 Bot 账号能沿用。插件以
   `WONTON_MATRIX_USER_ID` 为准。
2. 用 JWT 代 Bot 登录一台新设备，首次登录即建号（ADR-0009）；不申请刷新令牌。Tuwunel 的 `access_token_ttl`
   只作用于申请了刷新令牌的客户端，这样拿到的访问令牌不会过期，而 Agent 多半不会自己刷新令牌。插件只拿到
   令牌，Bot 账号也没有密码。
3. 按 `bot` 设置显示名（取中文，没有时取英文；没写 `bot.display_name` 时用 `title`）和头像（png、jpg、gif 或
   webp）。
4. 准备 Bot 的主房间：Bot 建一个加密房间（`trusted_private_chat`，主人和 Bot 都是管理员），先不邀请主人。没有主房间
   时 Hermes 会在每个新会话的第一条回复里插一段设置提示（ADR-0008）。
5. 注入 `WONTON_MATRIX_*` 变量。
6. 插件装好、健康检查都通过后，把主房间交给主人：核心用 JWT 临时登录主人的一台设备（主人的账号在第一次登录时才建，
   所以先登录再邀请），Bot 邀请主人，主人加入，双方的 `m.direct` 都记着它，然后退出登录。私聊这时才出现在主人的
   聊天列表里：早交出去的话，主人在 Bot 启动前发的话它收不到。装不上时主人那边不留私聊；交出之后才失败的（如写
   安装记录失败），回滚时主人和 Bot 都退出这个房间。

- `WONTON_MATRIX_HOMESERVER` 是 `http://matrix.wonton.internal:<网关端口>`，核心在声明了 `matrix.bot` 的插件网络里的
  别名。网关按连接的来源地址认出请求来自哪个插件网络（不算网桥自己的地址，那是宿主那一侧），只转发声明了
  `matrix.bot` 的插件的请求，而且只转发 Matrix 客户端接口（`/_matrix/client/`、`/_matrix/media/`），管理接口和
  联邦接口不转；其他来源得到 403 `M_FORBIDDEN`。插件不和 Tuwunel 共用网络。
- 账号和主房间建好后一直沿用：不删数据卸载后重装、安装失败后重试，都是同一个账号。Bot 还在主房间里，并且主人也在、
  或者还没交给过主人（上次安装没装完）时沿用它；主人退出过就新建一个。设备每次安装新建，安装失败回滚时吊销。
- 插件的容器启动时，第一次安装的主人还不在主房间里。要往主房间里发消息的插件，等主人加入（`m.room.member` 事件）
  之后再发：之前发的消息，房间密钥没有发给主人的设备，主人解不开。
- 令牌对应一台设备，核心不主动轮换。吊销设备、删掉 Bot 时令牌立即失效（ADR-0009）。核心重启时核对 Bot 的
  设备还在，被吊销了就把插件标为异常，重装（可以不删数据）重新签发；Bot 换了一台设备，要重新建加密身份。
- 重建容器时沿用设备和令牌（升级；第一次配上或取消出网代理，ADR-0023）。核心不单独保存令牌，它只在 Docker 的容器配置里，所以从旧容器读回：
  1. 用占位值渲染一次，找出哪些服务的哪些环境变量引用了 `WONTON_MATRIX_ACCESS_TOKEN`。
  2. 从旧容器读出这些变量。全部读到之后才删旧容器。
  3. 旧容器不在，或者令牌写在 `command`、`configs` 里读不回来：不删、不重建，把插件标为异常，提示重装。不悄悄签新设备，换设备要重新建加密身份。
- Agent 自己生成的加密恢复密钥之类，用 `secrets` 的 `collect` 交给核心保管（见“插件密钥”）。
- 卸载插件时吊销 Bot 的所有设备；选择删除数据时再停用 Bot 账号，账号随之退出所有房间。停用不可恢复，所以
  保留数据重装时沿用原账号，房间和历史都还在；删除数据后重装是一个新账号。卸载结果的 `bot` 字段写着 Bot 的
  用户 ID、吊销的设备数和是否停用。

### 一个插件多个 Bot（`bots`）

M1 只有默认 Bot，家人的私人 Bot 在 M2（ADR-0007）。格式现在定下，M2 时插件不用改结构。

```yaml
x-wonton:
  bot:
    display_name: { zh: Hermes, en: Hermes }   # 新 Bot 的默认显示名
  bots:
    services: [init, gateway]                  # 这些服务每个 Bot 一份
```

- 每个 Bot 有自己的账号、令牌、主房间和模型凭据（凭据名 `<插件 id>/<Bot 名>`）。安装时核心建默认 Bot
  `default`；主人新建 Bot 时，核心按 `bots.services` 为它再建一组容器；删掉 Bot 就删掉这组容器并吊销它的
  凭据。其余服务全插件一份。
- 在这些服务里，`WONTON_MATRIX_*`、`WONTON_MODEL_API_KEY` 是这个 Bot 的值；`WONTON_BOT_NAME` 是 Bot 名
  （小写字母、数字和单个 `-`），`WONTON_BOT_DATA` 是 `${WONTON_APP_DATA}/bots/<Bot 名>`。
- 隔离：Bot 服务挂载应用数据目录时只能挂自己的 `${WONTON_BOT_DATA}`，也不能用命名卷，这样一个 Bot 读不到
  别的 Bot 的记忆和凭据。
- 依赖：Bot 服务可以依赖同一个 Bot 的服务和全插件一份的服务；全插件一份的服务不能依赖 Bot 服务。
  `webui`、`mcp`、`provides` 不能指向 Bot 服务。
- Bot 服务在插件网络里的别名是 `<服务名>-<Bot 名>`，同一个 Bot 的服务之间也用它互访，如
  `http://sidecar-${WONTON_BOT_NAME}:8080`。
- 全插件一份的服务引用 `WONTON_MATRIX_*` 时安装失败；引用 `WONTON_MODEL_API_KEY` 时拿到插件级的凭据
  （凭据名是插件 `id`）。
- 没有 `bots` 时插件只有一个 Bot，变量对所有服务都一样。

> **已定：每个 Bot 一组容器**（主人 2026-10-28 确认），即上面的写法，沿用单 Bot 的变量名。隔离最好：
> Hermes 的文档写明 profile 不隔离文件系统，同一个容器里的 Bot 能读到彼此的记忆和凭据，而家人的私人
> Bot 必须互不可见（ADR-0007）。代价是内存，Hermes 空闲时约 210 MB 一个。没选的方案是一个容器跑多个 Bot（Hermes
> 的多 profile）：容器级的环境变量只作用于 Hermes 的默认 profile，其他 profile 只读自己的
> `profiles/<名字>/.env`，核心要按插件声明的模板把每个 Bot 的凭据写成文件；省内存，但只适合同一个人的
> 多个 Bot。同一个人的多个 Bot 要不要另外支持 B，M2 按内存实测再定。

## 工具与 MCP 总线

Agent 经核心的 MCP 总线调用工具（ADR-0006、ADR-0016）：总线把核心自己的工具和所有在运行的插件声明的工具汇总成
一个 MCP 服务器，按调用方授权，敏感的调用先请主人确认。

### 提供工具（`mcp`）

```yaml
x-wonton:
  mcp:
    service: mcp           # 说 MCP 的服务
    port: 8080
    path: /mcp             # 默认 /mcp
    transport: streamable-http
    tool_idle_seconds: 600    # 可选，见“工具调用的期限”
    tool_total_seconds: 1800  # 可选
```

- 核心经插件网络连这个服务（同网页入口），它只对核心可见；插件在运行时它的工具才出现在总线上。只说 stdio 的
  MCP 服务器要在镜像里自带一个转成 Streamable HTTP 的桥（如 supergateway，见文件搜索插件
  `geraldpeng6/wonton-plugin-files`）。
- 工具在总线上叫 `<slug>__<工具名>`，如 `app-wonton-files__search_files`。核心连插件的 MCP 服务用 `initialize` 握手
  （MCP 2025-11-25）。
- 没有标 `readOnlyHint: true`（MCP 的工具注解）的工具算敏感：总线先请主人确认，确认了才转给插件。会改数据的工具
  不要标成只读。不会改数据的工具要标上，否则 Bot 每次调用都要主人确认。
- 工具返回的内容交给 Agent，Agent 把它当不可信输入（ADR-0011）。除了下面的文件引用，总线不改内容。

**列工具**

- 每个 Agent 列工具时，总线并发地问所有插件。一个插件最多 15 秒，包括没连上时的握手；最多 50 页、2000 个工具、8 MiB。超时或超限的插件，它的工具这次不出现，原因在 `wonton__list_plugins` 的 `tools_error` 里。翻页翻回同一个 `cursor` 算插件的错。

**连接**

- 一个请求的回答不对（非 2xx、`Content-Type` 不对、JSON 解析不了、回答的 `id` 不是这个请求的）算插件对这一个请求的错误，连接照用，别的请求不受影响。连接断了才重连。
- 连续 3 次带期限的请求（`tools/list`、`resources/read`、`tools/call`）没等到回答，中间也没有任何请求拿到回答，总线就当这个会话卡死了：取消它，下次新建一个，有状态的 stdio 子进程跟着丢。同时超时的算一次。有更早开始的 `tools/call` 或 `resources/read` 还在跑时，超时不算：串行的服务器做长调用时 `tools/list` 排在后面，这段时间工具不出现，但会话和调用都留着。

**工具调用的期限**

- 总线等一个 `tools/call` 有两个期限：空闲期限（默认 10 分钟，`tool_idle_seconds`），收到这个调用的 `notifications/progress` 就重新计时，总线的请求都带 `progressToken`；总期限（默认 30 分钟，`tool_total_seconds`），不管有没有进度。都可以声明得更长，最多一周。长的工具要么发进度，要么把两个期限都写长：只写总期限，空闲期限先到；空闲期限超过总期限（不写时按 30 分钟算）的清单不合法。
- 到期后总线给插件发 `notifications/cancelled`，工具要尽量停下来。这时不知道工具做到了哪一步，结果算不确定，不算失败：Agent 收到 `isError: true`，`_meta` 的 `app.wonton/failure` 写明原因（`tool_timed_out`，或者 `session_replaced`、`connection_lost`、`result_too_large`）；需要确认的调用，卡片标“结果不确定”。
- 请求没到插件（连接已经关了、连不上插件）不算不确定，算失败。
- 只读的调用，Agent 发了 `notifications/cancelled` 或者关闭了会话，总线把取消传给插件。确认过的调用不跟着 Agent 走（ADR-0016）：它取消了或者关闭了会话都照样执行，结果在卡片上。

**工具结果里的文件**（ADR-0020）

工具结果是交给 Agent 的。Agent 没有主人的登录，打不开插件网页，也看不到共享文件夹。文件要这样返回，核心把引用换成一个编号，
Agent 用编号调 `wonton__view_file`、`wonton__share_file`，主人的设备按链接经核心的文件服务下载。

| 文件 | 返回 |
| --- | --- |
| 小图（不超过 1 MiB），Agent 看一眼就够 | `image` 块。它没有编号，分享不了；要能分享原图，再给一个下面的引用。 |
| 共享文件夹里的文件 | `resource_link`，`uri` 是 `wonton-share:<共享文件夹>/<相对路径>`，路径按 URI 规则做百分号编码。 |
| 插件自己的文件 | `resource_link`，`uri` 是插件的 MCP 资源，插件实现 `resources/read`。大文件在 `_meta` 里加内网地址，支持 `Range` 才能续传和流式播放。 |

```json
{
  "content": [
    { "type": "text", "text": "找到 2 张 7 月的照片" },
    {
      "type": "resource_link",
      "uri": "wonton-share:photos/2025/07/IMG_1234.jpg",
      "name": "IMG_1234.jpg",
      "mimeType": "image/jpeg",
      "size": 4404019
    },
    {
      "type": "resource_link",
      "uri": "immich://assets/3f2a/original",
      "name": "IMG_1235.heic",
      "mimeType": "image/heic",
      "_meta": { "app.wonton/internal_url": "http://server:2283/internal/assets/3f2a/original" }
    }
  ]
}
```

- `name`、`mimeType`、`size` 用来告诉 Agent 和显示卡片。`name` 没写时取路径的最后一段。下载时的类型由核心决定，不可信的类型一律按附件下载。
- 共享文件夹必须是这个插件申请到的（读或写都算），写别的共享文件夹不会登记。路径不能是绝对路径，不能含 `.`、`..` 或空段，路径上不能有软链接，
  必须是普通文件。第一段是共享文件夹的名字，不是容器里的挂载点：`${WONTON_SHARE_PHOTOS}:/photos:ro` 里的 `/photos/a.jpg` 写成
  `wonton-share:photos/a.jpg`。
- `app.wonton/internal_url` 只有核心会访问：主机名必须是这个插件 compose 里的服务名，协议是 `http`，不跟随跳转，不用 IP 字面量。不声明时核心用
  `resources/read`，最多读 32 MiB。这个键在交给 Agent 前去掉。
- `uri` 是 `http`、`https` 的 `resource_link` 不是文件引用，原样交给 Agent。
- 引用登记不了时（没有授权、路径不合法等），Agent 看到的是一句说明原因的文字，不是这个引用。
- 不要：
  - 返回宿主路径；
  - 用 `blob`、`image`、`audio` 塞大文件：超过 1 MiB 的块，总线换成一句说明原因的文字；
  - 自己往聊天里发文件：发不发给主人由 Agent 决定；
  - 一次返回几百个文件：一个结果最多登记 100 个，要分页。

### 使用工具（`tools`）

- 申请了 `tools` 的插件得到 `WONTON_MCP_URL`，以 Streamable HTTP 连接。网关按来源地址认出调用方（同“模型网关”），
  不在插件网络里的请求得到 403 `unauthenticated`，没申请 `tools` 的得到 403 `permission_denied`。
- M1 申请了 `tools` 的插件看到全部工具；按 Bot 的白名单放行在 M2（一个插件多个 Bot 时，每个 Bot 另发令牌）。
- 插件装上、卸掉、恢复运行时工具列表随之变化，总线给连着的客户端发 `notifications/tools/list_changed`。
- 核心自己的工具：

  | 工具 | 敏感 | 说明 |
  | --- | --- | --- |
  | `wonton__list_plugins` | 否 | 已装的插件、版本、状态和网页地址；工具列不出来时带 `tools_error`。 |
  | `wonton__search_catalog` | 否 | 按关键词在官方索引里找插件：ID、名字、说明和要申请的权限。 |
  | `wonton__install_plugin` | 是 | 按官方索引里的 ID 安装，授予它声明的全部权限。声明了高权限的插件只能由主人在 App 里装。 |
  | `wonton__uninstall_plugin` | 是 | 卸载，`delete_data` 为真时同时删除它的数据。 |
  | `wonton__view_file` | 否 | 按文件编号返回图片预览，看不了的类型报错（ADR-0020）。 |
  | `wonton__share_file` | 否 | 把编号指向的文件分享给主人，返回文件链接，原样放进回复（ADR-0020）。 |

- 插件目录就是官方索引（见“发布与来源”）。Bot 只能装官方索引里的插件，不能装社区插件和自己写的 compose。
- 敏感的调用会一直等到主人回答或者确认过期（10 分钟），Agent 的工具超时要比这长。确认卡片发在核心和主人的“Wonton”
  私聊里，主人在 Wonton App 里回答（ADR-0016）。主人拒绝或者过期时工具返回错误（`isError: true`），说明原因。

## 插件密钥（`secrets`）

```yaml
x-wonton:
  secrets:
    web_key:                          # 核心生成，注入为 WONTON_SECRET_WEB_KEY
      generate: { bytes: 32 }
    matrix_recovery_key:              # 插件运行中生成，交给核心保管
      collect: { service: gateway, path: /opt/data/platforms/matrix/recovery-key.txt }
```

名字由小写字母、数字和 `_` 组成，以字母开头。每项必须有 `generate` 或 `collect` 之一，不能两个都有。
两种密钥都存在核心的密钥库里（核心数据目录下，文件权限 0600，插件的容器访问不到）：卸载时保留数据就一起
保留，重装时沿用（数据库密码之类已经写进了数据，换了就对不上）；选择删除数据时一起删除。

**核心生成（`generate`）**

- `bytes` 是随机字节数，1 到 1024，默认 32；`encoding` 是 `hex`（默认，小写）或 `base64`（带填充、不换行），
  结果和 `openssl rand -hex <bytes>`、`openssl rand -base64 <bytes>` 的格式相同。
- 首次安装时生成，以 `WONTON_SECRET_<名字大写>` 注入；升级时保留，升级新增的项在升级时生成。已有的值
  不随 `bytes`、`encoding` 的修改重新生成。
- 做法同 Runtipi 的 `random` 字段：随机生成后保存，不像 Umbrel 的 `APP_SEED` 那样从主种子派生——只恢复了
  应用数据、没恢复种子时，派生出的值就对不上。
- v0.1 不轮换，也不在 App 里显示（待定）。

**插件生成、核心保管（`collect`）**

插件运行中自己生成、丢了就无法恢复的秘密，如 Hermes 自动建交叉签名后写下的恢复密钥。清单声明它在哪个
服务的容器里、哪个文件：

- 核心在安装时所有服务就绪后、以及核心每次启动核对插件时，用 Docker 的归档接口读取。路径在容器里解析，
  软链接出不了容器，最后一级是软链接时不跟随；只接受不超过 64 KiB 的普通文件，是目录、软链接或超过上限时
  安装失败（核心启动核对时则把插件标为异常）。文件还不存在时不算失败，下次核对时再试；读到以后每次核对，
  变了就更新。
- 核心不修改、不删除插件里的这个文件，也不注入回去。
- 声明在 Bot 服务上时，每个 Bot 各存一份。

## 显卡（`devices.gpu`）

要用显卡的插件（生图、生视频、本地模型）申请 `devices.gpu` 权限，并在 `gpu.services` 里写明哪些服务拿到显卡。
交给容器什么由核心按宿主实际有的显卡决定，compose 里自己写的 `devices` 一律拒绝（ADR-0018）：插件写不了宿主路径，
一份清单在 AMD 和 NVIDIA 的主机上都能装。

```yaml
name: comfyui
services:
  comfyui:
    image: ghcr.io/example/comfyui:1
    shm_size: 8g
    healthcheck:
      test: ["CMD", "curl", "-f", "http://127.0.0.1:8188/"]
  web:
    image: ghcr.io/example/comfyui-proxy:1

x-wonton:
  id: ai.example.comfyui
  version: 1.0.0
  kind: app
  title: { zh: ComfyUI, en: ComfyUI }
  description: { zh: 用工作流生图、生视频, en: Image and video generation with workflows }
  webui:
    service: web
    port: 8188
  gpu:
    services: [comfyui]
  permissions:
    - devices.gpu
  requirements:
    gpu: required
```

| 字段 | 说明 |
| --- | --- |
| `gpu.services` | 拿到显卡的服务，至少一项，不能重复，都要在 `services` 里。其余服务什么都不拿。 |
| `requirements.gpu` | `required`（没写时的默认）：本机没有可用的显卡时装不上。`optional`：照常装，不给设备，插件自己退回 CPU。 |

三者缺一不可：写了 `gpu` 或 `requirements.gpu` 却没申请 `devices.gpu`，或者申请了 `devices.gpu` 却没写 `gpu.services`，
清单都不合法。

核心按宿主的显卡交给这些服务，容器里的路径和宿主相同：

| 宿主 | 交给容器什么 |
| --- | --- |
| AMD、Intel 等开源驱动的显卡（有 DRM 渲染节点） | 设备 `/dev/dri`（整个目录）；有 KFD（amdgpu 的 ROCm 计算接口）时还有 `/dev/kfd`。容器加入这些设备节点所属的组（宿主 `video`、`render` 组的 GID），以非 root 用户运行的服务才开得了设备。 |
| NVIDIA 官方驱动 | Docker 的 `DeviceRequests`（同 `docker run --gpus all`），由宿主上的 NVIDIA Container Toolkit 把全部 NVIDIA 显卡和驱动库放进容器。 |

- 两种显卡都有时都给。核心只给显卡本身：没有 `privileged`、`cap_add`、`security_opt`、`ipc: host`；要大一点的共享内存
  写 `shm_size`。
- 有没有可用的显卡看 sysfs 里的驱动和设备节点，不看 `nvidia-smi`、`rocminfo` 这类命令装没装。`required` 的插件装不上时
  错误类别是 `gpu_unavailable`（HTTP 422），原因分清：没有找到显卡；有显卡但没有可用的驱动（没加载驱动，或者是
  nouveau）；有 NVIDIA 驱动但 Docker 没有登记 `nvidia` 运行时（没装 NVIDIA Container Toolkit）；核心在容器里却没挂载
  宿主的 `/dev`。
- 安装预览的说明（`notices`）里有一行写明拿显卡的服务，以及本机会交给它们什么（厂商、设备、组）或者为什么给不了；`required`
  而给不了时同时是装不了的原因（`blocked`）。确认卡片列出拿显卡的服务。
- 核心在容器里看不到宿主的 `/dev`，compose 把它只读、不递归地挂到 `/host/dev`（`WONTON_HOST_DEV`，`/dev/shm` 这类子挂载
  不带进来）；没挂时申请显卡的插件装不上，原因写明，核心不拿容器自己的 `/dev` 当宿主的。
- 装好后核心不再盯着显卡：显卡拔了、驱动换了，容器自己会出错。可运行的检查插件见
  [`plugins/examples/gpu-check/`](../plugins/examples/gpu-check/README.md)。

## 局域网（`network.lan`）

要让局域网里的电脑、手机直接连上来的插件（Samba、Syncthing），申请 `network.lan` 权限，并在 `lan.ports` 里逐项写明开放哪个
服务的哪个端口。为什么这样做、没选宿主网络的理由和实测结果见 [ADR-0019](adr/0019-lan-services-for-plugins.md)。

```yaml
name: samba
services:
  samba:
    image: ghcr.io/example/samba:4
    volumes:
      - ${WONTON_SHARE_DOCUMENTS}:/shares/documents
  web:
    image: ghcr.io/example/samba-web:1
    environment:
      LAN_ADDRESSES: ${WONTON_LAN_ADDRESSES}

x-wonton:
  id: org.example.samba
  version: 1.0.0
  kind: app
  title: { zh: 文件共享, en: File sharing }
  description: { zh: 用 SMB 在局域网里共享文件夹, en: Share folders over SMB on the LAN }
  webui:
    service: web
    port: 8080
  lan:
    ports:
      - { service: samba, port: 445 }
  permissions:
    - network.lan
    - storage.share.documents:write
```

| 字段 | 说明 |
| --- | --- |
| `lan.ports[].service` | 开端口的服务，要在 `services` 里，不能写 `network_mode: none`。 |
| `lan.ports[].port` | 端口号。宿主端口和容器端口相同，不做映射。 |
| `lan.ports[].protocol` | `tcp`（默认）或 `udp`。 |

清单不合法的情况：`lan` 和 `network.lan` 只写了一个；`ports` 为空、端口为 0 或者同一端口和协议写了两次；把 `mcp`
的 TCP 端口列进来（它只经核心访问）；同时申请了 `matrix.bot`（地址变了核心要按记录重建容器，而它不保存 Bot 的访问令牌）。

- **端口开在哪里**由核心决定：默认是宿主默认路由所在网卡的私有 IPv4 主地址，主人可以在 App 的局域网设置里改选网卡或地址
  （管理接口 `/_wonton/api/lan`，见 ADR-0019 的“接口”），或者给核心设 `WONTON_LAN_ADDRESSES` 指定。端口只发布在这些地址上，
  从不发布到 `0.0.0.0` 或 `::`。选出的地址经 `WONTON_LAN_ADDRESSES` 交给插件，插件在自己的网页上告诉用户怎么连。
- **选不出地址就装不上**：错误类别是 `lan_unavailable`（HTTP 422），原因写明是哪一种（没有默认路由、默认路由走的是虚拟网卡、
  网卡上没有私有 IPv4 地址、检测失败）。预览的 `notices` 里每个端口一行，写明开在哪块网卡的哪个地址上；端口被别的插件的
  `lan.ports` 占着、或者选不出地址时，`blocked` 写明原因。被宿主自己的服务占着的端口，Docker 启动容器时报错，安装附上原文并回滚。
  `lan_webui` 关着、没有要开的端口时，不检查地址，不报 `lan_unavailable`。
- **发布在哪个网络上**：声明了 `network.egress` 的插件直接发布在自己的插件网络上。没声明的，核心另建 `<插件网络>--lan`
  （关掉地址伪装的桥接网络），只把开着局域网端口的服务以网关优先级 1 接进去（`webui` 的端口要看 `lan_webui`，见“网关代登录”和“应用自带的登录”），端口发布在它上面；这些服务仍在插件网络里，
  主动连外网和局域网的其他主机仍然不通。
- **地址变了自动跟上**：装了这类插件时，核心启动后和之后每 5 分钟检测一次。地址变了，只重建 `lan.ports` 里的服务的容器，
  数据不动，正在用的连接会断开；选不出地址时插件标为异常，端口不改开到别的地址上，地址回来后恢复。主人改了设置也一样重建。
- **不做发现**：核心不发 mDNS、WS-Discovery、SSDP，也不转发组播和广播，用户按地址连。
- **身份由插件自己验证**：局域网的连接不经核心登录，用不用密钥、开不开鉴权由主人决定，核心和 `plugin check` 不拦。`webui` 的端口也可以列进来，这时连接绕过网关，没有 TLS；要不要主人打开开关见“网关代登录”和“应用自带的登录”。插件经 IPv4 地址看得到真实的客户端地址；主人选了 IPv6 地址时，
  那上面的连接由 docker-proxy 转发，插件看到的是网桥的网关地址。

## Compose 子集

核心经 Docker API 渲染插件，只接受下面的字段，其余字段一律报错：

- 顶层：`name`、`services`、`volumes`（只支持不带参数的本地卷）、`configs`（见“插件包”）、`x-wonton`，
  以及任意 `x-` 扩展字段（如 `x-casaos`）；`version` 会被忽略。
- 服务：`image`、`command`、`entrypoint`、`environment`、`volumes`、`configs`、`restart`（未写时默认
  `unless-stopped`）、`healthcheck`、`depends_on`（含 `service_started`、`service_healthy`、
  `service_completed_successfully`）、`user`、`working_dir`、`labels`（`wonton.` 前缀保留给核心）、
  `hostname`、`expose`、`init`、`read_only`、`shm_size`、`tmpfs`、`stop_signal`、`stop_grace_period`、
  `pull_policy`（默认 `missing`）、`network_mode`（只接受 `none`），以及 `x-` 扩展字段。`image` 既没写
  标签也没写摘要时按 `latest` 拉取，和 compose 一致。
- 卷：短语法和长语法（`bind`、`volume`、`tmpfs`）。宿主路径只能是 `${WONTON_APP_DATA}`（或其子目录）和
  已授权共享文件夹的根目录，不能含 `..`，不支持相对路径；同一插件的绑定挂载源不能嵌套（同一个目录可以挂进
  多个服务）；命名卷必须在顶层 `volumes` 声明。理由见 ADR-0011 的“插件隔离的约束”。
- 命名卷在 Docker 里叫 `wonton-<slug>--<卷名>`。同名卷已存在时：属于本插件（上次卸载时保留的数据）就
  直接复用；属于其他插件或不是核心创建的，拒绝安装。安装失败回滚时只删除本次新建的卷，保留的卷不删。
- 明确拒绝并给出原因：`ports`（局域网端口写在 `lan.ports`，见“局域网”）、`network_mode`（`none` 以外的取值）、`networks`、`privileged`、
  `devices`（显卡由核心按 `gpu.services` 交给服务，见“显卡”）、`cap_add`、`security_opt`、`pid`、`ipc`、
  `container_name`、`build`、`env_file`、`extends`、`links`、`external_links`、`volumes_from`，以及顶层的
  `networks`、`secrets`。
- `environment` 里只有键没有值的写法（取宿主环境变量）不受支持。

安装时核心按 `depends_on` 顺序启动容器，等每个长期运行的容器健康（有健康检查时）或稳定运行、每个一次性
服务以 0 退出，再确认网页入口能响应，全部通过才算安装完成；任一步失败都按相反顺序回滚已创建的资源，并在
错误里附上容器最近的日志。

## 健康检查

- 长期运行的服务都要有健康检查，而且检查的是真正可用，不只是进程还活着：Hermes 要 Matrix 已经连上
  （用它自己的 `/health/detailed`），magpie 用自带的 `magpie healthcheck`。
- 写在 compose 的 `healthcheck` 里。镜像自带的 `HEALTHCHECK` 也算数，但官方索引的插件要在 compose 里
  显式写出，便于审核，也随插件版本锁定。
- 写上 `start_period` 和 `start_interval`：安装时能尽快判断就绪，启动慢时又不会误判失败。
- 检查只能用镜像里已有的命令，distroless 之类的镜像里没有 `curl`、`wget`。
- 官方索引的插件缺健康检查时不收录（`wonton-core plugin check --official` 检查）。导入的应用和社区插件
  没有健康检查时照常安装，状态如实显示为“运行中，健康未知”，不显示“健康”。
- 核心在安装时和每次启动时检查，插件列表里运行中的插件带上最近一次的结果：每个长期运行的服务都有健康检查
  而且都通过了是 `healthy`；有服务没有健康检查，或者核心启动时检查还没通过，是 `unknown`（只知道在运行）。
  检查失败的插件标为异常。两次检查之间核心不盯着插件，状态可能已经变了。

## 网络

- 每个插件有独立的 Docker 网络 `wonton-<slug>`，容器叫 `wonton-<slug>--<服务名>`，服务之间用服务名互访。服务名只能有小写字母、数字、`-` 和 `_`，不能含 `--`，不能以 `-` 结尾。核心加入所有插件网络并负责
  路由。插件之间、插件和 Tuwunel、插件和模型网关都不共用网络。开发环境的核心设了实例名（`WONTON_INSTANCE`）时，网络、
  容器和命名卷的名字在 `wonton-` 后多一段 `<实例名>--`（ADR-0011）。
- 核心以低于默认的网关优先级（`gw_priority: -1`）接入插件网络，默认路由和发布的端口始终在核心自己的网络上。
  容器接了几个网络时，Docker 让优先级最高的端点提供默认路由和发布端口，一样高时取网络名按字典序排第一的，
  而插件网络排在核心的网络前面（`wonton-` 对 `wonton_`）。不压低的话，核心的出口跟着能出网的插件网络走，
  卸载这个插件时经发布端口的连接全部断开（#90）。
- 核心在插件网络里以带点的别名提供下面几个入口，每个别名只出现在需要它的插件网络里。核心按请求从哪个
  插件网络进来判断来源，只转发它有权访问的入口。`.internal` 是 ICANN 保留给私有网络的顶级域，不会和
  公网域名冲突。

  | 别名 | 出现条件 | 转发到 |
  | --- | --- | --- |
  | `auth.<基础域名>` | `webui.sso: oidc` | 身份提供方的公开端点；浏览器和插件看到同一个签发方地址。 |
  | `models.wonton.internal` | `models` | 模型网关声明的服务和端口。 |
  | `matrix.wonton.internal` | `matrix.bot` | Tuwunel 的 Matrix 客户端接口。 |
  | `mcp.wonton.internal` | `tools` | 核心的 MCP 总线。 |

  不让声明 `models` 的插件直接加入模型网关的网络：那样这些插件彼此也连通了，还能访问网关容器的所有端口。
  经核心转发只多一跳（核心网关的 p50 约 0.27 ms，见 ADR-0006），却守住了插件之间的隔离，核心也能按插件
  限流和审计。Matrix 同理。
- 出网：没有 `network.egress` 的插件网络是 Docker 的内部网络（`internal: true`）：不能访问外网和局域网，
  也解析不了外部域名（Docker Engine 26 起不再为只连内部网络的容器转发外部 DNS 查询，CVE-2024-29018），
  上面的入口照常可用。网桥还设成 `com.docker.network.bridge.gateway_mode_ipv4/ipv6=isolated`（Engine 28 起）：
  网桥在宿主上没有地址，插件连宿主自己监听的服务也访问不到，宿主也不能直接访问插件，核心经它加入的插件网络
  访问插件。声明了 `network.egress` 时插件网络是普通的桥接网络。`network.egress` 是一个开关：不区分外网和局域网，
  也不按服务细分，理由和接受的风险见 [ADR-0023](adr/0023-egress-proxy.md)。
  - 核心直接跑在宿主上（开发时，没设 `WONTON_CORE_CONTAINER`）要经网桥访问插件，所以不设 `isolated`，
    插件还能访问宿主上监听所有地址的服务；插件经自己网络的网关访问核心的身份提供方入口（内部网络没有
    默认路由，`host-gateway` 走不通）。
  - 核心启动核对插件时检查它的网络：和清单、核心的运行方式不符时（比如核心升级前装的插件网络能出网）把
    插件标为异常，提示重装（可以不删数据）。只是还没给核心留地址的，核心自动迁移，容器不重建（ADR-0023）。

  > **已定：默认不能出网**（主人 2026-10-28 确认）。要出网的插件声明 `network.egress`，安装时列在权限里：
  > 相册、文档这类插件连不上外网，就带不走数据，符合“住在自己家里”；代价是多一项确认，导入的 CasaOS
  > 应用要自动加上它。没选的方案是默认能出网（Docker、CasaOS、Umbrel 都是这样）：兼容最好，但插件不声明也能把
  > 数据发出去，只能靠审核。
- 核心的地址：每个插件网络的最后一个可用地址留给核心，容器自动分配只用网段的前一半，核心容器重建后地址不变（`WONTON_CORE_ADDRESS`，理由和建法见 [ADR-0023](adr/0023-egress-proxy.md) 的“只信任核心的地址”）。
- 走代理：要走代理才能出网的主机，主人配一次代理地址，插件不用管（设计、错误和实测见 [ADR-0023](adr/0023-egress-proxy.md)）。出口容器 `wonton-egress` 接在每个出网插件的网络上，别名 `egress.wonton.internal`，是这些网络里除核心和插件服务之外唯一的容器。连宿主时用网关地址或局域网地址都行，和不配代理时一样。
  - 核心给声明了 `network.egress` 的插件的每个服务（`network_mode: none` 的除外）写好 `HTTP_PROXY`、`HTTPS_PROXY`、`http_proxy`、`https_proxy`、`NO_PROXY`、
    `no_proxy` 和 `NODE_USE_ENV_PROXY=1`。地址是出口容器 `http://egress.wonton.internal:3128`，插件看不到主人的代理。`NO_PROXY` 已经包含回环、
    `.wonton.internal`、`auth.<基础域名>`、本插件的服务名和私有网段。没配代理时这些变量都不写。这是核心唯一直接写进容器环境的一组变量。
  - compose 里写了同名变量时以 compose 为准，写成空就是这个服务不走代理。不要写具体的代理地址：官方索引不收，换一台主机也用不了。
  - 程序不读这些变量但有自己的代理设置（SearXNG 的 `outgoing.proxies`、Java 的 `-Dhttp.proxyHost`）时，在配置里引用 `${WONTON_PROXY_URL}`。
  - 健康检查不走代理：核心改写调用 wget 和 shell 的写法，包括镜像自带的 `HEALTHCHECK`（规则见 ADR-0023）。入口脚本里的 busybox wget 不看 `no_proxy`，访问内部地址要自己加 `-Y off`。
  - 插件网络不是信任边界：主人的代理放行私有地址时，别的出网插件能经它连到出网插件和 `--lan` 网络上的服务的任何端口。建议：应用支持密码或密钥的内部服务（数据库、Redis），用 `secrets` 生成并设上。不支持的（比如 Ollama）不因此拒收，`lan.ports` 里的端口也不在此列。
  - 经代理出错时：403 是出口容器拒绝了这个目标（内部或私有地址走到了代理）；503 是连不上主人的代理（立刻返回是被拒绝，等 15 秒是没响应）；CONNECT 200 之后 TLS 断开是代理连不上目标。
- 核心要求 Docker Engine 28 及以上，并且不开 `allow-direct-routing`：28 起局域网里的其他主机不能再经路由
  直接访问容器里没发布的端口（moby#48724），插件网页才真的只能经核心网关访问；`sso: proxy` 信任网关加的
  请求头，靠的就是这一点。核心启动时检查 Engine 版本，低于 28 时报错退出，不降级运行；
  `allow-direct-routing` 经 Engine API 查不到，由安装器保证。
- 插件之间默认互不可达；需要互通时在清单里声明依赖（待定，M1 前补充）。
- 插件不向宿主暴露端口，只有 `network.lan` 的插件把 `lan.ports` 列出的端口发布在局域网地址上（见“局域网”）；
  网页一律经核心网关访问。
- 插件的容器、网络和卷都带标签 `wonton.plugin=<id>`，卸载时据此清理；卸载时选择删除数据才会删除
  命名卷、应用数据目录和插件密钥，共享文件夹永不删除。
- 安装和卸载不随请求取消：请求断了（手机切网、隧道断开），操作照样做完，结果看插件列表里的状态。同一个插件正在
  安装或卸载时，对它的新请求得到 409 `conflict`，说明正在安装或卸载。

## 发布与来源

### 插件仓库

- 一个插件一个 Git 仓库，仓库根目录就是插件包：`docker-compose.yml` 和它引用的文件。仓库里的其他文件（README、LICENSE、
  Dockerfile、源码、截图、测试）核心不取，截图只在商店的详情页里显示。
- 发布：把清单的 `version` 改成新版本，提交，打 tag `v<version>` 推上去。tag 和清单的版本不一致时装不上。带预发布后缀的
  版本（如 `v1.2.0-beta.1`）只能按 tag 明确安装，不进商店，也不作为升级提示。发布过的 tag 不要移动：已经装上的不受
  影响，但核心会提示主人这个 tag 被改过。
- 插件自己的镜像由作者构建，推到公开的镜像仓库，compose 按摘要引用（`名字:标签@sha256:…`，多架构镜像写清单列表的
  摘要）。核心不构建镜像。
- 公开仓库加上 topic `wonton-plugin`，几分钟后就出现在商店的社区插件里，标为“未审核”。想进官方商店，向官方索引提 PR
  （见“官方索引”）。

### 来源

| 写法 | 装的是 |
| --- | --- |
| `github:<owner>/<repo>@v1.2.0` | 这个 tag |
| `github:<owner>/<repo>` | 最新的正式版本：版本号最大的、不带预发布后缀的 `v<语义化版本>` tag |
| `github:<owner>/<repo>@<40 位提交号>` | 这个提交，开发时用；清单的版本照常校验，不检查升级 |

- 核心经 git 协议 v2 的 `ls-refs`（`https://github.com/<owner>/<repo>.git`）把 tag 解析成提交，之后按提交取文件：先取
  `docker-compose.yml`，再取它引用的文件。取到后和上传的插件包走同一套解析和校验。
- 私有仓库核心取不到（核心不存 GitHub 令牌），开发时从本地目录上传（见“开发与校验”）。
- 安装记录写下来源、tag、提交、插件包摘要和每个服务实际用的镜像摘要。

### 插件包摘要

插件包的每个文件一行 `<sha256 的十六进制>  <路径>`（两个空格），按路径的字节序排列，每行以换行结尾，即 `sha256sum`
的输出格式；路径相对包的根目录，不带 `./`，`docker-compose.yml` 也在内。这段文字的 sha256 就是插件包摘要，写成
`sha256:<十六进制>`。它只取决于文件的内容，和文件从哪里取、怎么打包无关；`wonton-core plugin check` 会打印它。

安装和升级的预览返回摘要，安装请求必须带回同一个摘要，对不上时得到 409 `changed`：主人确认的就是装上的。

### 镜像

- 官方索引的插件，所有服务的 `image` 都按摘要写。
- 社区插件和上传的插件包可以只写标签，预览时如实提示“镜像没有按摘要固定”。核心拉取后记下实际的镜像摘要，以后重建
  容器（如升级失败回滚）都用它，不再按标签拉。

### 官方索引

公开仓库 `geraldpeng6/wonton-store` 的 `index.json`：

```json
{
  "format": 1,
  "plugins": [
    {
      "source": "github:geraldpeng6/wonton-plugin-files",
      "version": "0.2.0",
      "commit": "<40 位提交号>",
      "files": {
        "docker-compose.yml": "<sha256 的十六进制>",
        "icon.svg": "<sha256 的十六进制>"
      },
      "compose": "<docker-compose.yml 的原文>"
    }
  ]
}
```

- `format` 是索引格式的版本，核心读不懂的格式按取不到处理，并提示升级核心。
- 一个插件一项，`source` 和插件 ID 都不重复，商店按这里的顺序列出官方插件。`files` 是插件包的全部文件，`compose` 必须
  和 `files` 里的 `docker-compose.yml` 一致。核心按 `files` 核对取到的每个文件，所以文件可以从任何地方取（GitHub、
  jsDelivr、核心内置的副本）。
- 条目用 `wonton-core plugin index-entry <插件包目录> github:<owner>/<repo> <提交号>` 生成，不手写：目录是那个提交的
  检出（核心不连 Git，不核对），条目的 JSON 写到标准输出；加 `--index <索引目录>` 时直接写进本地的索引目录，同一个
  来源的旧条目换掉。
- 收录要求：`wonton-core plugin check --official` 通过，即有 `icon`、`category`、`license`、`author`，所有服务的 `image`
  按摘要固定，长期运行的服务都有健康检查；申请的权限和插件做的事相符；镜像来自上游官方或作者可复现的构建。
- 核心镜像内置一份索引和其中插件包的文件。开发镜像和测试用本地的索引目录代替（`WONTON_STORE_INDEX`）：`index.json`
  加上按 sha256 存放的文件（`blobs/<sha256 的十六进制>`）。

### 社区插件

- 公开仓库、topic `wonton-plugin`、有 `v<语义化版本>` tag：核心按 GitHub 星数列出，标为“未审核”。
- 核心读最新正式版本的 `docker-compose.yml`；没有 tag、清单解析不了的仓库不列出。官方索引里的仓库、插件 ID 和官方插件
  相同的仓库也不列出。
- 列表缓存 10 分钟。主人可以关掉社区插件（`WONTON_STORE_COMMUNITY=off`）。

### 升级

- 官方插件按官方索引里的版本、社区插件按仓库最新的正式版本 tag 提示升级；上传的插件包不提示。只升不降。
- `id` 不能变。升级保留数据、插件密钥、Bot 账号和设备、模型凭据、OIDC 客户端，只换容器；新版本装不好就回滚到旧版本
  的容器。
- 新版本申请的权限和装着的不同时，主人要重新确认全部权限，新增的标出来。
- 数据迁移由应用自己在启动时做；不可逆的迁移要在 `release_notes` 里写明，回滚后旧版本可能读不了迁移过的数据。

### 开发与校验

- 校验：`wonton-core plugin check <目录>`，发布的核心镜像里就有。和安装时同一套解析和校验，加上商店信息和图片的检查，
  最后打印插件包里的文件和插件包摘要；`--official` 再按官方索引的要求检查。不连 Docker，也不联网。
  - 插件包解析不了时只报这一条（和安装时报的一样）；解析得了时列出所有问题。
  - 不合格（退出码 1）：装不上的（核心还不支持的权限、渲染不了的 compose），图标或截图处理不了。
  - 提醒（退出码 0）：镜像没有按摘要固定，长期运行的服务没有健康检查，没有图标，图标的角是透明的，截图的长边不到
    1280，横屏和竖屏的截图混在一起，`kind: system`。`--official` 时前三项也算不合格，另外要有 `category`、`license`、
    `author`。只看得到 compose 里写的健康检查，镜像自带的看不到。
  - 用法不对时退出码 2。
- 打包：`wonton-core plugin pack <目录>` 把插件包打成 tar 写到标准输出，只有 compose 文件和它引用的文件，README、截图
  这些不在里面；解析不了时和 `check` 一样报错，退出码 1。
- 开发时从本地目录安装：把 `pack` 打出的 tar 上传到安装接口（见“插件包”），装上后是本地来源，不检查升级。改了以后
  上传到升级接口（`POST /_wonton/api/plugins/{id}/upgrade`），保留数据。
- 模板仓库 `geraldpeng6/wonton-plugin-template` 带开发环境和检查、发布的脚本。开发环境用发布的核心镜像在本机起一套
  核心，和正式安装是同一份 compose，换掉项目名、核心的容器名（`WONTON_CORE_CONTAINER`）和实例名
  （`WONTON_INSTANCE`），几套开发环境可以共用一台 Docker，同一个插件也能同时装在几套里。

## 兼容 CasaOS 应用

导入 CasaOS 格式（`x-casaos`）的应用时，核心按以下规则转换，再请主人确认：

- `x-casaos.title`、`description` 映射到 `title`、`description`。`x-casaos.icon` 是图片地址：核心导入时把它取下来放进
  插件包，取不到或不合法时没有图标。
- `x-casaos.index` 与 `port_map` 映射到 `webui`。
- `/DATA/AppData/$AppID` 下的卷映射到 `WONTON_APP_DATA`。
- 其余宿主路径（如 `/DATA/Media`）映射到共享文件夹，并生成对应的读写权限，必须由主人确认。
- `network_mode: host`、`privileged`、`devices` 生成对应的高权限。
- `ports` 去掉，网页入口改由网关提供。
- 加上 `network.egress`：CasaOS 应用都假定能出网。
- 没有健康检查的服务照常导入，状态显示为“健康未知”。

## 待定问题

- 插件间依赖与通信的声明方式。
- 官方索引的签名（minisign），签名后索引也能经镜像和国内的静态托管分发。
- 一个仓库放多个插件（`github:<owner>/<repo>/<路径>`）。
- 多个入口和快捷方式（`shortcuts`）、插件给主人发通知（`notify`）的写法（ADR-0017 的“自定义接口”）。
- `settings.schema.json` 支持的 JSON Schema 子集。
- 插件升级时的数据迁移钩子。
- 网关上 HTTPS 以后，插件网页地址和 OIDC 签发方地址的形式（M0 只有 http）。
- 共享 Bot（M2）的白名单：多个用户，而且会在运行中变化，环境变量表达不了。
- 多 Bot 时的存储授权：共享文件夹是插件级的授权，家人的私人空间（`storage.home`）要按 Bot 的主人分别
  挂载（M2）。
- `network.lan` 能不能和 `matrix.bot` 一起申请：Bot 的令牌能从旧容器读回（见“Matrix Bot”）后，地址变了也能重建。
- 插件密钥在 App 里的查看（如应用的管理员密码）与轮换；收存的秘密在恢复数据时怎样交还插件。
- compose 的 `secrets`：以文件形式交给容器的密钥，给遵循 `*_FILE` 约定的镜像用。

## 实现计划

v0.1 新增的部分按下面的顺序落地，每项一个 PR，完成后从本节删掉，全部完成后删掉本节。

1. **接口：清单类型迁到 `wonton-protocol`**（只改接口，先合并）。
   - `crates/wonton-core/src/plugin/manifest.rs` 的类型和校验移到 `crates/wonton-protocol` 的 `plugin`
     模块；`wonton-core` 加依赖并改为引用它。
   - 新增：`Permission::NetworkEgress`。`secrets`（`generate`、`collect`）、`bots`、`provides.models`
     （`plugin/provides.rs`）、`Sso::Proxy` 和 `webui.proxy`（模板校验在 `plugin/proxy_auth.rs`）已在核心里，随其他类型
     一起迁移。
   - 校验一起迁移：`{secret:…}` 模板只引用 `generate` 的项、只有 `kind: system` 能声明 `provides`、`provides`
     引用的服务存在，这几条已在核心里。
   - 契约测试：`plugins/` 下所有插件包都能解析，几类违规各被拒绝；App 的安装确认页以后用同一套类型
     （需要 UniFFI 导出的类型按它的约束调整）。
12. **多 Bot**（M2，每个 Bot 一组容器）：按 `bots.services` 为每个 Bot 实例化容器、限制 Bot 服务的挂载、
    按 Bot 签发凭据和收存秘密。
13. **出网走代理：接口**（ADR-0023）：`wonton-protocol` 的 `egress` 模块，只改接口，先合并。
14. **出网走代理：渲染**：注入变量和 `WONTON_PROXY_URL`，改写健康检查。都是纯函数，写行为测试。
15. **出网走代理：出口容器**：gost 的生命周期、接入网络、拒绝名单和放行网段（包括额外直连的网段）、Bot 令牌读回和重建、探测和错误、设置接口。依赖 Docker 的测试标 `#[ignore]`，加进 CI 的 Docker 作业。
16. **出网走代理：安装和核心**：安装脚本检测；两份 compose 给核心传入 `WONTON_EGRESS_PROXY`、`WONTON_EGRESS_PROXY_DETECTED` 并加 `host.docker.internal:host-gateway`；核心显式用这个地址，同一个 PR 清空核心的代理变量（#121 里核心的那部分）。之后 vane 删掉写死的代理地址。
17. **代登录只信任核心的地址**（ADR-0023 的“只信任核心的地址”，不依赖 13 到 16）：建网络时固定核心的地址、核心接入和核对、自动迁移已装插件的网络、注入 `WONTON_CORE_ADDRESS`、`plugin check` 的提醒和检查、局域网端口的开关 `lan_webui`、清单放开 `webui` 的端口、两份 compose 给核心加 `net.ipv4.ip_forward=0`（写 Docker 集成测试）。之后 navidrome 等 `sso: proxy` 的插件改配置。

v0.2（ADR-0017）的部分同样每项一个 PR：

2. **商店**：核心镜像内置一份索引和其中的插件包；
   官方索引和社区插件的列表落盘，重启后先用；详情页的截图。
3. **升级**：检查新版本、升级预览、升级和回滚，系统插件随核心升级；安装记录写下每个服务实际用的镜像摘要，回滚时用它。
4. **客户端核心**：`PluginsViewModel` 和插件相关的操作。
6. **插件移出主仓库**：预装的 magpie、Hermes 改由锁文件固定。

ADR-0020 的部分：

- **文件服务**：先提只改接口的 PR（`wonton-protocol` 的 `file` 模块）。核心的编号、下载接口和 tusd；总线登记编号、改写结果，`wonton__view_file`、
  `wonton__share_file`；客户端核心和 App 的文件卡片与传输；Hermes 的引导；文件搜索插件返回引用，并申请 `transfers`。
