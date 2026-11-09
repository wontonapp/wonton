//! Bot 只读写自己的资料；文件与存文件工具共用 outbox 的打开和成功后删除规则（ADR-0044）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool};
use serde::Deserialize;
use serde_json::{Value, json};
use wonton_protocol::{
    BotActivity, FileFailure, MAX_AVATAR_BYTES, MAX_AVATAR_DURATION_MS, MAX_AVATAR_EDGE,
    MAX_BOT_NAME_CHARS,
};

use super::audit::{Decision, Record};
use super::core_tools::tool;
use super::{Bus, Caller, ToolFailure, naming};
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
            "Read your own bot profile: actual name, whether a static avatar is set, which states have avatars and \
             when each state applies, plus the name and media limits. Can't read other bots.",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
            true,
        ),
        tool(
            SET,
            "Change your own bot's name or avatar; no owner confirmation needed. Change at least one thing. Put files \
             in your own outbox first and pass relative paths; mxc URIs are not accepted. \
             Static avatars accept PNG, JPEG, and static WebP; state avatars also accept GIF, animated WebP, and \
             H.264 MP4. On success, used outbox files that are not rewritten again are deleted; on failure they are \
             kept. At most 10 successful changes per hour; over the limit the error returns how many seconds to wait.",
            json!({"type": "object", "properties": {
                "name": {"type": "string", "description": "new name, 1 to 32 characters after trimming, no control characters, and it must not impersonate the owner or the home server"},
                "avatar": {"type": "string", "description": "relative path in your outbox of the static avatar, e.g. avatar.png"},
                "avatars": {"type": "object", "properties": states, "additionalProperties": false,
                    "description": "only the states being changed: a relative path sets the avatar, null clears it; states not mentioned stay unchanged"},
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

fn parse(args: &JsonObject) -> Result<Arguments, ToolFailure> {
    // serde 的 Option 会接受 null；只有 avatars 里的 null 才表示清掉。
    for key in ["name", "avatar"] {
        if args.get(key).is_some_and(|value| !value.is_string()) {
            return Err(ToolFailure::new(
                "invalid_input",
                format!("invalid_input: {key} must be a string"),
            ));
        }
    }
    let args: Arguments = serde_json::from_value(Value::Object(args.clone()))
        .map_err(|e| ToolFailure::new("invalid_input", format!("invalid_input: {e}")))?;
    if args.name.is_none() && args.avatar.is_none() && args.avatars.is_empty() {
        return Err(ToolFailure::new(
            "invalid_input",
            "invalid_input: change at least name, avatar, or one state avatar",
        ));
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
            &ToolFailure::new(
                "internal_error",
                format!("internal: profile operation aborted unexpectedly: {e}"),
            ),
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
            Err(ToolFailure::new(
                "invalid_input",
                "invalid_input: get_profile takes no arguments; it only reads your own bot",
            ))
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
        Err(failure) => bus.deny(caller, &name, &value, &failure),
    }
}

fn problem(error: ProfileError) -> ToolFailure {
    // 这两个的文字按 Bot 面向的英文写，限流的还带机器可读的秒数；别的用错误自己的文字（开头就是类别）。
    let message = match &error {
        ProfileError::NotFound => {
            "not_found: the app is not installed or has no bot of its own".to_owned()
        }
        ProfileError::RateLimited { retry_after_secs } => {
            format!(
                "rate_limited: retry_after_secs={retry_after_secs}; wait {retry_after_secs} seconds and try again"
            )
        }
        error => error.to_string(),
    };
    ToolFailure::new(error.kind(), message)
}

async fn set(bus: &Bus, caller: &Caller, args: &JsonObject) -> Result<Profile, ToolFailure> {
    let args = parse(args)?;
    let mut paths = BTreeMap::new();
    for path in args
        .avatar
        .iter()
        .chain(args.avatars.values().filter_map(Option::as_ref))
    {
        *paths.entry(path.clone()).or_insert(0) += 1;
    }
    let (outgoing, mut files) = read(bus.plugins.app_data_dir(&caller.id), paths)
        .await
        .map_err(|e| ToolFailure::new(e.kind.as_str(), e.to_string()))?;
    let mut take = |path: String| -> Result<MediaChange, ToolFailure> {
        // 重复路径只读一次；上传接口拥有字节，所以最后一次使用才移动，其余使用复制。
        let (filename, data, uses) = files.get_mut(&path).ok_or_else(|| {
            ToolFailure::new("internal_error", "internal: avatar file was not read")
        })?;
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
        .collect::<Result<_, ToolFailure>>()?;
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
        tracing::warn!(%error, "profile updated; cleaning up the outbox aborted unexpectedly");
    }
    Ok(profile)
}

type Files = BTreeMap<String, (String, Vec<u8>, usize)>;

async fn read(
    app_data: PathBuf,
    paths: BTreeMap<String, usize>,
) -> Result<(Option<Outgoing>, Files), crate::files::Failure> {
    if paths.is_empty() {
        return Ok((None, BTreeMap::new()));
    }
    tokio::task::spawn_blocking(move || {
        let unique: Vec<_> = paths.keys().cloned().collect();
        let outgoing = Outgoing::open(&app_data, &unique)?;
        let bytes = outgoing.read_bounded(MAX_AVATAR_BYTES + 1)?;
        let files = paths
            .into_iter()
            .zip(bytes)
            .map(|((path, uses), (name, data))| (path, (name, data, uses)))
            .collect();
        Ok((Some(outgoing), files))
    })
    .await
    .map_err(|e| {
        crate::files::Failure::new(
            FileFailure::Internal,
            format!("internal: reading the outbox aborted unexpectedly: {e}"),
        )
    })?
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
            "offline": "the app is not running: installing, upgrading, starting, stopped, or in error (highest priority)",
            "waiting": "the app is running and a confirmation card from this bot is waiting for the owner",
            "working": "the app is running with no pending confirmation card; a model request or tool call is in progress, or the bot is typing",
            "idle": "none of the above",
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
