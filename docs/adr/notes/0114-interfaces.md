# ADR-0114 接口清单（只改接口的 PR）

依据 `docs/adr/0114-plugin-agent-platform.md`。路径相对主仓库根。

标记两种：「只加」= 旧核心忽略、旧插件不写照跑；「破坏性」= 同一 PR 里要改完所有调用方。

## 1. 清单：`member_accounts`（只加）

新文件 `crates/wonton-protocol/src/manifest/member_accounts.rs`。

`Manifest` (`crates/wonton-protocol/src/manifest/mod.rs:108`) 加：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub member_accounts: Option<MemberAccounts>,

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberAccounts {
    /// 命令在哪一个服务里跑，必须在本插件 `services` 里。
    pub service: String,
    pub commands: MemberCommands,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberCommands {
    pub add: String,
    pub remove: String,
    pub credential: String,
}
```

命令的标准输入与凭据输出（同文件）。退出码 `0` 成功、`2` 输入非法、`3` 只给 `credential` 表示凭据失效：

```rust
pub struct MemberInput { pub user: String, #[serde(default)] pub admin: bool }
pub struct MemberTarget { pub user: String }

#[serde(untagged)]
pub enum MemberCredential {
    Password { username: String, password: String },
    Token { username: String, token: String },
}
```

校验：`service` 在 `services` 里；三条命令以 `/` 开头；申请时必须同时有 `files` 权限；`webui.service` 不必相同。

## 2. 清单：`inbox`（只加）

```rust
#[serde(default)]
pub inbox: Option<InboxSpec>,

pub struct InboxSpec {
    pub service: String,
    /// 容器里的绝对路径，挂在插件数据卷上。
    pub path: String,
}
```

校验：`/` 开头的绝对路径，不含 `..`，不在 `/proc`、`/sys`、`/dev` 下；声明了也要 `files` 权限。

## 3. 事件（协议只加；清单用了在旧核心装不上，预期）

- 新文件 `crates/wonton-protocol/src/event.rs`。
- 在 `manifest/permission.rs` 的 `Permission` 加 `Events`（序列化成 `events`）。
- 插件网络别名：`events.wonton.internal`；注入变量：`WONTON_EVENTS_URL`。

`POST` 的请求体和 202 回体（新类型在 `event.rs`）：

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventReport {
    pub kind: EventKind,                 // 小写 ASCII、`_`、`.`，1..=64
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<String>,
    pub title: String,                   // 最多 120 字符
    pub body: Option<String>,            // 最多 2000
    pub path: Option<String>,
    pub link: Option<String>,
    pub data: Option<serde_json::Value>, // 最多 4 KiB
    pub dedupe_key: Option<String>,
    pub level: Option<NoticeLevel>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventAccepted { pub event_id: String }
```

核心工具的参数与返回：

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventsQuery { pub since: Option<String>, pub wait_seconds: Option<u32> }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventsPage { pub events: Vec<DeliveredEvent> }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AckQuery { pub ids: Vec<String> }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AckResult { pub acked: usize }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeliveredEvent {
    pub id: String, pub plugin_id: String, pub kind: String, pub person: Option<String>,
    pub title: String, pub body: Option<String>, pub link: Option<String>, pub path: Option<String>,
    pub level: NoticeLevel, pub count: u32, pub created_at: i64,
}
```

## 4. 通知体（只加；`to` 有人不可达时 400 是新行为）

`crates/wonton-protocol/src/plugin_notice.rs:17` 的 `PluginNotice` 加：

```rust
/// 收件人用户名；空 = 主人。去重，最多 16 个。
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub to: Vec<String>,
/// 深链，优先于 `path`。
#[serde(default, skip_serializing_if = "Option::is_none")]
pub link: Option<String>,
```

`path`、其余字段不动。

## 5. 工具和结果的 `_meta` 键（只加；`user` 的语义是破坏性变化）

新常量（`crates/wonton-protocol/src/family.rs`，和现有 `TOOL_CALL_USER_META` 同处）：

```rust
pub const TOOL_CALL_SPEAKERS_META: &str = "app.wonton/speakers";     // [String]，1..=8
pub const TOOL_CALL_ROOM_PEOPLE_META: &str = "app.wonton/room_people";
pub const TOOL_CARD_META: &str = "app.wonton/card";                 // tools/list
pub const TOOL_FILES_META: &str = "app.wonton/files";               // [String]，1..=4
pub const TOOL_CARD_RESULT_META: &str = "app.wonton/card_result";   // 结果
pub const TOOL_DEEP_LINK_META: &str = "app.wonton/deep_link";       // 结果
pub const TOOL_RESULT_SCOPE_META: &str = "app.wonton/result_scope"; // "room" | "keeper"
```

类型：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResultScope { Room, Keeper }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardTemplate {

    pub title: String,                  // 模板，渲染后最多 80 字符
    #[serde(default)]
    pub lines: Vec<CardLine>,           // 最多 12 行
    pub result: Option<CardResult>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardLine { pub text: String, pub arg: Option<String>, pub emphasis: Option<CardEmphasis> }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardResult { pub summary: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardEmphasis { Normal, Warning, Danger }
```

`app.wonton/user` 的取值改成：私人 Bot = 归属人；共享 Bot 且 `speakers` 正好一人 = 那人；其余不带。键名不变，读法不变（可选），
只是共享 Bot 单人时会从“不带”变成“带”，所以标破坏性。

事件里 `result_scope` 只在工具结果上生效，确认卡不看它。

## 6. 文件访问者 `Accessor::Room`（破坏性）

`crates/wonton-core/src/files/record.rs:29` 的枚举加一个分支：

```rust
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Accessor {
    Owner,
    User(String),
    Bot(String),
    Room(String),   // 本地房间 ID，序列化成 `room:<id>`
}
```

破坏点：`Display`/`TryFrom<String>` 的穷尽匹配；磁盘上已有的记录本身能读（旧分支不变），但**用新核心写过的记录旧核心读不了**
（`TryFrom` 报未知前缀）。降级要连同记录一起回滚，写进升级说明。

`Gateway` 的鉴权要新加一条：`Accessor::Room(id)` 用请求方的 Matrix 身份实时查 `joined_members`。房间 ID 只存本地 ID。

## 7. `wonton-docs`（公开规范，同批）

- “权限”表加 `events` 一行；注入变量表加 `WONTON_EVENTS_URL`；网络别名表加 `events.wonton.internal`。
- 新节“家人账号”写 `member_accounts`、三条命令、JSON、退出码、`lan.client` 按人、Jellyfin 用 Quick Connect。
- 新节“收件箱”写 `inbox`、`app.wonton/files`、`inbox:` 前缀、24 小时清理、`source_unavailable`。
- “通知”一节加 `to`、`link`，把“家人不收”改成按 `to`。
- `tools/list` 一节加 `card`、`files` 两个 `_meta`；`tools/call` 一节加 `speakers`、`room_people`、`result_scope`；
  结果一节加 `card_result`、`deep_link`。
- `plugin check --official` 的清单校验跟上第 1、2 节。
- 规范版本按现有规则升一小版，写明需要的最低核心版本。

## 8. 不动的接口

- `notify` 的 `path`、返回码、级别含义不动。
- `_meta` 的 `origin`、`failure`、`confirmation`、`capabilities` 不动。
- `wonton://tab/plugin-path/...` 的解析器不改，在 `link.rs` 只加前缀校验函数。
- 文件编号格式、上传、注册、过期、`BotRef` 不动。

