//! 中继表设置页（ADR-0048）：核心有一张从配置来的中继表，主人在 App 里看当前经哪台中继、加自己的中继、调档位。
//!
//! 核心的管理接口 `/_wonton/api/relays` 收发这里的类型，客户端核心原样交给壳。表里每一项的
//! [`RelaySettingEntry::state`] 由核心读 iroh 的家中继状态填：只有当前家中继有确切状态，其余是
//! [`RelayState::Unknown`]。当前这条连接走的中继在 [`HomeViewModel::current_relay`](crate::HomeViewModel::current_relay)，
//! 由 App 按自己的路径填，设置页把它标出来。
//!
//! 核心可能比 App 新：核心多给的字段被忽略；枚举遇到不认识的取值时落到 [`RelaySource::Other`] 或
//! [`RelayState::Other`]，不让整份设置解析失败。

use serde::{Deserialize, Serialize};

use crate::SecretString;

/// 这一项从哪来。决定它能被谁改。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum RelaySource {
    /// 编译进核心的默认中继（`WONTON_BUILTIN_RELAYS`）。能删、能挪，删了记进配置文件。
    #[default]
    Builtin,
    /// 签名的中继列表（`WONTON_RELAY_LIST_URL`）。能删、能挪，删了记进配置文件。
    Signed,
    /// `WONTON_RELAYS` 环境变量。设了环境变量时整张表只读。
    Env,
    /// 主人自己在 App 里配的，存在核心的数据目录里。
    Settings,
    /// 比这个 App 新的核心给的来源。
    #[serde(other)]
    Other,
}

/// 这台中继现在的连接状态，核心读 iroh 的家中继状态给。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RelayState {
    /// 连上了。
    Connected,
    /// 中继拒绝了令牌：换一个令牌，或者找中继管理员要准入。`reason` 是中继自己给的原话，可能是空的。
    Refused {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// 连不上：地址不对、端口不通或者中继没在运行。
    Unreachable {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// 还没试过，或者不是当前家中继、核心看不到它的状态。
    #[default]
    Unknown,
    /// 比这个 App 新的核心给的状态。
    #[serde(other)]
    Other,
}

/// 设置页里的一行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RelaySettingEntry {
    /// 中继地址，规范成不带令牌、不带 FQDN 尾点的一种写法。改地址等于删一条再加一条。
    pub url: String,
    /// 准入令牌。`Debug` 打码；设置页上按打码显示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<SecretString>,
    /// QUIC 地址发现的 UDP 端口。空着用 iroh 的默认端口（7842）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quic_port: Option<u16>,
    /// 档位：数字小的先用，同一档里由 iroh 按延迟自己挑。
    pub tier: u8,
    pub source: RelaySource,
    /// 核心看到的这台中继的状态。
    pub state: RelayState,
    /// 这一项能不能改。环境变量固定时为假。
    pub editable: bool,
}

/// 核心现在用的一整张中继表，以及它给的提示。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RelaySettingsSnapshot {
    /// 按档位排好的表。
    pub entries: Vec<RelaySettingEntry>,
    /// 留不留 n0 公共中继兜底（ADR-0048：内置 n0 放最后一档）。
    pub n0: bool,
    /// `WONTON_RELAYS` 设了：表由环境变量固定，设置页只读。
    pub locked_by_env: bool,
    /// 核心读配置时的提示，比如签名的列表没验过、某一项被跳过。没有时为空。
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// 设置页这次是加载、编辑还是保存。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum RelaySettingsPhase {
    /// 还没打开过设置页。
    #[default]
    Idle,
    /// 正在向核心取表。
    Loading,
    /// 表取到了，可以看和改。
    Ready,
    /// 正在保存。
    Saving,
    /// 取或存都失败了；`reason` 是给人看的一句话。
    Failed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// 比这个 App 新的核心给的状态；壳按 Idle 当没打开过处理。
    #[serde(other)]
    Other,
}

/// 当前这条连接走的中继。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RelayCurrent {
    /// 中继地址，规范成不带令牌的一种写法。
    pub url: String,
    /// 这条中继路径的往返时间，毫秒。样本不够时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<u32>,
}

/// 设置页的一整份 ViewModel。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RelaySettingsView {
    /// 设置页这次是加载、编辑还是保存；还没打开过设置页时为空，壳按 Idle 处理。
    ///
    /// 加 `uniffi(default = None)` 让 Kotlin/Swift 能省掉它：uniffi 0.32 的绑定生成器只支持
    /// 字面量默认值，枚举字段直接写 `uniffi(default)` 会报 `Invalid literal for enum type: Default`。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub phase: Option<RelaySettingsPhase>,
    /// 正在编辑的表。没在编辑时就是核心给的最近一份。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default))]
    pub entries: Vec<RelaySettingEntry>,
    /// 留不留 n0 兜底。缺省和核心一样是留。
    #[serde(default = "n0_default")]
    #[cfg_attr(feature = "uniffi", uniffi(default = true))]
    pub n0: bool,
    /// 环境变量固定，只读。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default))]
    pub locked_by_env: bool,
    /// 核心的提示。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default))]
    pub warnings: Vec<String>,
    /// 改过还没保存。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default))]
    pub dirty: bool,
    /// 取表失败的原因。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default))]
    pub load_problem: Option<String>,
    /// 保存失败的原因。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default))]
    pub save_problem: Option<String>,
}

impl Default for RelaySettingsView {
    fn default() -> Self {
        Self {
            phase: None,
            entries: Vec::new(),
            n0: n0_default(),
            locked_by_env: false,
            warnings: Vec::new(),
            dirty: false,
            load_problem: None,
            save_problem: None,
        }
    }
}

fn n0_default() -> bool {
    true
}

/// 设置页上的操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelaySettingsAction {
    /// 打开设置页，向核心取表。正在保存时等保存完。
    Refresh,
    /// 加一条。地址要做 [`Relay::validate`](../../wonton_tunnel/relays/struct.Relay.html) 的同一套检查：
    /// https、不带 userinfo、令牌单独放。加在同档的最后。
    Add {
        url: String,
        #[serde(default)]
        token: Option<SecretString>,
        #[serde(default)]
        quic_port: Option<u16>,
        /// 放在哪一档。空着放最后一档。
        #[serde(default)]
        tier: Option<u8>,
    },
    /// 改一条的令牌、端口或档位。三个可选字段的「空」按字段本身有没有「没有」这个状态算：
    /// `token` 空着不动令牌，给空串清掉令牌；`quic_port` 空着清掉自定义端口、回到默认端口；
    /// `tier` 空着不动档位（档位不能清，总有一档）。
    Update {
        url: String,
        #[serde(default)]
        token: Option<SecretString>,
        #[serde(default)]
        quic_port: Option<u16>,
        #[serde(default)]
        tier: Option<u8>,
    },
    /// 删一条。内置或签名的项删掉后记进配置文件，不再是「没配」。
    Remove { url: String },
    /// 挪一条到某一档，放在 `before` 那一条前面；`before` 为空时放这一档的最后。
    ///
    /// 表按档位排序，`before` 只决定同一档里的先后：`before` 就是自己时放这一档的最前；`before`
    /// 在别的档时按档位算，在更高档就放目标档的最后（仍排在它前面），在更低档就放目标档的开头。
    Move {
        url: String,
        tier: u8,
        #[serde(default)]
        before: Option<String>,
    },
    /// 留不留 n0 兜底。
    SetN0 { on: bool },
    /// 存下来：交给核心改表、写配置文件、当场换掉 endpoint 的中继。
    Save,
    /// 丢掉没保存的修改，回到核心给的那一份。
    Discard,
}

/// `PUT /_wonton/api/relays` 的请求体：整张表。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RelaySettingsUpdate {
    /// 留不留 n0 兜底。
    pub n0: bool,
    /// 按档位排好的整张表。核心拿它和内置、签名列表合并，算出的差集写进配置文件。
    pub entries: Vec<RelaySettingUpdate>,
}

/// 请求体里的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RelaySettingUpdate {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quic_port: Option<u16>,
    pub tier: u8,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// 每个字段都得带 uniffi 默认值：Kotlin/Swift 的无参构造 `RelaySettingsView()` 靠它们成立。
    /// serde 的 `{}` 只测 serde default，把字段上的 `uniffi(default…)` 删掉这条也不会红，所以直接查源码。
    #[test]
    fn every_settings_view_field_has_a_uniffi_default() {
        let source = include_str!("relays.rs");
        let section = source
            .split("pub struct RelaySettingsView {")
            .nth(1)
            .expect("RelaySettingsView 还在");
        let fields = section.split('}').next().expect("结构体有结尾");
        for field in [
            "phase",
            "entries",
            "n0",
            "locked_by_env",
            "warnings",
            "dirty",
            "load_problem",
            "save_problem",
        ] {
            let marker = format!("pub {field}:");
            let at = fields.find(&marker).expect("字段还在");
            let before = fields[..at].trim_end();
            let attr = before.rsplit('\n').next().unwrap_or("");
            assert!(
                attr.contains("uniffi(default"),
                "{field} 紧挨着的那一行不是 uniffi(default…)：{attr:?}"
            );
        }
    }

    #[test]
    fn unknown_source_and_state_fall_back_to_other() {
        assert_eq!(
            serde_json::from_value::<RelaySource>(json!("ceremony")).unwrap(),
            RelaySource::Other
        );
        assert_eq!(
            serde_json::from_value::<RelayState>(json!({"state": "hologram"})).unwrap(),
            RelayState::Other
        );
    }

    #[test]
    fn a_refused_relay_round_trips_with_its_reason() {
        let entry = RelaySettingEntry {
            url: "https://relay.example.org".into(),
            token: Some(SecretString::new("secret-token")),
            quic_port: Some(7842),
            tier: 0,
            source: RelaySource::Signed,
            state: RelayState::Refused {
                reason: Some("invalid token".into()),
            },
            editable: true,
        };
        let text = serde_json::to_string(&entry).unwrap();
        assert_eq!(
            serde_json::from_str::<RelaySettingEntry>(&text).unwrap(),
            entry
        );
        assert!(!format!("{entry:?}").contains("secret-token"));
    }

    #[test]
    fn an_add_action_round_trips_and_debug_redacts_the_token() {
        let action = RelaySettingsAction::Add {
            url: "https://relay.example.org".into(),
            token: Some(SecretString::new("secret-token")),
            quic_port: Some(7842),
            tier: None,
        };
        let text = serde_json::to_string(&action).unwrap();
        let parsed: RelaySettingsAction = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, action);
        assert!(!format!("{parsed:?}").contains("secret-token"));
    }

    #[test]
    fn an_empty_settings_view_reads_from_the_old_json() {
        let view: RelaySettingsView = serde_json::from_str("{}").unwrap();
        // 缺键按核心的缺省：n0 留着。
        assert_eq!(view, RelaySettingsView::default());
        assert!(view.n0);
    }

    #[test]
    fn an_unknown_phase_falls_back_to_other() {
        let view: RelaySettingsView =
            serde_json::from_str(r#"{"phase": {"phase": "reticulating"}}"#).unwrap();
        assert_eq!(view.phase, Some(RelaySettingsPhase::Other));
    }

    #[test]
    fn an_unknown_phase_value_is_other() {
        let phase: RelaySettingsPhase = serde_json::from_str(r#"{"phase":"hologram"}"#).unwrap();
        assert_eq!(phase, RelaySettingsPhase::Other);
    }
}
