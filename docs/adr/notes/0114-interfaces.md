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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
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

校验：`service` 在 `services` 里；`add`/`remove`/`credential` 必须以 `/` 开头，`status` 若有同样；`webui.service` 不必相同。
`status` 的输出是一条 JSON：`{"users": ["<用户名>", …]}`，含已停用的账号、不含插件自己的管理员（只给对账用）：

```rust
pub struct MemberStatus { pub users: Vec<String> }
```
另有 `status: Option<String>`（可选，一条 JSON 列出已建账号）。对账只在安装完成、容器重建后跑一次、以及每天一次；补建前先看这人还在不在成员里，
  刚被移除的不会被补回来；补建失败进重试。

运行约束：每条命令 30 秒超时，stdout 4 KiB、stderr 8 KiB 截断；退出码 0 成功、2 输入非法、3 只对 `credential` 有意义（凭据失效，重放 `add`），
其它算失败并进待重试。

## 2. 清单：`inbox`（只加）

```rust
#[serde(default)]
pub inbox: Option<InboxSpec>,

pub struct InboxSpec {
    pub service: String,
    /// 容器里的绝对路径，挂在插件数据卷上。
    pub path: String,
    /// 放完字节要调的命令（可选）；不写等于插件自己定期扫
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan: Option<String>,
}
```

校验：`/` 开头的绝对路径，不含 `..`，不在 `/proc`、`/sys`、`/dev` 下；必须同时有 `files` 权限。

## 3. 事件（协议只加；清单用了在旧核心装不上，预期）

- 新文件 `crates/wonton-protocol/src/event.rs`。
- 在 `manifest/permission.rs` 的 `Permission` 加 `Events`（`events`）与 `Files`（`files`）；`Files` 是 `inbox` 的前置。
- `spec_floor.rs` 按清单里实际用到的字段与权限逐条登记（`Added::Permission`、`Added::Field`）：`member_accounts`（含 `status`）、`inbox`、
  `inbox.scan`、`events`、`files`；`tools/list` 与结果里的 `_meta` 不受 `spec_floor` 管，旧核心忽略即回退 `keeper`（旧核心 `deny_unknown_fields` 会拒装，靠版本号提示升级）。
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
    pub id: String, pub plugin_id: Option<String>, pub kind: String, pub person: Option<String>,
    pub title: String, pub body: Option<String>, pub link: Option<String>, pub path: Option<String>,
    /// 原文事件数据；不进聊天消息正文，只在 `wonton__events` 的工具结果里回
    pub data: Option<serde_json::Value>,
    /// 原文被截断过
    pub truncated: bool,
    pub level: NoticeLevel, pub count: u32, pub created_at: i64,
    /// 取走时加的 60 秒租约；同一个人有两个 Bot 时防重
    pub lease_until: Option<i64>,
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
pub const TOOL_ROOM_RESULTS_META: &str = "app.wonton/room_results";  // tools/list，bool，缺省 false
pub const TOOL_RESULT_SCOPE_META: &str = "app.wonton/result_scope"; // 结果：room | keeper，缺省 keeper
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

破坏点：`Display`/`FromStr` 的穷尽匹配（serde 走 `FromStr`）。旧记录能读；**带 `room:` 的记录旧核心读不了**。
核对、缓存、降级脚本、加 `Room` 时核谁、读的时候核谁，全见 ADR-0114 第 7 节与卡 11，这里只留类型与序列化。

## 7. `wonton-docs`（公开规范，同批）

- “权限”表加 `events` 一行；注入变量表加 `WONTON_EVENTS_URL`；网络别名表加 `events.wonton.internal`。
- 新节“家人账号”写 `member_accounts`、四条命令、JSON、退出码、`status` 输出形状、`lan.client` 按人、Jellyfin 用 Quick Connect。
- 权限表加 `files`，和 `events` 一起进安装预览、商店元数据与四种语言。
- 新节“收件箱”写 `inbox`、`scan`、`app.wonton/files`、`inbox:` 前缀、24 小时清理、`source_unavailable`。
- `tools/list` 一节写 `card`、`files`、`room_results`；`tools/call` 一节写 `speakers`、`room_people`；结果一节写 `result_scope`、`card_result`、`deep_link`。
  `room_results` 坏值只在 `plugin check` 与 `tools/list` 校验时记警告，运行期按 false。
- “通知”一节加 `to`、`link`，把“家人不收”改成按 `to`。
- `plugin check --official` 的清单校验跟上第 1、2 节。
- 规范版本按现有规则升一小版，写明需要的最低核心版本。

## 8. 不动的接口

- `notify` 的 `path`、返回码、级别含义不动。
- `_meta` 的 `origin`、`failure`、`confirmation`、`capabilities` 不动。
- `wonton://tab/plugin-path/...` 的解析器不改，在 `link.rs` 只加前缀校验函数。
- 文件编号格式、上传、注册、过期、`BotRef` 不动。

