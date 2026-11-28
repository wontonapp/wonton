use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::task::{DeviceId, Person, TaskState};
use super::view::StopReason;
use super::{DeviceProfile, MediaItemRef, Timestamp, known_or_other};

/// 播放会话号，核心发。
pub type PlaybackSessionId = Uuid;
/// 队列版本，所有改动带读到的版本。
pub type Revision = u64;

/// 一个人的播放队列，自带版本；所有改动带读到的 `revision`，不等就 409。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Queue {
    pub revision: Revision,
    pub entries: Vec<MediaItemRef>,
    pub cursor: Option<usize>,
    pub mode: QueueMode,
    pub sleep_at_ms: Option<u64>,
}

impl Queue {
    /// 队列改了就加一版，让并发的旧版本落空。
    pub fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

/// 队列的播放顺序。新版本新加的模式落在 `Other`。
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum QueueMode {
    Sequential,
    Shuffle,
    RepeatAll,
    RepeatOne,
    /// 新版本新加的模式，原值在里。
    Other(String),
}

known_or_other!(QueueMode {
    Sequential => "sequential",
    Shuffle => "shuffle",
    RepeatAll => "repeat_all",
    RepeatOne => "repeat_one",
});

/// 起播入参；切轨用同一 `item`、当前 `position_ms` 和新轨号重取。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PlaybackRequest {
    /// 从上次位置起播；0 表示从头
    pub position_ms: u64,
    pub audio: Option<u16>,
    pub subtitle: Option<u16>,
    pub item: MediaItemRef,
    pub device: DeviceId,
    pub person: Person,
    pub profile: DeviceProfile,
}

/// 提供方给的核心可播计划。`session` 是上游会话号，和核心的 `PlaybackSessionId` 是两套，映射只在核心。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PlaybackPlan {
    pub session: String,
    /// 核心填好，客户端只读这个
    pub stream: ProviderStream,
    /// 提供方原址，只给 cast-url 用
    pub origin_stream: Option<ProviderStream>,
    pub segments: Vec<Segment>,
    pub audio: Vec<MediaTrack>,
    pub subtitles: Vec<MediaTrack>,
    pub seekable: bool,
    pub duration_ms: u64,
    pub expires_at: Timestamp,
}

/// 可播的流：直连原片或转码流。新版本新加的流型落在 `Other`，里面是原地址。
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ProviderStream {
    Direct(String),
    Transcoded(String),
    /// 新版本新加的流型；载荷是字符串时存原地址，否则只留标签名。
    Other(String),
}

impl ProviderStream {
    fn url(&self) -> &str {
        match self {
            Self::Direct(url) | Self::Transcoded(url) | Self::Other(url) => url,
        }
    }
}

impl Serialize for ProviderStream {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let index = match self {
            Self::Direct(_) => 0,
            Self::Transcoded(_) => 1,
            Self::Other(_) => 2,
        };
        let name = match self {
            Self::Direct(_) => "direct",
            Self::Transcoded(_) => "transcoded",
            Self::Other(_) => "other",
        };
        serializer.serialize_newtype_variant("ProviderStream", index, name, self.url())
    }
}

impl<'de> Deserialize<'de> for ProviderStream {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Object(fields) if fields.len() == 1 => {
                let (key, payload) = fields.into_iter().next().expect("刚判过只有一个键");
                match (key.as_str(), payload) {
                    ("direct", serde_json::Value::String(url)) => Ok(Self::Direct(url)),
                    ("transcoded", serde_json::Value::String(url)) => Ok(Self::Transcoded(url)),
                    (_, serde_json::Value::String(url)) => Ok(Self::Other(url)),
                    _ => Ok(Self::Other(key)),
                }
            }
            other => Err(serde::de::Error::custom(format!(
                "可播的流应该是单键对象，拿到 {other}"
            ))),
        }
    }
}

/// 一条音轨或字幕轨。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MediaTrack {
    pub index: u16,
    pub language: String,
    pub label: Option<String>,
    pub kind: TrackKind,
    pub selected: bool,
}

/// 轨道类型。新版本新加的类型落在 `Other`。
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum TrackKind {
    Audio,
    Subtitle,
    Other(String),
}

known_or_other!(TrackKind {
    Audio => "audio",
    Subtitle => "subtitle",
});

/// 章节、片头片尾。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Segment {
    pub kind: SegmentKind,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// 片段类型。
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SegmentKind {
    Chapter,
    Intro,
    Outro,
    Other(String),
}

known_or_other!(SegmentKind {
    Chapter => "chapter",
    Intro => "intro",
    Outro => "outro",
});

/// 一条播放会话。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PlaybackSession {
    pub id: PlaybackSessionId,
    pub person: Person,
    pub device: DeviceId,
    pub item: MediaItemRef,
    pub state: TaskState,
    pub position_ms: u64,
    /// 起播时带回的 plan；过期由核心重取
    pub plan: Option<PlaybackPlan>,
    /// 停的原因，停了才有且不再变；活动会话为空
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
}

#[cfg(test)]
mod tests {
    use super::super::MediaKind;
    use super::super::tests::item;
    use super::super::view::QueueOp;
    use super::*;

    fn roundtrip<T>(value: &T) -> T
    where
        T: Serialize + serde::de::DeserializeOwned,
    {
        let json = serde_json::to_value(value).unwrap();
        serde_json::from_value(json).unwrap()
    }

    fn queue() -> Queue {
        Queue {
            revision: 1,
            entries: vec![item()],
            cursor: Some(0),
            mode: QueueMode::Sequential,
            sleep_at_ms: None,
        }
    }

    #[test]
    fn bumping_the_queue_increases_the_revision() {
        let mut queue = queue();
        assert_eq!(queue.revision, 1);
        queue.bump();
        assert_eq!(queue.revision, 2);
        queue.bump();
        assert_eq!(queue.revision, 3);
    }

    #[test]
    fn queue_roundtrips() {
        let queue = Queue {
            mode: QueueMode::Shuffle,
            sleep_at_ms: Some(120_000),
            ..queue()
        };
        assert_eq!(roundtrip(&queue), queue);
    }

    #[test]
    fn unknown_queue_mode_falls_to_other_and_keeps_reading() {
        let mut json = serde_json::to_value(queue()).unwrap();
        json["mode"] = serde_json::json!("random");
        let back: Queue = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(back.mode, QueueMode::Other("random".into()));
        assert_eq!(serde_json::to_value(&back).unwrap(), json);
    }

    #[test]
    fn every_queue_op_roundtrips() {
        let ops = [
            QueueOp::Read,
            QueueOp::Enqueue {
                at: Some(1),
                item: item(),
            },
            QueueOp::Enqueue {
                at: None,
                item: item(),
            },
            QueueOp::Remove { index: 2 },
            QueueOp::Jump { index: 3 },
            QueueOp::Clear,
            QueueOp::SetMode(QueueMode::RepeatOne),
            QueueOp::Replace {
                entries: vec![item(), item()],
                cursor: Some(1),
            },
            QueueOp::Replace {
                entries: Vec::new(),
                cursor: None,
            },
        ];
        for op in ops {
            let json = serde_json::to_value(&op).unwrap();
            let back: QueueOp = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(serde_json::to_value(&back).unwrap(), json);
        }
        let json = serde_json::to_value(QueueOp::SetMode(QueueMode::Shuffle)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"op": "set_mode", "value": "shuffle"})
        );
        let json = serde_json::to_value(QueueOp::Replace {
            entries: Vec::new(),
            cursor: Some(0),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"op": "replace", "value": {"entries": [], "cursor": 0}})
        );
    }

    /// 队列改动是要执行的：不认识的不能当没事跳过（旧核心收到新 App 的 `replace` 得拒，不能回成功）。
    #[test]
    fn a_queue_op_this_build_does_not_know_is_refused() {
        let unknown = serde_json::json!({"op": "shuffle_from_here", "value": {"index": 2}});
        assert!(serde_json::from_value::<QueueOp>(unknown).is_err());
    }

    fn plan() -> PlaybackPlan {
        PlaybackPlan {
            session: "upstream-7".into(),
            stream: ProviderStream::Direct("https://example.test/s.m3u8".into()),
            origin_stream: Some(ProviderStream::Transcoded(
                "https://upstream.test/t.m3u8".into(),
            )),
            segments: vec![Segment {
                kind: SegmentKind::Intro,
                start_ms: 0,
                end_ms: 30_000,
            }],
            audio: vec![MediaTrack {
                index: 1,
                language: "zh".into(),
                label: Some("中文".into()),
                kind: TrackKind::Audio,
                selected: true,
            }],
            subtitles: vec![],
            seekable: true,
            duration_ms: 3_600_000,
            expires_at: 1_700_000_600_000,
        }
    }

    #[test]
    fn unknown_stream_types_keep_the_url() {
        let stream: ProviderStream =
            serde_json::from_str("{\"dash\": \"https://example.test/m.mpd\"}").unwrap();
        assert_eq!(
            stream,
            ProviderStream::Other("https://example.test/m.mpd".into())
        );
        assert_eq!(
            serde_json::to_value(&stream).unwrap(),
            serde_json::json!({"other": "https://example.test/m.mpd"})
        );

        let stream: ProviderStream = serde_json::from_str("{\"opaque\": {\"a\": 1}}").unwrap();
        assert_eq!(stream, ProviderStream::Other("opaque".into()));
        assert_eq!(
            serde_json::to_value(&stream).unwrap(),
            serde_json::json!({"other": "opaque"})
        );
    }

    #[test]
    fn playback_plan_roundtrips() {
        let plan = plan();
        assert_eq!(roundtrip(&plan), plan);
        assert_eq!(
            serde_json::to_value(ProviderStream::Direct("x".into())).unwrap(),
            serde_json::json!({"direct": "x"})
        );
    }

    fn session() -> PlaybackSession {
        PlaybackSession {
            id: Uuid::nil(),
            person: "Alice".into(),
            device: "phone".into(),
            item: item(),
            state: TaskState::Running,
            position_ms: 12_500,
            plan: Some(plan()),
            stop_reason: None,
        }
    }

    #[test]
    fn playback_session_roundtrips() {
        let session = session();
        assert_eq!(roundtrip(&session), session);
    }

    #[test]
    fn a_stopped_session_carries_its_reason() {
        let session = session();
        assert!(
            serde_json::to_value(&session)
                .unwrap()
                .get("stop_reason")
                .is_none()
        );
        let mut stopped = session;
        stopped.stop_reason = Some(StopReason::TakenOver);
        assert_eq!(roundtrip(&stopped), stopped);
    }

    #[test]
    fn playback_request_roundtrips_without_optional_tracks() {
        let request = PlaybackRequest {
            position_ms: 0,
            audio: None,
            subtitle: None,
            item: item(),
            device: "tv".into(),
            person: "Alice".into(),
            profile: DeviceProfile {
                direct_play: vec!["video/mp4".into()],
                max_bitrate: 8_000_000,
                hdr: false,
                container: vec!["mp4".into()],
            },
        };
        assert_eq!(roundtrip(&request), request);
    }

    #[test]
    fn track_and_segment_kinds_are_snake_case() {
        assert_eq!(
            serde_json::to_value(TrackKind::Subtitle).unwrap(),
            serde_json::json!("subtitle")
        );
        assert_eq!(
            serde_json::to_value(SegmentKind::Outro).unwrap(),
            serde_json::json!("outro")
        );
        assert!(serde_json::from_str::<MediaKind>("\"video\"").is_ok());
    }

    #[test]
    fn unknown_track_and_segment_kinds_fall_to_other() {
        let track: TrackKind = serde_json::from_str("\"lyrics\"").unwrap();
        assert_eq!(track, TrackKind::Other("lyrics".into()));
        assert_eq!(
            serde_json::to_value(&track).unwrap(),
            serde_json::json!("lyrics")
        );
        let segment: SegmentKind = serde_json::from_str("\"recap\"").unwrap();
        assert_eq!(segment, SegmentKind::Other("recap".into()));
        assert_eq!(
            serde_json::to_value(&segment).unwrap(),
            serde_json::json!("recap")
        );
    }
}
