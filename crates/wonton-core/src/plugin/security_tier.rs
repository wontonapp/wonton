//! 商店、预览和详情页显示的安全档位（ADR-0080）：核心只按清单算，作者不能填。

use wonton_protocol::{PermissionRisk, SecurityTier};

use super::capabilities;
use super::manifest::{Manifest, Runtime};
use super::views_permissions::permission_risk;

/// 声明了的权限都算，包括默认关的可选权限：档位说的是插件能拿到什么，不是现在开了什么。
/// 第三方能力（`x-` 前缀）高不高看提供方（ADR-0053），只看使用方清单时判断不了，不算进来。
pub fn security_tier(manifest: &Manifest) -> SecurityTier {
    let connects = manifest
        .static_web
        .as_ref()
        .is_some_and(|spec| !spec.connect.is_empty());
    match manifest.runtime {
        Runtime::Link => return SecurityTier::Minimal,
        Runtime::StaticWeb if !connects => return SecurityTier::Minimal,
        _ => {}
    }
    let permissions =
        manifest
            .all_permissions()
            .filter_map(permission_risk)
            .map(|risk| match risk {
                PermissionRisk::High => SecurityTier::High,
                PermissionRisk::VeryHigh => SecurityTier::VeryHigh,
                PermissionRisk::Highest | PermissionRisk::Other => SecurityTier::Highest,
            });
    let high_capability = manifest
        .requires
        .iter()
        .chain(&manifest.wants)
        .filter_map(|name| capabilities::lookup(name))
        .any(|capability| capability.high_privilege)
        .then_some(SecurityTier::High);
    permissions
        .chain(high_capability)
        .max()
        .unwrap_or(SecurityTier::Standard)
}

/// 商店里要不要把安全档位醒目标红：High 及以上；比这个 App 新的档位（`Other`，按最高档画）也算。
/// 规则只在这里，壳不比较档位。
pub fn prominent(tier: SecurityTier) -> bool {
    !matches!(tier, SecurityTier::Minimal | SecurityTier::Standard)
}
