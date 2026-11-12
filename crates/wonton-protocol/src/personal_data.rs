//! 手机的位置和健康（ADR-0098）：类型清单、按类型按 Bot 的授权页、`personal_data` 存储能力的契约和失败类别。
//!
//! 数据在手机上，存在存储提供方插件里；核心只判定授权，经这条能力契约存、查、删，不认得某个具体插件。
//! 提供方不直接给 Agent 注册工具，Agent 只经核心的 MCP 工具（[`PERSONAL_DATA_STATUS_TOOL`] 这一组）读。
//!
//! 契约的每个请求都带 [`crate::WONTON_USER_HEADER`]：数据属于谁（数据主体）。提供方按人分开存，授权不在这里判定。
//! 失败时回 4xx/5xx，响应体是 [`PersonalDataProblem`]：这个提供方不存这类数据用
//! [`PersonalDataFailure::NotSupported`]，提供方自己出错用 [`PersonalDataFailure::ProviderUnavailable`]，
//! 请求写法不对也归后者、原始原因写进 `message`。
//!
//! 核心可能比 App 新：多给的字段被忽略；枚举遇到不认识的取值时落到 `Other`，不认识的类型壳不显示。

use serde::{Deserialize, Serialize};

use crate::BotRef;

/// 核心认识的能力名（ADR-0053 的能力表、ADR-0098 第 5 节）。核心按它找存储提供方，不认插件 ID。
/// 核心没有这项时，清单里写它的插件装不上，要先升级核心。
pub const PERSONAL_DATA_CAPABILITY: &str = "personal_data";

/// `POST`：一个位置点，请求体是 [`LocationPoint`]（OwnTracks 的 `_type: location` 写法），回 [`WriteAccepted`]。
pub const LOCATION_PATH: &str = "/location";

/// `POST`：一批健康样本，请求体是 [`HealthSamples`]，回 [`WriteAccepted`]。
pub const HEALTH_PATH: &str = "/health";

/// `GET` 的查询串，回 [`DataQueryResult`]：`?type=<类型名>&from=<Unix 毫秒>&to=<Unix 毫秒>`，两端闭区间。
/// `type` 取 [`PersonalDataType::as_str`]。
pub const QUERY_PATH: &str = "/query";

/// `DELETE` 的查询串，回 [`CleanupResult`]：写法同 [`query_path`]，只删命中的那些。
pub const DATA_PATH: &str = "/data";

/// `GET`：这个人的全部数据，回 [`ExportBundle`]。
pub const EXPORT_PATH: &str = "/export";

/// `POST`：按保存期限删到期数据，请求体是 [`CleanupRequest`]，回 [`CleanupResult`]。核心定期调。
pub const CLEANUP_PATH: &str = "/cleanup";

/// [`QUERY_PATH`] 的查询串。类型名是 ASCII 的下划线写法，原样拼进去，不转义。
pub fn query_path(type_name: &str, from_ms: u64, to_ms: u64) -> String {
    format!("{QUERY_PATH}?type={type_name}&from={from_ms}&to={to_ms}")
}

/// [`DATA_PATH`] 的查询串。
pub fn data_path(type_name: &str, from_ms: u64, to_ms: u64) -> String {
    format!("{DATA_PATH}?type={type_name}&from={from_ms}&to={to_ms}")
}

/// 工具结果 `_meta` 里的失败类别键，值是 [`PersonalDataFailure`] 的一项。文字另给，给 Agent 看。
pub const FAILURE_META: &str = "app.wonton/failure";

/// 按需查一次位置的工具：只读，核心核对授权后唤醒手机，不发确认卡片（ADR-0024 第 2 步的“不敏感”）。
pub const LOCATION_NOW_TOOL: &str = "wonton__location_now";

/// 查时间段内的位置点，读服务器上已同步的数据，带每点的测量时间。
pub const LOCATION_HISTORY_TOOL: &str = "wonton__location_history";

/// 查某类健康数据在时间段内的样本或按天汇总；结果是服务器上的数据，带 `synced_through`。
pub const HEALTH_QUERY_TOOL: &str = "wonton__health_query";

/// 请手机同步一次某类健康数据；不懂唤醒手机取数。
pub const HEALTH_REFRESH_TOOL: &str = "wonton__health_refresh";

/// 这个 Bot 能读哪些类型、各自最后同步时间、系统授权状态。
pub const PERSONAL_DATA_STATUS_TOOL: &str = "wonton__personal_data_status";

/// 按需查位置的等待上限（毫秒）：静默推送唤醒后的执行窗口。到时没回来按
/// [`PersonalDataFailure::DeviceNoResponse`] 报，不退回同步来的旧点冒充“现在”。
pub const LOCATION_REQUEST_TIMEOUT_MS: u64 = 30_000;

/// 每人每小时按需查位置的次数上限，超过直接报 [`PersonalDataFailure::RateLimited`]，不发推送。
pub const LOCATION_REQUESTS_PER_HOUR: u32 = 12;

/// 手机交回的位置比请求时刻早这么多（毫秒）就按缓存点处理：显著位置变化的第一条常是旧位置。
pub const CACHED_LOCATION_MAX_AGE_MS: u64 = 2 * 60 * 1000;

/// 位置默认保存天数。
pub const DEFAULT_LOCATION_RETENTION_DAYS: u32 = 30;

/// 健康默认保存天数。
pub const DEFAULT_HEALTH_RETENTION_DAYS: u32 = 365;

/// 数据的大类。位置和健康的授权、按需查、失败原因都不一样。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PersonalDataCategory {
    Location,
    Health,
    #[serde(other)]
    Other,
}

/// 核心定的类型清单，第一批按 ADR-0098 第 3 节：位置加七项健康。清单只增不改名，数据的 JSON 里写
/// [`Self::as_str`] 的取值，换名字等于换类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PersonalDataType {
    Location,
    /// 步数。
    Steps,
    /// 步行加跑步的距离。
    WalkingRunningDistance,
    /// 活动能量。
    ActiveEnergy,
    /// 心率样本。
    HeartRate,
    /// 静息心率。
    RestingHeartRate,
    /// 睡眠的一段，阶段见 [`HealthValue::Sleep`]。
    Sleep,
    /// 体重。
    BodyWeight,
    /// 比 App 新的核心给的未知类型。
    #[serde(other)]
    Other,
}

impl PersonalDataType {
    /// 清单里的名字，也是数据的 JSON 里写的类型名。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Location => "location",
            Self::Steps => "steps",
            Self::WalkingRunningDistance => "walking_running_distance",
            Self::ActiveEnergy => "active_energy",
            Self::HeartRate => "heart_rate",
            Self::RestingHeartRate => "resting_heart_rate",
            Self::Sleep => "sleep",
            Self::BodyWeight => "body_weight",
            Self::Other => "other",
        }
    }

    pub fn category(self) -> PersonalDataCategory {
        match self {
            Self::Location => PersonalDataCategory::Location,
            Self::Other => PersonalDataCategory::Other,
            _ => PersonalDataCategory::Health,
        }
    }
}

/// 系统权限的状态。HealthKit 看不出用户拒绝了读取（拒绝时就像没有数据），健康的各项只能是 [`Self::Unknown`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum OsPermissionState {
    Granted,
    /// 系统里关了；已存的数据不动，只停上报。
    Missing,
    /// 看不出来，按没有新数据判断。
    Unknown,
    #[serde(other)]
    Other,
}

/// 要系统给多少位置权限。“只在打开 App 时”只要使用期间，“后台同步和随时查”才要始终。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum LocationScope {
    Foreground,
    Background,
    #[serde(other)]
    Other,
}

/// 存多久。到期数据由提供方删。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "policy", rename_all = "snake_case")]
pub enum RetentionPolicy {
    /// 不保存：收下就删。
    Never,
    Days {
        days: u32,
    },
    /// 比 App 新的核心给的新期限；按最长的处理，别提前删。
    #[serde(other)]
    Other,
}

/// 失败的一项和原始说明：工具结果里给 Agent 看，协议回错时给核心看；用户看到的句子在壳的本地化资源里。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PersonalDataProblem {
    pub reason: PersonalDataFailure,
    /// 核心写的说明，原样给 Agent 或主人看。
    pub message: String,
}

/// 取不到的原因（ADR-0098 第 6 节）：分开报，不让 Agent 猜是哪一种。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PersonalDataFailure {
    /// 这类数据没打开、没勾这个 Bot，或说话人不是数据主体。
    PermissionDenied,
    /// 系统权限没给或关了。健康看不出，不报这个。
    OsPermissionMissing,
    /// 这个人没有开了该类型的设备，或者推送渠道没登记。
    DeviceUnreachable,
    /// 推送发出去到超时也没回应：手机关机、没网、被划掉或被系统限流都可能，一并照实列，不挑一个。
    DeviceNoResponse,
    /// 健康：手机锁着读不到。
    DeviceLocked,
    /// 每小时上限到了，带下次能查的时刻。
    RateLimited,
    /// 平台没有这种数据（iPad 没有 HealthKit、Android 版没做健康）。
    NotSupported,
    /// 存储提供方没装、没就绪或出错，带原始错误。
    ProviderUnavailable,
    #[serde(other)]
    Other,
}

/// “设置 → 手机数据”页。整页只属于数据主体自己；共享 Bot、别人的 Bot、管理员都读不到这里的数据。
///
/// 没有系统权限的类型也照常列出，让主人知道有这项。开关、Bot 的勾选和保存期限都由核心存，
/// 这里只给壳画。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PersonalDataViewModel {
    /// 还没取到整页时为 true；这时 [`Self::types`] 为空，不代表一项都没有。
    pub loading: bool,
    /// 取整页失败的原因，这时显示上次取到的列表；再取得成功后清空。
    pub problem: Option<PersonalDataProblem>,
    /// 按清单顺序，不管系统怎么排。
    pub types: Vec<PersonalDataTypeRow>,
}

/// 一个类型的一行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PersonalDataTypeRow {
    pub data_type: PersonalDataType,
    /// 这个人在这一项上的开关。
    pub enabled: bool,
    /// 平台有没有这种数据；没有时开关只读，原因按 [`PersonalDataFailure::NotSupported`] 写在壳里。
    pub supported: bool,
    /// 只有位置有；“只在打开 App 时”和“后台同步和随时查”是两档。
    pub location_scope: Option<LocationScope>,
    pub os_permission: OsPermissionState,
    pub retention: RetentionPolicy,
    /// 看过本页披露，可以打开开关。打开的顺序是披露、系统权限弹窗、选 Bot。
    pub disclosure_acknowledged: bool,
    /// 同意“Bot 读到的内容会发给它使用的模型服务”；只有健康有。
    pub model_disclosure_acknowledged: Option<bool>,
    /// 能读这个类型的私人 Bot，只有他的 Bot；没打开时为空。
    pub bots: Vec<PersonalDataBotGrant>,
    /// 这类数据同步到什么时候；没同步过或不是健康时为空。
    pub synced_through_ms: Option<u64>,
    /// 健康太久没有新数据，可能在系统里关掉了。HealthKit 看不出拒绝，所以只提示，不当作已撤回。
    pub stale: bool,
    /// 最近的读取，新的在前。
    pub recent_reads: Vec<PersonalDataRead>,
}

/// 一个 Bot 在这个类型上的授权。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PersonalDataBotGrant {
    pub bot: BotRef,
    /// 勾上才能读；打开类型时默认只勾他的默认 Agent。
    pub enabled: bool,
    /// Bot 用的模型服务名，披露页要显示它。没配模型时为空。
    pub model_service: Option<String>,
}

/// 一次读取的审计，按类型列在页上。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PersonalDataRead {
    pub bot: BotRef,
    /// Unix 毫秒。
    pub at_ms: u64,
}

/// OwnTracks 的 `_type`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum LocationPointKind {
    Location,
    #[serde(other)]
    Other,
}

/// [`LOCATION_PATH`] 的请求体：OwnTracks 的 `_type: location` 写法，多家存储都收。这里定的是必须认的字段，
/// 多带的（电量、速度、海拔）提供方忽略。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LocationPoint {
    #[serde(rename = "_type")]
    pub kind: LocationPointKind,
    /// 纬度和经度，WGS84 十进制度。
    pub lat: f64,
    pub lon: f64,
    /// 水平精度（米）。
    pub acc: f64,
    /// 测量时间（Unix 秒），不是上报时间。
    pub tst: u64,
    /// 来源设备；同一个人的几台设备要能分开。
    pub tid: String,
}

/// 睡眠段的阶段，取自 HealthKit 的睡眠分析取值。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum SleepStage {
    InBed,
    Asleep,
    Core,
    Deep,
    Rem,
    Awake,
    #[serde(other)]
    Other,
}

/// 健康样本的值。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HealthValue {
    /// 带单位的数值：步数 `count`、距离 `m`、能量 `kcal`、心率 `count/min`、体重 `kg`。
    Quantity { value: f64, unit: String },
    /// 睡眠的一段，不看数值。
    Sleep { stage: SleepStage },
    #[serde(other)]
    Other,
}

/// 一个健康样本。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct HealthSample {
    /// [`PersonalDataType::as_str`] 的类型名；比提供方新的名字原样存。
    pub type_name: String,
    /// 段的开始（Unix 毫秒）；点样本等于结束时刻。
    pub start_ms: u64,
    /// 段的结束；点样本为空。
    pub end_ms: Option<u64>,
    pub value: HealthValue,
    /// 手机侧的去重键，同一段重传不变；提供方按它去重。
    pub source_id: String,
}

/// [`HEALTH_PATH`] 的请求体：一批样本，加上这批同步到了什么时候。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct HealthSamples {
    pub samples: Vec<HealthSample>,
    /// 这次同步到这个时刻（Unix 毫秒）。这次没取到新样本时也带，核心才知道没有缺口。
    pub synced_through_ms: Option<u64>,
}

/// 写入收了几条。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WriteAccepted {
    pub accepted: u32,
}

/// [`QUERY_PATH`] 的结果，按查的类型是两种形状之一。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataQueryResult {
    Location {
        /// 按测量时间从早到晚。
        points: Vec<LocationPoint>,
    },
    Health {
        samples: Vec<HealthSample>,
        /// 这个类型同步到什么时候；没同步过为空。缺口就是没有，不补造。
        synced_through_ms: Option<u64>,
    },
    #[serde(other)]
    Other,
}

/// [`EXPORT_PATH`] 的结果：一个人的全部数据。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExportBundle {
    pub location: Vec<LocationPoint>,
    pub health: Vec<HealthSample>,
    /// 导出时的时刻（Unix 毫秒）。
    pub exported_at_ms: u64,
}

/// 一个类型的保存期限，[`CleanupRequest`] 的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RetentionSetting {
    /// [`PersonalDataType::as_str`] 的类型名。
    pub type_name: String,
    pub retention: RetentionPolicy,
}

/// [`CLEANUP_PATH`] 的请求体：核心给的当前期限，按人。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CleanupRequest {
    /// 只管列出来的类型；[`RetentionPolicy::Never`] 的可以不列，提供方收下就删。
    pub policies: Vec<RetentionSetting>,
}

/// 删了几条，[`DATA_PATH`] 和 [`CLEANUP_PATH`] 的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CleanupResult {
    pub deleted: u64,
}

#[cfg(test)]
#[path = "personal_data_tests.rs"]
mod tests;
