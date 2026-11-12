//! Bot 只读写自己的资料；文件与存文件工具共用 outbox 的打开和成功后删除规则（ADR-0044）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool};
use serde::Deserialize;
use serde_json::{Value, json};
use wonton_protocol::{
    BotActivity, MAX_AVATAR_BYTES, MAX_AVATAR_DURATION_MS, MAX_AVATAR_EDGE, MAX_BOT_NAME_CHARS,
};

use super::audit::{Decision, Record};
use super::core_tools::tool;
use super::{Bus, Caller, naming};
use crate::files::share_fs::saving::Outgoing;
use crate::plugin::bot_profile::{LocalUpdate, MediaChange, Profile, ProfileError, UploadedAvatar};

pub(super) const GET: &str = "get_profile";
pub(super) const SET: &str = "set_profile";

pub(super) fn definitions() -> Vec<Tool> {
    let states: JsonObject = BotActivity::ALL
        .into_iter()
        .map(|state| (state.key().to_owned(), json!({"type": ["string", "null"]})))
        .collect();
    vec![
        tool(
            GET,
            "读取你自己的 Bot 资料：实际名字、有没有静态头像、哪些状态设置了头像、四种状态何时显示，以及名字和媒体限制。不能读取其他 Bot。",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
            true,
        ),
        tool(
            SET,
            "修改你自己的 Bot 名字或头像，不需要主人确认。至少改一项。文件先放进你自己的 outbox，传相对路径，不接受 mxc 地址。\
             静态头像只接受 PNG、JPEG、静态 WebP；状态头像还支持 GIF、动图 WebP 和 H.264 MP4。\
             成功后删除用过且没有再被改写的 outbox 文件，失败时保留。每小时最多成功修改 10 次，超限返回还要等的秒数。",
            json!({"type": "object", "properties": {
                "name": {"type": "string", "description": "新名字，去掉首尾空白后 1 到 32 个字符，不能含控制字符，也不能冒充主人或核心"},
                "avatar": {"type": "string", "description": "静态头像在 outbox 里的相对路径，如 avatar.png"},
                "avatars": {"type": "object", "properties": states, "additionalProperties": false,
                    "description": "只改写到的状态：相对路径设置头像，null 清掉；没写的状态保持不变"},
            }, "additionalProperties": false, "anyOf": [
                {"required": ["name"]}, {"required": ["avatar"]},
                {"required": ["avatars"], "properties": {"avatars": {"minProperties": 1}}},
            ]}),
            false,
        ),
    ]
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    name: Option<String>,
    avatar: Option<String>,
    #[serde(default)]
    avatars: BTreeMap<BotActivity, Option<String>>,
}

fn parse(args: &JsonObject) -> Result<Arguments, String> {
    // serde 的 Option 会接受 null；只有 avatars 里的 null 才表示清掉。
    for key in ["name", "avatar"] {
        if args.get(key).is_some_and(|value| !value.is_string()) {
            return Err(format!("invalid_input: {key} 必须是字符串"));
        }
    }
    let args: Arguments = serde_json::from_value(Value::Object(args.clone()))
        .map_err(|e| format!("invalid_input: {e}"))?;
    if args.name.is_none() && args.avatar.is_none() && args.avatars.is_empty() {
        return Err("invalid_input: 至少修改 name、avatar 或一个状态头像".to_owned());
    }
    Ok(args)
}

pub(super) async fn call(
    bus: &Arc<Bus>,
    caller: &Caller,
    which: &str,
    args: JsonObject,
) -> CallToolResult {
    // 写入一旦开始就要完成审计和收尾，不跟着 MCP 请求断开而取消。
    let (task_bus, task_caller, task_tool) = (bus.clone(), caller.clone(), which.to_owned());
    let task_args = args.clone();
    let task =
        tokio::spawn(async move { run(&task_bus, &task_caller, &task_tool, task_args).await });
    match task.await {
        Ok(result) => result,
        Err(e) => bus.deny(
            caller,
            &naming::join(naming::CORE, which),
            &Value::Object(args),
            &format!("internal: 资料操作意外中止：{e}"),
        ),
    }
}

async fn run(bus: &Bus, caller: &Caller, which: &str, args: JsonObject) -> CallToolResult {
    let name = naming::join(naming::CORE, which);
    let value = Value::Object(args.clone());
    let result = if which == GET {
        if args.is_empty() {
            bus.plugins.bot_profile(&caller.id).await.map_err(problem)
        } else {
            Err("invalid_input: get_profile 不接受参数，只读取你自己的 Bot".to_owned())
        }
    } else {
        set(bus, caller, &args).await
    };
    match result {
        Ok(profile) => {
            bus.audit.record(&Record {
                caller: caller.id.as_str(),
                tool: &name,
                arguments: &value,
                decision: Decision::Allowed,
                basis: None,
                outcome: Some("ok"),
                files: &[],
            });
            CallToolResult::success(vec![ContentBlock::text(view(&profile).to_string())])
        }
        Err(message) => bus.deny(caller, &name, &value, &message),
    }
}

fn problem(error: ProfileError) -> String {
    match error {
        ProfileError::NotFound => "invalid_input: 插件没有安装或没有自己的 Bot".to_owned(),
        ProfileError::RateLimited { retry_after_secs } => {
            format!(
                "rate_limited: retry_after_secs={retry_after_secs}，请再等 {retry_after_secs} 秒"
            )
        }
        error => error.to_string(),
    }
}

async fn set(bus: &Bus, caller: &Caller, args: &JsonObject) -> Result<Profile, String> {
    let args = parse(args)?;
    let mut paths = BTreeMap::new();
    for path in args
        .avatar
        .iter()
        .chain(args.avatars.values().filter_map(Option::as_ref))
    {
        *paths.entry(path.clone()).or_insert(0) += 1;
    }
    let (outgoing, mut files) = read(bus.plugins.app_data_dir(&caller.id), paths).await?;
    let mut take = |path: String| -> Result<MediaChange, String> {
        // 重复路径只读一次；上传接口拥有字节，所以最后一次使用才移动，其余使用复制。
        let (filename, data, uses) = files
            .get_mut(&path)
            .ok_or_else(|| "internal: 没有读到头像文件".to_owned())?;
        *uses -= 1;
        let data = if *uses == 0 {
            std::mem::take(data)
        } else {
            data.clone()
        };
        Ok(MediaChange::Set(UploadedAvatar {
            filename: filename.clone(),
            data,
            poster: None,
        }))
    };
    let avatar = args.avatar.map(&mut take).transpose()?;
    let states = args
        .avatars
        .into_iter()
        .map(|(state, path)| {
            Ok((
                state,
                path.map(&mut take)
                    .transpose()?
                    .unwrap_or(MediaChange::Clear),
            ))
        })
        .collect::<Result<_, String>>()?;
    let profile = bus
        .plugins
        .set_bot_profile(
            &caller.id,
            LocalUpdate {
                name: args.name,
                avatar,
                states,
            },
        )
        .await
        .map_err(problem)?;
    if let Some(outgoing) = outgoing
        && let Err(error) = tokio::task::spawn_blocking(move || outgoing.remove_originals()).await
    {
        tracing::warn!(%error, "资料已更新，清理 outbox 意外中止");
    }
    Ok(profile)
}

type Files = BTreeMap<String, (String, Vec<u8>, usize)>;

async fn read(
    app_data: PathBuf,
    paths: BTreeMap<String, usize>,
) -> Result<(Option<Outgoing>, Files), String> {
    if paths.is_empty() {
        return Ok((None, BTreeMap::new()));
    }
    tokio::task::spawn_blocking(move || {
        let unique: Vec<_> = paths.keys().cloned().collect();
        let outgoing = Outgoing::open(&app_data, &unique).map_err(|e| e.to_string())?;
        let bytes = outgoing
            .read_bounded(MAX_AVATAR_BYTES + 1)
            .map_err(|e| e.to_string())?;
        let files = paths
            .into_iter()
            .zip(bytes)
            .map(|((path, uses), (name, data))| (path, (name, data, uses)))
            .collect();
        Ok((Some(outgoing), files))
    })
    .await
    .map_err(|e| format!("internal: 读取 outbox 意外中止：{e}"))?
}

fn view(profile: &Profile) -> Value {
    let avatars: Vec<_> = BotActivity::ALL
        .into_iter()
        .filter(|state| profile.avatars.states.contains_key(state.key()))
        .map(BotActivity::key)
        .collect();
    json!({
        "name": profile.name,
        "avatar": profile.avatar.is_some(),
        "avatars": avatars,
        "when": {
            "offline": "插件没在运行：在安装、升级、启动，停了或者出错（最高优先级）",
            "waiting": "插件在运行，有这个 Bot 发起的确认卡片在等主人",
            "working": "插件在运行、没有待确认卡片，有模型请求或工具调用在进行，或 Bot 正在输入",
            "idle": "以上条件都不成立",
        },
        "limits": {
            "max_name_chars": MAX_BOT_NAME_CHARS,
            "max_edge": MAX_AVATAR_EDGE,
            "max_bytes": MAX_AVATAR_BYTES,
            "max_duration_ms": MAX_AVATAR_DURATION_MS,
            "square": true,
            "static_formats": ["image/png", "image/jpeg", "image/webp"],
            "animated_formats": ["image/gif", "image/webp"],
            "video_formats": ["video/mp4 (H.264)"],
            "successful_changes_per_hour": 10,
        },
    })
}
