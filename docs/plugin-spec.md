# 插件规范 `x-wonton`（草案 v0）

> 草案，M1 前定稿。字段可能调整，调整时同步更新本文和 `crates/wonton-protocol` 中的类型。

一个插件就是一个 `docker-compose.yml`，在顶层加一个 `x-wonton` 扩展字段描述它在 Wonton 里的身份、
入口、能力和权限。没有 `x-wonton` 的 compose 文件可以由用户手动导入，但必须先补全权限声明。

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
| `id` | 是 | 反向域名形式，全局唯一，安装后不可修改。 |
| `version` | 是 | 插件包版本（语义化版本），和镜像版本无关。 |
| `kind` | 是 | `app`（普通应用）、`agent`（Agent 运行时）、`channel`（第三方 IM 渠道）、`model`（模型服务）、`system`（系统插件，仅官方）。 |
| `title`、`description` | 是 | 多语言文本，至少提供 `zh` 和 `en`。 |
| `icon` | 否 | 相对插件包的路径，或 HTTPS URL。 |
| `category` | 否 | 商店分类。 |
| `webui` | 否 | 作为小程序打开的入口：`service`、`port`、`path`（默认 `/`）、`sso`（`oidc`、`none`）。 |
| `mcp` | 否 | 给 Agent 的工具入口：`service`、`port`、`path`、`transport`（`streamable-http`）。 |
| `bot` | 否 | 插件以 bot 身份发消息时的显示名、头像。 |
| `agent` | `kind=agent` 时必填 | `protocol`：`matrix`（Agent 自己登录 Matrix）或 `acp`（经 ACP 桥接入）。 |
| `permissions` | 是 | 安装时由主人确认的权限，见下表。没有权限时写空列表。 |
| `settings` | 否 | JSON Schema 文件，App 用原生表单渲染。 |
| `requirements` | 否 | `arch`、`memory`、`gpu`（`required`、`optional`）。 |

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

高权限在安装确认时单独醒目展示。插件不声明的能力一律不给。

## 安装时注入的变量

| 变量 | 说明 |
| --- | --- |
| `WONTON_APP_ID` | 插件 `id`。 |
| `WONTON_APP_DATA` | 插件私有数据目录。 |
| `WONTON_SHARE_<名称>` | 已授权的共享文件夹路径（大写，如 `WONTON_SHARE_PHOTOS`）。 |
| `TZ`、`PUID`、`PGID` | 时区与运行用户。 |
| `WONTON_MODEL_BASE_URL`、`WONTON_MODEL_API_KEY` | 声明了 `models` 时注入，指向 magpie，令牌只属于该插件。 |
| `WONTON_OIDC_ISSUER`、`WONTON_OIDC_CLIENT_ID`、`WONTON_OIDC_CLIENT_SECRET` | `webui.sso: oidc` 时注入。 |

## 网络

- 每个插件有独立的 Docker 网络，核心网关加入所有插件网络并负责路由。
- 声明 `models` 的插件额外加入模型网关网络。
- 插件之间默认互不可达；需要互通时在清单里声明依赖（待定，M1 前补充）。
- 除 `network.host` 外，插件不向宿主暴露端口；网页一律经核心网关访问。

## 兼容 CasaOS 应用

导入 CasaOS 格式（`x-casaos`）的应用时，核心按以下规则转换，再请主人确认：

- `x-casaos.title`、`icon`、`description` 映射到 `title`、`icon`、`description`。
- `x-casaos.index` 与 `port_map` 映射到 `webui`。
- `/DATA/AppData/$AppID` 下的卷映射到 `WONTON_APP_DATA`。
- 其余宿主路径（如 `/DATA/Media`）映射到共享文件夹，并生成对应的读写权限，必须由主人确认。
- `network_mode: host`、`privileged`、`devices` 生成对应的高权限。

## 待定问题

- 插件间依赖与通信的声明方式。
- 插件包的签名与官方目录的审核流程。
- `settings.schema.json` 支持的 JSON Schema 子集。
- 插件升级时的数据迁移钩子。
