# 插件规范 `x-wonton`（草案 v0）

> 草案，M1 前定稿。字段可能调整，调整时同步更新本文和清单类型。M0 期间类型在
> `crates/wonton-core/src/plugin/manifest.rs`，定稿后迁到 `crates/wonton-protocol`。

一个插件就是一个 `docker-compose.yml`，在顶层加一个 `x-wonton` 扩展字段描述它在 Wonton 里的身份、
入口、能力和权限。没有 `x-wonton` 的 compose 文件可以由用户手动导入，但必须先补全权限声明。
可运行的示例见 [`plugins/examples/`](../plugins/examples/README.md)。

## 示例

```yaml
name: immich
services:
  server:
    image: ghcr.io/immich-app/immich-server:v2
    volumes:
      - ${WONTON_APP_DATA}/upload:/usr/src/app/upload
      - ${WONTON_SHARE_PHOTOS}:/photos:ro
  mcp:
    image: ghcr.io/example/immich-mcp:1

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
  bot:
    display_name: { zh: 相册助手, en: Photos }
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
| `icon` | 否 | 相对插件包的路径，或 HTTPS URL。 |
| `category` | 否 | 商店分类。 |
| `webui` | 否 | 作为小程序打开的入口：`service`、`port`、`path`（默认 `/`）、`sso`（`oidc`、`none`，默认 `none`）、`oidc`（见下节）。 |
| `mcp` | 否 | 给 Agent 的工具入口：`service`、`port`、`path`、`transport`（`streamable-http`）。 |
| `bot` | 否 | 插件以 bot 身份发消息时的显示名、头像。 |
| `agent` | `kind=agent` 时必填 | `protocol`：`matrix`（Agent 自己登录 Matrix）或 `acp`（经 ACP 桥接入）。 |
| `permissions` | 是 | 安装时由主人确认的权限，见下表。没有权限时写空列表。 |
| `settings` | 否 | JSON Schema 文件，App 用原生表单渲染。 |
| `requirements` | 否 | `arch`（安装时与宿主架构比对，不符则拒绝）、`memory`、`gpu`（`required`、`optional`）。 |

`x-wonton` 里出现未知字段时安装失败。

## 网页入口与单点登录

插件网页的地址是 `<scheme>://<slug>.<基础域名>[:端口]`，`slug` 是把 `id` 里的 `.` 换成 `-`
（`app.immich.photos` → `app-immich-photos`）。所有请求经核心网关，网关先校验主人的会话，未登录的请求
不会到达插件；转发时去掉网关自己的 Cookie 和客户端伪造的 `X-Forwarded-*`，重新设置 `X-Forwarded-For`、
`X-Forwarded-Host`、`X-Forwarded-Proto`，保留原始 `Host`，支持 WebSocket。

`sso: oidc` 的插件由核心的身份提供方（Ory Hydra，见 ADR-0009 验证结果）登录，用户不会看到第二个
登录页。清单要写明回调路径：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `webui.oidc.redirect_paths` | 是 | 应用登录回调的路径（以 `/` 开头，相对插件网页地址），核心据此登记客户端的回调地址。 |
| `webui.oidc.token_endpoint_auth_method` | 否 | 应用在令牌端点出示密钥的方式：`client_secret_basic`（默认）或 `client_secret_post`。 |

客户端可申请的 scope：`openid`、`profile`（`name`、`preferred_username`）、`email`（`email`、
`email_verified`）、`groups`（主人为 `["owner"]`，可用来映射应用内角色）、`offline_access`。
客户端 ID 就是插件 `id`，密钥在安装时生成，卸载时删除客户端。

## 权限

| 权限 | 含义 |
| --- | --- |
| `storage.share.<名称>:read` / `:write` | 读或写某个共享文件夹，如 `photos`、`documents`、`music`、`downloads`。 |
| `storage.home:read` / `:write` | 读或写当前用户的私人空间。 |
| `models` | 访问模型网关（magpie）。 |
| `matrix.bot` | 以 bot 身份收发消息。 |
| `notify` | 给主人发通知。 |
| `network.host` | 使用宿主网络（如 Samba、DLNA）。高权限。 |
| `devices.gpu` | 使用 GPU。高权限。 |
| `devices.<路径>` | 访问宿主设备，如 USB。高权限。 |
| `docker.socket` | 访问 Docker socket。最高权限，默认拒绝，需要单独确认。 |

高权限在安装确认时单独醒目展示。插件不声明的能力一律不给。安装请求必须逐项确认清单里的全部权限，
少一项或多一项都拒绝安装。

M0 的核心只兑现 `storage.share.*`：共享文件夹在数据目录的 `shares/<名称>` 下，只授权读时必须以 `:ro`
挂载。其余权限清单里可以声明，但安装时如实报“尚未支持”。

## 安装时注入的变量

| 变量 | 说明 |
| --- | --- |
| `WONTON_APP_ID` | 插件 `id`。 |
| `WONTON_APP_DATA` | 插件私有数据目录（宿主路径）。绑定挂载的子目录由核心预先创建并交给 `PUID:PGID`。 |
| `WONTON_SHARE_<名称>` | 已授权的共享文件夹路径（大写，`-` 换成 `_`，如 `WONTON_SHARE_PHOTOS`）。 |
| `TZ`、`PUID`、`PGID` | 时区与运行用户。 |
| `WONTON_APP_URL` | 声明了 `webui` 时注入：插件网页的公开地址，不带末尾 `/`。应用生成回调地址、外部链接时使用。 |
| `WONTON_MODEL_BASE_URL`、`WONTON_MODEL_API_KEY` | 声明了 `models` 时注入，指向 magpie，令牌只属于该插件。 |
| `WONTON_OIDC_ISSUER`、`WONTON_OIDC_CLIENT_ID`、`WONTON_OIDC_CLIENT_SECRET` | `webui.sso: oidc` 时注入。 |
| `WONTON_OIDC_AUTH_URL`、`WONTON_OIDC_TOKEN_URL`、`WONTON_OIDC_USERINFO_URL` | `webui.sso: oidc` 时注入，给不支持发现文档的应用（如 Grafana）。 |

- 这些变量只用于 compose 插值（`${VAR}`、`${VAR:-默认值}` 等，`$$` 表示字面量 `$`），不会自动进入
  容器环境；需要的变量在对应服务的 `environment` 里显式引用，密钥只给用得到它的容器。
- 引用了没有注入的变量（例如没有声明 `sso: oidc` 却引用 `WONTON_OIDC_CLIENT_SECRET`）且没有默认值时，
  安装失败，而不是像 docker compose 那样替换成空串。核心不把自己的环境变量传给插件。

## Compose 子集

核心经 Docker API 渲染插件，只接受下面的字段，其余字段一律报错：

- 顶层：`name`、`services`、`volumes`（只支持不带参数的本地卷）、`x-wonton`，以及任意 `x-` 扩展字段
  （如 `x-casaos`）；`version` 会被忽略。
- 服务：`image`、`command`、`entrypoint`、`environment`、`volumes`、`restart`（未写时默认
  `unless-stopped`）、`healthcheck`、`depends_on`（含 `service_started`、`service_healthy`、
  `service_completed_successfully`）、`user`、`working_dir`、`labels`（`wonton.` 前缀保留给核心）、
  `hostname`、`expose`、`init`、`read_only`、`shm_size`、`tmpfs`、`stop_signal`、`stop_grace_period`、
  `pull_policy`（默认 `missing`），以及 `x-` 扩展字段。`image` 既没写标签也没写摘要时按 `latest` 拉取，
  和 compose 一致。
- 卷：短语法和长语法（`bind`、`volume`、`tmpfs`）。宿主路径只能在 `${WONTON_APP_DATA}` 或已授权的
  共享文件夹里，不能含 `..`，不支持相对路径；命名卷必须在顶层 `volumes` 声明。
- 命名卷在 Docker 里叫 `wonton-<slug>-<卷名>`。同名卷已存在时：属于本插件（上次卸载时保留的数据）就
  直接复用；属于其他插件或不是核心创建的，拒绝安装。安装失败回滚时只删除本次新建的卷，保留的卷不删。
- 明确拒绝并给出原因：`ports`、`network_mode`、`networks`、`privileged`、`devices`、`cap_add`、
  `security_opt`、`pid`、`ipc`、`container_name`、`build`、`env_file`、`extends`、`links`、
  `external_links`、`volumes_from`，以及顶层的 `networks`、`configs`、`secrets`。
- `environment` 里只有键没有值的写法（取宿主环境变量）不受支持。

安装时核心按 `depends_on` 顺序启动容器，等每个容器健康（有健康检查时）或稳定运行，再确认网页入口能
响应，全部通过才算安装完成；任一步失败都按相反顺序回滚已创建的资源，并在错误里附上容器最近的日志。

## 网络

- 每个插件有独立的 Docker 网络 `wonton-<slug>`，服务之间用服务名互访。核心加入所有插件网络并负责
  路由；`sso: oidc` 的插件在自己的网络里经别名 `auth.<基础域名>` 访问核心上的身份提供方入口。
- 声明 `models` 的插件额外加入模型网关网络。
- 插件之间默认互不可达；需要互通时在清单里声明依赖（待定，M1 前补充）。
- 除 `network.host` 外，插件不向宿主暴露端口；网页一律经核心网关访问。
- 插件的容器、网络和卷都带标签 `wonton.plugin=<id>`，卸载时据此清理；卸载时选择删除数据才会删除
  命名卷和应用数据目录，共享文件夹永不删除。

## 兼容 CasaOS 应用

导入 CasaOS 格式（`x-casaos`）的应用时，核心按以下规则转换，再请主人确认：

- `x-casaos.title`、`icon`、`description` 映射到 `title`、`icon`、`description`。
- `x-casaos.index` 与 `port_map` 映射到 `webui`。
- `/DATA/AppData/$AppID` 下的卷映射到 `WONTON_APP_DATA`。
- 其余宿主路径（如 `/DATA/Media`）映射到共享文件夹，并生成对应的读写权限，必须由主人确认。
- `network_mode: host`、`privileged`、`devices` 生成对应的高权限。
- `ports` 去掉，网页入口改由网关提供。

## 待定问题

- 插件间依赖与通信的声明方式。
- 插件包的签名与官方目录的审核流程。
- `settings.schema.json` 支持的 JSON Schema 子集。
- 插件升级时的数据迁移钩子。
- 网关上 HTTPS 以后，插件网页地址和 OIDC 签发方地址的形式（M0 只有 http）。
