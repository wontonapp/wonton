//! 你的资料（ADR-0030）：显示名和邮箱在 App 里填，存在家里的服务器上，单点登录和要身份的插件从那里取。

use serde::{Deserialize, Serialize};

/// 读写资料的接口：`GET` 和 `PUT` 都返回 [`OwnerProfile`]，`PUT` 的请求体是 [`OwnerProfileUpdate`]。
pub const OWNER_PROFILE_PATH: &str = "/_wonton/api/owner-profile";

/// 家里的服务器上的资料。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OwnerProfile {
    /// 用户名，装好后不能改（ADR-0022）。
    pub username: String,
    /// 还没填时为空。
    pub display_name: Option<String>,
    /// 还没填时为空。
    pub email: Option<String>,
}

/// 保存资料。两项都要写：填过以后只能改，不能改回没有。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OwnerProfileUpdate {
    pub display_name: String,
    pub email: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_profile_not_filled_in_yet_has_no_name_and_no_email() {
        let profile: OwnerProfile = serde_json::from_value(json!({
            "username": "owner",
            "display_name": null,
            "email": null,
        }))
        .unwrap();
        assert_eq!(profile.username, "owner");
        assert_eq!(profile.display_name, None);
        assert_eq!(profile.email, None);
    }
}
