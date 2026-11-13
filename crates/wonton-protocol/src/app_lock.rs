//! App 锁（ADR-0102）：人不在场时挡一眼。锁什么、什么时候锁由核心判定，壳只探测设备有没有锁屏、调系统
//! 验证、画锁定页；验证怎么过的核心不管，也不存验证结果。

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// 退到后台多久以后要求解锁。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AppLockTimeout {
    /// 一退到后台就锁。
    #[default]
    Immediate,
    OneMinute,
    FiveMinutes,
    FifteenMinutes,
}

impl AppLockTimeout {
    /// 按设置页里的显示顺序。
    pub const ALL: [Self; 4] = [
        Self::Immediate,
        Self::OneMinute,
        Self::FiveMinutes,
        Self::FifteenMinutes,
    ];

    /// 宽限期。判定只用核心的一个时刻，壳不各算一遍。
    pub fn span(self) -> Duration {
        match self {
            Self::Immediate => Duration::ZERO,
            Self::OneMinute => Duration::from_secs(60),
            Self::FiveMinutes => Duration::from_secs(300),
            Self::FifteenMinutes => Duration::from_secs(900),
        }
    }
}

/// App 锁的开关、宽限和此刻锁没锁。壳照 `locked` 盖锁定页，`problem` 是设置文件读不了的原因。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AppLockViewModel {
    pub enabled: bool,
    pub timeout: AppLockTimeout,
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub problem: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timeouts_grow_in_display_order() {
        let spans: Vec<Duration> = AppLockTimeout::ALL
            .iter()
            .map(|timeout| timeout.span())
            .collect();
        assert!(spans.windows(2).all(|pair| pair[0] < pair[1]), "{spans:?}");
        assert_eq!(AppLockTimeout::default(), AppLockTimeout::Immediate);
    }
}
