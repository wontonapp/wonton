use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::known_or_other;
use super::session::{
    PlaybackPlan, PlaybackSession, PlaybackSessionId, Queue, QueueMode, Revision,
};
use super::task::{ItemState, MediaFailure, Person, Progress, TaskId, TaskKind, TaskState};
use super::{MediaItemRef, MediaKind};

/// 播放控制动作。新版本新加的动作落在 `Other`，只留原值，载荷丢弃。
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ControlOp {
    Play,
    Pause,
    Resume,
    SeekMs(u64),
    Next,
    Prev,
    Volume(u8),
    SetAudio(u16),
    SetSubtitle(Option<u16>),
    /// 新版本新加的动作，原值在里。
    Other(String),
}

/// 停播的原因。新版本新加的原因落在 `Other`。
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum StopReason {
    User,
    Agent,
    TakenOver,
    Finished,
    Error,
    /// 这个人退出了家庭，核心替他停的。
    Removed,
    /// 新版本新加的原因，原值在里。
    Other(String),
}

known_or_other!(StopReason {
    User => "user",
    Agent => "agent",
    TakenOver => "taken_over",
    Finished => "finished",
    Error => "error",
    Removed => "removed",
});

impl Serialize for ControlOp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Play => serializer.serialize_str("play"),
            Self::Pause => serializer.serialize_str("pause"),
            Self::Resume => serializer.serialize_str("resume"),
            Self::SeekMs(ms) => serializer.serialize_newtype_variant("ControlOp", 3, "seek_ms", ms),
            Self::Next => serializer.serialize_str("next"),
            Self::Prev => serializer.serialize_str("prev"),
            Self::Volume(volume) => {
                serializer.serialize_newtype_variant("ControlOp", 6, "volume", volume)
            }
            Self::SetAudio(track) => {
                serializer.serialize_newtype_variant("ControlOp", 7, "set_audio", track)
            }
            Self::SetSubtitle(track) => {
                serializer.serialize_newtype_variant("ControlOp", 8, "set_subtitle", track)
            }
            Self::Other(raw) => serializer.serialize_str(raw),
        }
    }
}

impl<'de> Deserialize<'de> for ControlOp {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(match value {
            serde_json::Value::String(raw) => match raw.as_str() {
                "play" => Self::Play,
                "pause" => Self::Pause,
                "resume" => Self::Resume,
                "next" => Self::Next,
                "prev" => Self::Prev,
                _ => Self::Other(raw),
            },
            serde_json::Value::Object(fields) if fields.len() == 1 => {
                let (key, raw) = fields.into_iter().next().expect("刚判过只有一个键");
                match key.as_str() {
                    "seek_ms" => Self::SeekMs(lift(raw)?),
                    "volume" => Self::Volume(lift(raw)?),
                    "set_audio" => Self::SetAudio(lift(raw)?),
                    "set_subtitle" => Self::SetSubtitle(lift(raw)?),
                    _ => Self::Other(key),
                }
            }
            other => {
                return Err(serde::de::Error::custom(format!(
                    "播放控制应该是字符串或单键对象，拿到 {other}"
                )));
            }
        })
    }
}

fn lift<T: serde::de::DeserializeOwned, E: serde::de::Error>(
    value: serde_json::Value,
) -> Result<T, E> {
    serde_json::from_value(value).map_err(E::custom)
}

/// 队列改动。内部标记放不下序列和模式，载荷另起 `value` 键。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case", tag = "op", content = "value")]
// 形状照接口清单；装箱会改公开类型和绑定，不值当。
#[allow(clippy::large_enum_variant)]
pub enum QueueOp {
    Read,
    Enqueue {
        at: Option<usize>,
        item: MediaItemRef,
    },
    Remove {
        index: usize,
    },
    Jump {
        index: usize,
    },
    Clear,
    SetMode(QueueMode),
}

/// 下载完的一项，带本机路径。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DoneItem {
    pub item: MediaItemRef,
    pub local_path: Option<String>,
}

/// 壳→核的动作与核→设备的命令是同一个类型。新版本新加的动作落在 `Other`，旧壳跳过那一条、同批其他
/// 命令照常执行。
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[allow(clippy::large_enum_variant)]
pub enum MediaAction {
    StartUpload {
        task: TaskId,
    },
    StartDownload {
        task: TaskId,
    },
    Playback {
        session: PlaybackSessionId,
    },
    Control {
        session: PlaybackSessionId,
        ctl: ControlOp,
    },
    Stop {
        session: PlaybackSessionId,
        reason: StopReason,
    },
    Queue {
        person: Person,
        revision: Revision,
        op: QueueOp,
    },
    /// 立即跑一次备份（`wonton__backup_now`）：设备照自己的备份设置挑出还没传的，建一个上传任务，回执的
    /// `task` 带它。`kinds` 空着是这台设备上开了备份的全部种类。
    Backup {
        kinds: Vec<MediaKind>,
    },
    /// 删掉这台设备上这几项的离线副本（`wonton__media_delete`）。删完设备照常 `PUT /offline` 报一次占用。
    DeleteOffline {
        items: Vec<MediaItemRef>,
    },
    /// 新版本新加的动作，原值在里，载荷丢弃。
    Other(String),
}

/// 认得的动作；未知 `kind` 落到 [`MediaAction::Other`]。
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[allow(clippy::large_enum_variant)]
enum KnownMediaAction {
    StartUpload {
        task: TaskId,
    },
    StartDownload {
        task: TaskId,
    },
    Playback {
        session: PlaybackSessionId,
    },
    Control {
        session: PlaybackSessionId,
        ctl: ControlOp,
    },
    Stop {
        session: PlaybackSessionId,
        reason: StopReason,
    },
    Queue {
        person: Person,
        revision: Revision,
        op: QueueOp,
    },
    Backup {
        kinds: Vec<MediaKind>,
    },
    DeleteOffline {
        items: Vec<MediaItemRef>,
    },
}

impl From<KnownMediaAction> for MediaAction {
    fn from(action: KnownMediaAction) -> Self {
        match action {
            KnownMediaAction::StartUpload { task } => Self::StartUpload { task },
            KnownMediaAction::StartDownload { task } => Self::StartDownload { task },
            KnownMediaAction::Playback { session } => Self::Playback { session },
            KnownMediaAction::Control { session, ctl } => Self::Control { session, ctl },
            KnownMediaAction::Stop { session, reason } => Self::Stop { session, reason },
            KnownMediaAction::Queue {
                person,
                revision,
                op,
            } => Self::Queue {
                person,
                revision,
                op,
            },
            KnownMediaAction::Backup { kinds } => Self::Backup { kinds },
            KnownMediaAction::DeleteOffline { items } => Self::DeleteOffline { items },
        }
    }
}

impl Serialize for MediaAction {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self {
            Self::StartUpload { task } => {
                map.serialize_entry("kind", "start_upload")?;
                map.serialize_entry("task", task)?;
            }
            Self::StartDownload { task } => {
                map.serialize_entry("kind", "start_download")?;
                map.serialize_entry("task", task)?;
            }
            Self::Playback { session } => {
                map.serialize_entry("kind", "playback")?;
                map.serialize_entry("session", session)?;
            }
            Self::Control { session, ctl } => {
                map.serialize_entry("kind", "control")?;
                map.serialize_entry("session", session)?;
                map.serialize_entry("ctl", ctl)?;
            }
            Self::Stop { session, reason } => {
                map.serialize_entry("kind", "stop")?;
                map.serialize_entry("session", session)?;
                map.serialize_entry("reason", reason)?;
            }
            Self::Queue {
                person,
                revision,
                op,
            } => {
                map.serialize_entry("kind", "queue")?;
                map.serialize_entry("person", person)?;
                map.serialize_entry("revision", revision)?;
                map.serialize_entry("op", op)?;
            }
            Self::Backup { kinds } => {
                map.serialize_entry("kind", "backup")?;
                map.serialize_entry("kinds", kinds)?;
            }
            Self::DeleteOffline { items } => {
                map.serialize_entry("kind", "delete_offline")?;
                map.serialize_entry("items", items)?;
            }
            Self::Other(raw) => map.serialize_entry("kind", raw)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for MediaAction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let kind = value
            .get("kind")
            .and_then(|kind| kind.as_str())
            .map(str::to_owned)
            .ok_or_else(|| serde::de::Error::missing_field("kind"))?;
        let known = matches!(
            kind.as_str(),
            "start_upload"
                | "start_download"
                | "playback"
                | "control"
                | "stop"
                | "queue"
                | "backup"
                | "delete_offline"
        );
        if known {
            KnownMediaAction::deserialize(value)
                .map(Self::from)
                .map_err(serde::de::Error::custom)
        } else {
            Ok(Self::Other(kind))
        }
    }
}

/// 叫醒睡着的设备去取命令的推送（ADR-0113 第 5 节）。明文是 `{"server": …, "media_command": {"seq": N}}`，
/// 只带 `seq`、不带命令内容：设备醒来照常 `GET /commands` 拉，命令只从那里来。
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MediaCommandPush {
    pub seq: u64,
}

/// 推送明文里 [`MediaCommandPush`] 那一段的字段名。
pub const MEDIA_COMMAND_PUSH_FIELD: &str = "media_command";

/// 核发给设备的命令，`command_id` 幂等，`seq` 单调。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DeviceCommand {
    pub command_id: Uuid,
    pub seq: u64,
    pub body: MediaAction,
}

/// 壳渲染任务用；文案只放类别不给句子（壳按类别出字）。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MediaViewModel {
    pub tasks: Vec<TaskView>,
    /// 还没从服务器取到时为空，不拿零冒充。
    pub quota: Option<QuotaView>,
    /// 最近一次和服务器对齐没成的原因；成了就清掉。
    pub failure: Option<MediaFailure>,
}

/// 任务列表的一项。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TaskView {
    pub id: TaskId,
    pub kind: TaskKind,
    pub title: String,
    pub state: TaskState,
    pub progress: Progress,
    pub failure: Option<MediaFailure>,
    /// 逐项条目（一次最多 100 条，多的用 cursor 翻）
    pub items: Vec<ItemView>,
    pub items_cursor: Option<String>,
}

/// 任务里的一个上传项。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ItemView {
    pub item_id: String,
    pub name: String,
    pub state: ItemState,
    pub upload_offset: u64,
}

/// 两本账各一项，壳才知道去清哪边
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct QuotaView {
    pub provider_upload: QuotaEntry,
    pub device_offline: QuotaEntry,
}

/// 一本账：用了多少、上限多少。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct QuotaEntry {
    pub used: u64,
    pub limit: Option<u64>,
}

/// 播放页的视图：会话、队列和当前计划。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PlaybackViewModel {
    pub session: PlaybackSession,
    /// 还没从服务器取到时为空。
    pub queue: Option<Queue>,
    pub plan: Option<PlaybackPlan>,
    /// 这台设备最近一次播放动作没成的原因；换会话或下一次成了就清掉。
    pub failure: Option<MediaFailure>,
}

/// 导出的媒体操作没做成：原因的类别照核心给的，壳按类别出字。
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error), uniffi::export(Display))]
pub enum MediaError {
    #[error("{}", .failure.message)]
    Failed { failure: MediaFailure },
}

impl From<MediaFailure> for MediaError {
    fn from(failure: MediaFailure) -> Self {
        Self::Failed { failure }
    }
}

#[cfg(test)]
mod tests {
    use super::super::task::{Progress, ProgressUnit, TaskState};
    use super::super::tests::item;
    use super::*;

    fn roundtrip<T>(value: &T) -> T
    where
        T: Serialize + serde::de::DeserializeOwned,
    {
        let json = serde_json::to_value(value).unwrap();
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn every_media_action_roundtrips() {
        let actions = [
            MediaAction::StartUpload { task: Uuid::nil() },
            MediaAction::StartDownload { task: Uuid::nil() },
            MediaAction::Playback {
                session: Uuid::nil(),
            },
            MediaAction::Control {
                session: Uuid::nil(),
                ctl: ControlOp::SeekMs(42_000),
            },
            MediaAction::Control {
                session: Uuid::nil(),
                ctl: ControlOp::SetSubtitle(None),
            },
            MediaAction::Stop {
                session: Uuid::nil(),
                reason: StopReason::Finished,
            },
            MediaAction::Queue {
                person: "Alice".into(),
                revision: 4,
                op: QueueOp::Clear,
            },
            MediaAction::Backup {
                kinds: vec![MediaKind::Photo],
            },
            MediaAction::Backup { kinds: Vec::new() },
            MediaAction::DeleteOffline {
                items: vec![crate::media::MediaItemRef {
                    id: "v1".into(),
                    kind: MediaKind::Video,
                    title: "第一集".into(),
                    artist: None,
                    album: None,
                    artwork_url: None,
                    duration_ms: None,
                    series: None,
                    index: None,
                    played: false,
                    starred: false,
                    source: None,
                }],
            },
        ];
        for action in actions {
            let json = serde_json::to_value(&action).unwrap();
            let back: MediaAction = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(serde_json::to_value(&back).unwrap(), json);
        }
    }

    #[test]
    fn unknown_media_action_variant_is_other_and_skippable() {
        let json = serde_json::json!({"kind": "beam_me_up", "session": Uuid::nil()});
        let action: MediaAction = serde_json::from_value(json).unwrap();
        assert_eq!(action, MediaAction::Other("beam_me_up".into()));
        assert_eq!(
            serde_json::to_value(&action).unwrap(),
            serde_json::json!({"kind": "beam_me_up"})
        );
    }

    #[test]
    fn a_batch_of_commands_survives_an_unknown_one() {
        let batch = serde_json::json!([
            {
                "command_id": Uuid::nil(),
                "seq": 1,
                "body": {"kind": "playback", "session": Uuid::nil()},
            },
            {
                "command_id": Uuid::nil(),
                "seq": 2,
                "body": {"kind": "beam_me_up", "session": Uuid::nil()},
            },
        ]);
        let commands: Vec<DeviceCommand> = serde_json::from_value(batch).unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[1].body, MediaAction::Other("beam_me_up".into()));
    }

    #[test]
    fn unknown_control_ops_and_stop_reasons_fall_to_other() {
        let ctl: ControlOp = serde_json::from_value(serde_json::json!({"speed": 2})).unwrap();
        assert_eq!(ctl, ControlOp::Other("speed".into()));
        assert_eq!(
            serde_json::to_value(&ctl).unwrap(),
            serde_json::json!("speed")
        );

        let ctl: ControlOp = serde_json::from_value(serde_json::json!("rewind")).unwrap();
        assert_eq!(ctl, ControlOp::Other("rewind".into()));

        let reason: StopReason = serde_json::from_value(serde_json::json!("sleep")).unwrap();
        assert_eq!(reason, StopReason::Other("sleep".into()));
    }

    #[test]
    fn control_ops_use_their_shapes() {
        let json = serde_json::to_value(ControlOp::Volume(80)).unwrap();
        assert_eq!(json, serde_json::json!({"volume": 80}));
        let json = serde_json::to_value(ControlOp::SetAudio(2)).unwrap();
        assert_eq!(json, serde_json::json!({"set_audio": 2}));
        let json = serde_json::to_value(ControlOp::SetSubtitle(Some(3))).unwrap();
        assert_eq!(json, serde_json::json!({"set_subtitle": 3}));
        assert_eq!(roundtrip(&ControlOp::Play), ControlOp::Play);
    }

    #[test]
    fn stop_reasons_are_snake_case() {
        assert_eq!(
            serde_json::to_value(StopReason::TakenOver).unwrap(),
            serde_json::json!("taken_over")
        );
    }

    #[test]
    fn device_command_roundtrips() {
        let command = DeviceCommand {
            command_id: Uuid::nil(),
            seq: 7,
            body: MediaAction::Playback {
                session: Uuid::nil(),
            },
        };
        assert_eq!(roundtrip(&command), command);
    }

    #[test]
    fn media_view_model_roundtrips() {
        let view = MediaViewModel {
            tasks: vec![TaskView {
                id: Uuid::nil(),
                kind: TaskKind::Download,
                title: "海边".into(),
                state: TaskState::Running,
                progress: Progress {
                    done: 10,
                    total: Some(100),
                    unit: ProgressUnit::Bytes,
                },
                failure: None,
                items: vec![
                    ItemView {
                        item_id: "i1".into(),
                        name: "a.jpg".into(),
                        state: ItemState::Running,
                        upload_offset: 10,
                    },
                    ItemView {
                        item_id: "i2".into(),
                        name: "b.jpg".into(),
                        state: ItemState::Queued,
                        upload_offset: 0,
                    },
                ],
                items_cursor: Some("next".into()),
            }],
            quota: Some(QuotaView {
                provider_upload: QuotaEntry {
                    used: 1_000,
                    limit: Some(10_000),
                },
                device_offline: QuotaEntry {
                    used: 0,
                    limit: None,
                },
            }),
            failure: None,
        };
        assert_eq!(roundtrip(&view), view);
        assert_eq!(
            serde_json::to_value(DoneItem {
                item: item(),
                local_path: Some("/data/x".into()),
            })
            .unwrap()["local_path"],
            "/data/x"
        );
    }

    #[test]
    fn playback_view_model_roundtrips_without_a_plan() {
        let view = PlaybackViewModel {
            session: PlaybackSession {
                id: Uuid::nil(),
                person: "Alice".into(),
                device: "phone".into(),
                item: item(),
                state: TaskState::Preparing,
                position_ms: 0,
                plan: None,
                stop_reason: None,
            },
            queue: Some(Queue {
                revision: 0,
                entries: vec![item()],
                cursor: None,
                mode: QueueMode::Sequential,
                sleep_at_ms: None,
            }),
            plan: None,
            failure: None,
        };
        assert_eq!(roundtrip(&view), view);
    }
}
