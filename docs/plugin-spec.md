# 插件规范 `x-wonton`（草案 v0.1）

> 草案，M1 前定稿。字段可能调整，调整时同步更新本文和清单类型。M0 期间类型在
> `crates/wonton-core/src/plugin/manifest.rs`，定稿后迁到 `crates/wonton-protocol`。
> v0.1 补上了 M0-4 实测 Hermes 和 magpie 时发现的缺口。新增的部分核心还没有实现，步骤见末尾的
> “实现计划”。三处取舍已由主人确认（2026-10-28）：插件默认不能出网，每个 Bot 一组容器，模型凭据默认可以用
> 网关里的所有模型。

一个插件就是一个 `docker-compose.yml`，在顶层加一个 `x-wonton` 扩展字段描述它在 Wonton 里的身份、
入口、能力和权限。compose 引用的配置文件、图标和设置表单与它放在同一个目录里，合称插件包。
没有 `x-wonton` 的 compose 文件可以由用户手动导入，但必须先补全权限声明。可运行的示例见
[`plugins/examples/`](../plugins/examples/README.md)；默认 Agent 和模型网关的草稿见
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
  id: app.immich.photos
  version: 2.0.0
  kind: app
  title: { zh: 相册, en: Photos }
  description: { zh: 照片备份、人脸与语义搜索, en: Photo backup with face and semantic search }
  icon: icon.png
  category: media
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
| `id` | 是 | 反向域名形式，全局唯一，安装后不可修改。至少两段，每段由小写字母、数字和单个连字符组成，总长不超过 63。 |
| `version` | 是 | 插件包版本（语义化版本），和镜像版本无关。 |
| `kind` | 是 | `app`（普通应用）、`agent`（Agent 运行时）、`channel`（第三方 IM 渠道）、`model`（模型服务）、`system`（系统插件，仅官方，只能由安装器预装）。 |
| `title`、`description` | 是 | 多语言文本，至少提供 `zh` 和 `en`。 |
| `icon` | 否 | 插件包里的文件，或 HTTPS URL。 |
| `category` | 否 | 商店分类。 |
| `webui` | 否 | 作为小程序打开的入口：`service`、`port`、`path`（默认 `/`）、`sso`（`oidc`、`proxy`、`none`，默认 `none`）、`oidc`、`proxy`（见“网页入口与单点登录”）。 |
| `mcp` | 否 | 给 Agent 的工具入口：`service`、`port`、`path`、`transport`（`streamable-http`）。 |
| `bot` | 否 | Bot 的显示名（`display_name`，多语言）和头像（`avatar`，插件包里的文件）。只能和 `matrix.bot` 权限一起声明。 |
| `bots` | 否 | 一个插件多个 Bot：`services` 列出每个 Bot 一份的服务（见“一个插件多个 Bot”）。 |
| `agent` | `kind=agent` 时必填 | `protocol`：`matrix`（Agent 自己登录 Matrix，须声明 `matrix.bot`）或 `acp`（经 ACP 桥接入）。 |
| `provides` | 否 | 向其他插件提供的能力，目前只有 `models`（见“模型网关”）。只有 `kind: system` 的插件可以声明。 |
| `secrets` | 否 | 插件自己的密钥：核心生成后注入，或插件运行中生成、交给核心保管（见“插件密钥”）。 |
| `permissions` | 是 | 安装时由主人确认的权限，见下表。没有权限时写空列表。 |
| `settings` | 否 | 插件包里的 JSON Schema 文件，App 用原生表单渲染。 |
| `requirements` | 否 | `arch`（安装时与宿主架构比对，不符则拒绝）、`memory`、`gpu`（`required`、`optional`）。 |

`x-wonton` 里出现未知字段时安装失败。

## 插件包

插件包是一个目录：`docker-compose.yml`，加上它引用的文件——`icon`、`settings`、`bot.avatar` 和
顶层 `configs` 的 `file`。

- 引用的路径相对 compose 文件所在的目录，不能是绝对路径、不能含 `..`、不能用变量，也不能经过软链接；
  单个文件不超过 1 MiB。引用的文件不存在时安装失败。
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

## 网页入口与单点登录

插件网页的地址是 `<scheme>://<slug>.<基础域名>[:端口]`，`slug` 是把 `id` 里的 `.` 换成 `-`
（`app.immich.photos` → `app-immich-photos`）。所有请求经核心网关，网关先校验主人的会话，未登录的请求
不会到达插件；转发时去掉网关自己的 Cookie 和客户端伪造的 `X-Forwarded-*`，重新设置 `X-Forwarded-For`、
`X-Forwarded-Host`、`X-Forwarded-Proto`，保留原始 `Host`，支持 WebSocket。

`webui.sso` 决定用户怎样登进应用：

- `oidc`：应用支持 OIDC 时首选，由核心的身份提供方登录（下一节）。
- `proxy`：应用不支持 OIDC，但认反向代理传来的用户名或一个固定的密钥时，由网关代为带上
  （见“网关代登录”）。
- `none`：网关只校验会话，应用保留自己的登录，或者本来不需要登录。

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

> 核心认得 `sso: proxy` 和 `webui.proxy`，但还没实现（实现计划第 10 项）：在那之前预览和安装这样的插件都如实
> 拒绝（`unsupported`），不装一个打开网页还要另外登录的插件。magpie 的“模型设置”网页也随第 10 项回来。

网关校验会话后，在转发给插件的请求上加上清单声明的请求头或 Cookie。应用认反向代理传来的用户名时
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
| `webui.proxy.headers` | 请求头名 → 值模板。不能是 `Host`、`Cookie`、`X-Forwarded-*` 和逐跳头部。 |
| `webui.proxy.cookies` | Cookie 名 → 值模板。和浏览器发来的其他 Cookie 合并成一个 `Cookie` 头。 |

两项至少写一项。值模板里可以用 `{user}`（用户名，同 OIDC 的 `preferred_username`）、`{name}`、
`{email}`，以及 `{secret:<名字>}`（`secrets` 里由核心生成的一项），其余文字原样保留，如
`"Bearer {secret:api_token}"`。

- 网关先删掉客户端发来的同名请求头和 Cookie 再加上声明的值，客户端无法伪造或覆盖。
- 这些值不回到浏览器：插件响应里设置同名 Cookie 的 `Set-Cookie` 由网关去掉，密钥只在核心和插件之间。
- 插件子域名彼此同站，一个插件的页面能让浏览器带着会话向另一个插件发请求。所以网关按 Fetch Metadata
  的资源隔离策略，只给这些请求加凭据：`Sec-Fetch-Site` 为 `same-origin` 或 `none`；顶层导航
  （`Sec-Fetch-Mode: navigate`，方法为 `GET` 或 `HEAD`）；没有 `Sec-Fetch-Site` 时，没有 `Origin`
  或 `Origin` 就是插件地址。其余请求照常转发，但不加凭据。
- 插件网络里只有核心和本插件的服务，局域网也不能直接访问容器里没发布的端口（见“网络”），所以应用的
  “信任的代理地址”可以放宽到整个插件网络（如 Navidrome 的 `ND_REVERSEPROXYWHITELIST: 0.0.0.0/0`）。
- 用 `{secret:…}` 的插件分不出是谁在用：每个能打开这个小程序的人都以同一个应用身份登录。

## 权限

| 权限 | 含义 |
| --- | --- |
| `storage.share.<名称>:read` / `:write` | 读或写某个共享文件夹，如 `photos`、`documents`、`music`、`downloads`。 |
| `storage.home:read` / `:write` | 读或写当前用户的私人空间。 |
| `models` | 访问模型网关（magpie），见“模型网关”。 |
| `matrix.bot` | 以 Bot 身份收发消息，见“Matrix Bot”。 |
| `notify` | 给主人发通知。 |
| `network.egress` | 访问外网和局域网，见“网络”。 |
| `network.host` | 使用宿主网络（如 Samba、DLNA）。高权限。 |
| `devices.gpu` | 使用 GPU。高权限。 |
| `devices.<路径>` | 访问宿主设备，如 USB。高权限。 |
| `docker.socket` | 访问 Docker socket。最高权限，默认拒绝，需要单独确认。 |

高权限在安装确认时单独醒目展示。插件不声明的能力一律不给。安装请求必须逐项确认清单里的全部权限，
少一项或多一项都拒绝安装。

核心目前兑现 `storage.share.*`、`network.egress`（见“网络”）、`models`（见“模型网关”）和 `matrix.bot`
（见“Matrix Bot”）。共享文件夹在数据目录的 `shares/<名称>` 下，
只授权读时必须以 `:ro` 挂载，而且只能整个挂载，不能挂其中的子路径（ADR-0011）。其余权限清单里可以声明，
但安装时如实报“尚未支持”。

## 安装时注入的变量

| 变量 | 何时注入 | 说明 |
| --- | --- | --- |
| `WONTON_APP_ID` | 总是 | 插件 `id`。 |
| `WONTON_APP_DATA` | 总是 | 插件私有数据目录（宿主路径）。绑定挂载的子目录由核心逐级预先创建，属主见下文；路径上有软链接或不是目录时拒绝安装（ADR-0011）。 |
| `WONTON_SHARE_<名称>` | 授权了该共享文件夹 | 共享文件夹路径（大写，`-` 换成 `_`，如 `WONTON_SHARE_PHOTOS`）。 |
| `TZ`、`PUID`、`PGID` | 总是 | 时区与运行用户。 |
| `WONTON_APP_URL` | 声明了 `webui` | 插件网页的公开地址，不带末尾 `/`。应用生成回调地址、外部链接时使用。 |
| `WONTON_OIDC_ISSUER`、`WONTON_OIDC_CLIENT_ID`、`WONTON_OIDC_CLIENT_SECRET` | `webui.sso: oidc` | OIDC 客户端配置。 |
| `WONTON_OIDC_AUTH_URL`、`WONTON_OIDC_TOKEN_URL`、`WONTON_OIDC_USERINFO_URL` | `webui.sso: oidc` | 各端点地址，给不支持发现文档的应用（如 Grafana）。 |
| `WONTON_MODEL_BASE_URL` | `models` | 模型网关 OpenAI 兼容接口的地址，以 `/v1` 结尾，如 `http://models.wonton.internal:8080/v1`。 |
| `WONTON_MODEL_ANTHROPIC_BASE_URL`、`WONTON_MODEL_GEMINI_BASE_URL` | `models`，且网关提供该接口 | Anthropic Messages、Gemini 接口的地址，按各自 SDK 的约定不带 `/v1`。 |
| `WONTON_MODEL_API_KEY` | `models` | 只属于本插件（多 Bot 时只属于这个 Bot）的网关凭据。 |
| `WONTON_MODEL_DEFAULT` | `models` | 默认模型名，由网关决定（magpie 是路由组 `group/wonton-default`）。 |
| `WONTON_MATRIX_HOMESERVER` | `matrix.bot` | Matrix 客户端接口的地址：核心在插件网络里的别名，如 `http://matrix.wonton.internal:8080`。 |
| `WONTON_MATRIX_USER_ID`、`WONTON_MATRIX_ACCESS_TOKEN` | `matrix.bot` | Bot 的 Matrix 用户 ID 和访问令牌。 |
| `WONTON_MATRIX_OWNER` | `matrix.bot` | Bot 主人的 Matrix 用户 ID，用作白名单：私人 Bot 只接受主人的指令。 |
| `WONTON_MATRIX_HOME_ROOM` | `matrix.bot` | Bot 的主房间：核心替主人建好的、和 Bot 的加密私聊，定时任务的结果发到这里。 |
| `WONTON_SECRET_<名字>` | `secrets` 里由核心生成的项 | 插件密钥，名字转成大写，如 `WONTON_SECRET_WEB_KEY`。 |
| `WONTON_BOT_NAME`、`WONTON_BOT_DATA` | `bots.services` 里的服务 | Bot 名和这个 Bot 的数据目录（宿主路径）。 |

- 这些变量只用于 compose 插值（`${VAR}`、`${VAR:-默认值}` 等，`$$` 表示字面量 `$`），不会自动进入
  容器环境；需要的变量在对应服务的 `environment` 里显式引用，密钥只给用得到它的容器。
- 引用了没有注入的变量且没有默认值时，安装失败，而不是像 docker compose 那样替换成空串。例如没有声明
  `sso: oidc` 却引用 `WONTON_OIDC_CLIENT_SECRET`，或者模型网关不提供 Anthropic 接口却引用
  `WONTON_MODEL_ANTHROPIC_BASE_URL`。核心不把自己的环境变量传给插件。
- 地址和令牌以注入值为准，插件不要自己拼：主机名、端口和格式都可能变。

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

## 模型网关

模型请求只经模型网关（ADR-0008，默认 magpie）。网关是一个声明了 `provides.models` 的系统插件；核心把
声明了 `models` 的插件接到它上面，给每个插件单独的凭据。

系统插件（`kind: system`）由安装器预装，API 装不了：核心启动后在后台从 `WONTON_SYSTEM_PLUGINS` 目录（每个子目录
一个插件包）装上还没装的系统插件，授予清单声明的全部权限，装核心就是主人对它们的确认；已经装着的照常核对，
不重装也不升级。装不上的如实标为异常、写明原因，不影响核心启动。

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
| `default_model` | 是 | 注入给使用方的 `WONTON_MODEL_DEFAULT`。 |
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
4. 准备 Bot 的主房间：Bot 建一个加密私聊（`trusted_private_chat`、`is_direct`）邀请主人；核心用 JWT 临时登录主人的
   一台设备加入它、写进主人的 `m.direct`，然后退出登录。Bot 的 `m.direct` 也记着它。没有主房间时 Hermes 会在每个
   新会话的第一条回复里插一段设置提示（ADR-0008）。
5. 注入 `WONTON_MATRIX_*` 变量。

- `WONTON_MATRIX_HOMESERVER` 是 `http://matrix.wonton.internal:<网关端口>`，核心在声明了 `matrix.bot` 的插件网络里的
  别名。网关按连接的来源地址认出请求来自哪个插件网络（不算网桥自己的地址，那是宿主那一侧），只转发声明了
  `matrix.bot` 的插件的请求，而且只转发 Matrix 客户端接口（`/_matrix/client/`、`/_matrix/media/`），管理接口和
  联邦接口不转；其他来源得到 403 `M_FORBIDDEN`。插件不和 Tuwunel 共用网络。
- 账号和主房间建好后一直沿用：不删数据卸载后重装、安装失败后重试，都是同一个账号；主房间里 Bot 和主人都还在
  就沿用，否则新建一个。设备每次安装新建，安装失败回滚时吊销。
- 令牌对应一台设备，核心不主动轮换。吊销设备、删掉 Bot 时令牌立即失效（ADR-0009）。核心重启时核对 Bot 的
  设备还在，被吊销了就把插件标为异常，重装（可以不删数据）重新签发；Bot 换了一台设备，要重新建加密身份。
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
- 明确拒绝并给出原因：`ports`、`network_mode`（`none` 以外的取值）、`networks`、`privileged`、
  `devices`、`cap_add`、`security_opt`、`pid`、`ipc`、`container_name`、`build`、`env_file`、`extends`、
  `links`、`external_links`、`volumes_from`，以及顶层的 `networks`、`secrets`。
- `environment` 里只有键没有值的写法（取宿主环境变量）不受支持。

安装时核心按 `depends_on` 顺序启动容器，等每个长期运行的容器健康（有健康检查时）或稳定运行、每个一次性
服务以 0 退出，再确认网页入口能响应，全部通过才算安装完成；任一步失败都按相反顺序回滚已创建的资源，并在
错误里附上容器最近的日志。

## 健康检查

- 长期运行的服务都要有健康检查，而且检查的是真正可用，不只是进程还活着：Hermes 要 Matrix 已经连上
  （用它自己的 `/health/detailed`），magpie 用自带的 `magpie healthcheck`。
- 写在 compose 的 `healthcheck` 里。镜像自带的 `HEALTHCHECK` 也算数，但官方目录的插件要在 compose 里
  显式写出，便于审核，也随插件版本锁定。
- 写上 `start_period` 和 `start_interval`：安装时能尽快判断就绪，启动慢时又不会误判失败。
- 检查只能用镜像里已有的命令，distroless 之类的镜像里没有 `curl`、`wget`。
- 官方目录的插件缺健康检查时不收录（契约测试扫描 `plugins/` 下除 `examples/` 以外的插件）。导入的应用
  没有健康检查时照常安装，状态如实显示为“运行中，健康未知”，不显示“健康”。
- 核心在安装时和每次启动时检查，插件列表里运行中的插件带上最近一次的结果：每个长期运行的服务都有健康检查
  而且都通过了是 `healthy`；有服务没有健康检查，或者核心启动时检查还没通过，是 `unknown`（只知道在运行）。
  检查失败的插件标为异常。两次检查之间核心不盯着插件，状态可能已经变了。

## 网络

- 每个插件有独立的 Docker 网络 `wonton-<slug>`，容器叫 `wonton-<slug>--<服务名>`，服务之间用服务名互访。核心加入所有插件网络并负责
  路由。插件之间、插件和 Tuwunel、插件和模型网关都不共用网络。
- 核心在插件网络里以带点的别名提供下面几个入口，每个别名只出现在需要它的插件网络里。核心按请求从哪个
  插件网络进来判断来源，只转发它有权访问的入口。`.internal` 是 ICANN 保留给私有网络的顶级域，不会和
  公网域名冲突。

  | 别名 | 出现条件 | 转发到 |
  | --- | --- | --- |
  | `auth.<基础域名>` | `webui.sso: oidc` | 身份提供方的公开端点；浏览器和插件看到同一个签发方地址。 |
  | `models.wonton.internal` | `models` | 模型网关声明的服务和端口。 |
  | `matrix.wonton.internal` | `matrix.bot` | Tuwunel 的 Matrix 客户端接口。 |

  不让声明 `models` 的插件直接加入模型网关的网络：那样这些插件彼此也连通了，还能访问网关容器的所有端口。
  经核心转发只多一跳（核心网关的 p50 约 0.27 ms，见 ADR-0006），却守住了插件之间的隔离，核心也能按插件
  限流和审计。Matrix 同理。
- 出网：没有 `network.egress` 的插件网络是 Docker 的内部网络（`internal: true`）：不能访问外网和局域网，
  也解析不了外部域名（Docker Engine 26 起不再为只连内部网络的容器转发外部 DNS 查询，CVE-2024-29018），
  上面的入口照常可用。网桥还设成 `com.docker.network.bridge.gateway_mode_ipv4/ipv6=isolated`（Engine 28 起）：
  网桥在宿主上没有地址，插件连宿主自己监听的服务也访问不到，宿主也不能直接访问插件，核心经它加入的插件网络
  访问插件。声明了 `network.egress` 时插件网络是普通的桥接网络。`network.egress` 不区分外网和局域网，也不按
  服务细分（待定）。
  - 核心直接跑在宿主上（开发时，没设 `WONTON_CORE_CONTAINER`）要经网桥访问插件，所以不设 `isolated`，
    插件还能访问宿主上监听所有地址的服务；插件经自己网络的网关访问核心的身份提供方入口（内部网络没有
    默认路由，`host-gateway` 走不通）。
  - 核心启动核对插件时检查它的网络：和清单、核心的运行方式不符时（比如核心升级前装的插件网络能出网）把
    插件标为异常，提示重装（可以不删数据）。

  > **已定：默认不能出网**（主人 2026-10-28 确认）。要出网的插件声明 `network.egress`，安装时列在权限里：
  > 相册、文档这类插件连不上外网，就带不走数据，符合“住在自己家里”；代价是多一项确认，导入的 CasaOS
  > 应用要自动加上它。没选的方案是默认能出网（Docker、CasaOS、Umbrel 都是这样）：兼容最好，但插件不声明也能把
  > 数据发出去，只能靠审核。
- 核心要求 Docker Engine 28 及以上，并且不开 `allow-direct-routing`：28 起局域网里的其他主机不能再经路由
  直接访问容器里没发布的端口（moby#48724），插件网页才真的只能经核心网关访问；`sso: proxy` 信任网关加的
  请求头，靠的就是这一点。核心启动时检查 Engine 版本，低于 28 时报错退出，不降级运行；
  `allow-direct-routing` 经 Engine API 查不到，由安装器保证。
- 插件之间默认互不可达；需要互通时在清单里声明依赖（待定，M1 前补充）。
- 除 `network.host` 外，插件不向宿主暴露端口；网页一律经核心网关访问。
- 插件的容器、网络和卷都带标签 `wonton.plugin=<id>`，卸载时据此清理；卸载时选择删除数据才会删除
  命名卷、应用数据目录和插件密钥，共享文件夹永不删除。

## 兼容 CasaOS 应用

导入 CasaOS 格式（`x-casaos`）的应用时，核心按以下规则转换，再请主人确认：

- `x-casaos.title`、`icon`、`description` 映射到 `title`、`icon`、`description`。
- `x-casaos.index` 与 `port_map` 映射到 `webui`。
- `/DATA/AppData/$AppID` 下的卷映射到 `WONTON_APP_DATA`。
- 其余宿主路径（如 `/DATA/Media`）映射到共享文件夹，并生成对应的读写权限，必须由主人确认。
- `network_mode: host`、`privileged`、`devices` 生成对应的高权限。
- `ports` 去掉，网页入口改由网关提供。
- 加上 `network.egress`：CasaOS 应用都假定能出网。
- 没有健康检查的服务照常导入，状态显示为“健康未知”。

## 待定问题

- 插件间依赖与通信的声明方式。
- 插件包的签名与官方目录的审核流程。
- `settings.schema.json` 支持的 JSON Schema 子集。
- 插件升级时的数据迁移钩子。
- 网关上 HTTPS 以后，插件网页地址和 OIDC 签发方地址的形式（M0 只有 http）。
- 共享 Bot（M2）的白名单：多个用户，而且会在运行中变化，环境变量表达不了。
- 多 Bot 时的存储授权：共享文件夹是插件级的授权，家人的私人空间（`storage.home`）要按 Bot 的主人分别
  挂载（M2）。
- `network.egress` 区分外网和局域网、按服务细分。
- 插件密钥在 App 里的查看（如应用的管理员密码）与轮换；收存的秘密在恢复数据时怎样交还插件。
- compose 的 `secrets`：以文件形式交给容器的密钥，给遵循 `*_FILE` 约定的镜像用。
- 规范版本号：`x-wonton` 遇到未知字段即失败，带新字段的插件会被旧核心拒绝，需要一个版本字段给出
  清楚的错误。
- 模型网关的首次配置：默认路由组由谁建，主人还没配置 provider 时 Bot 怎样提示。

## 实现计划

v0.1 新增的部分按下面的顺序落地，每项一个 PR，完成后从本节删掉，全部完成后删掉本节。改
`crates/wonton-core/src/plugin/` 的几项要和 #20（挂载约束、容器命名、网关 Cookie）协调先后。

1. **接口：清单类型迁到 `wonton-protocol`**（只改接口，先合并）。
   - `crates/wonton-core/src/plugin/manifest.rs` 的类型和校验移到 `crates/wonton-protocol` 的 `plugin`
     模块；`wonton-core` 加依赖并改为引用它。
   - 新增：`Permission::NetworkEgress`。`secrets`（`generate`、`collect`）、`bots`、`provides.models`
     （`plugin/provides.rs`）、`Sso::Proxy` 和 `webui.proxy`（只解析，见第 10 项）已在核心里，随其他类型一起迁移。
   - 新增的校验：模板只引用 `generate` 的项。只有 `kind: system` 能声明 `provides`、`provides` 引用的服务存在，
     这两条已在核心里。
   - 契约测试：`plugins/` 下所有插件包都能解析，几类违规各被拒绝；App 的安装确认页以后用同一套类型
     （需要 UniFFI 导出的类型按它的约束调整）。
10. **网关代登录**（`gateway/proxy.rs`）：`sso: proxy` 时剥离同名请求头和 Cookie，按 Fetch Metadata 的
    规则加上声明的值，去掉插件响应里同名 Cookie 的 `Set-Cookie`；行为测试覆盖伪造、跨插件请求和不回传。
    和 #20 的 `Set-Cookie` 加固一起改。做完后给 `plugins/magpie` 加回网页入口（“模型设置”），在那之前它不声明
    `webui`，provider 用 magpie 的命令行配置。
12. **多 Bot**（M2，每个 Bot 一组容器）：按 `bots.services` 为每个 Bot 实例化容器、限制 Bot 服务的挂载、
    按 Bot 签发凭据和收存秘密。
