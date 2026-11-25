use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::MediaItemRef;
use super::session::{
    PlaybackPlan, PlaybackSession, PlaybackSessionId, Queue, QueueMode, Revision,
};
use super::task::{ItemState, MediaFailure, Person, Progress, TaskId, TaskKind, TaskState};

/// 播放控制动作。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
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
}

/// 停播的原因。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    User,
    Agent,
    TakenOver,
    Finished,
    Error,
}

/// 队列改动。内部标记放不下序列和模式，载荷另起 `value` 键。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
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

/// 壳→核的动作与核→设备的命令是同一个类型。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case", tag = "kind")]
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
}

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
    pub quota: QuotaView,
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
    pub queue: Queue,
    pub plan: Option<PlaybackPlan>,
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
        ];
        for action in actions {
            let json = serde_json::to_value(&action).unwrap();
            let back: MediaAction = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(serde_json::to_value(&back).unwrap(), json);
        }
    }

    #[test]
    fn unknown_media_action_variant_is_an_error() {
        let json = serde_json::json!({"kind": "beam_me_up", "session": Uuid::nil()});
        assert!(serde_json::from_value::<MediaAction>(json).is_err());
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
            quota: QuotaView {
                provider_upload: QuotaEntry {
                    used: 1_000,
                    limit: Some(10_000),
                },
                device_offline: QuotaEntry {
                    used: 0,
                    limit: None,
                },
            },
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
            queue: Queue {
                revision: 0,
                entries: vec![item()],
                cursor: None,
                mode: QueueMode::Sequential,
                sleep_at_ms: None,
            },
            plan: None,
        };
        assert_eq!(roundtrip(&view), view);
    }
}
