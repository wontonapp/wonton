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

/// 保存资料。名字必填；邮箱选填，为空或只有空白算没填。填过的邮箱只能改，不能清空（ADR-0030）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OwnerProfileUpdate {
    pub display_name: String,
    #[serde(default)]
    pub email: Option<String>,
}

const MAX_DISPLAY_NAME_CHARS: usize = 64;

/// 显示名会原样进 OIDC 声明和代登录的请求头，有控制字符时写不进 HTTP。家里的服务器保存时查，引导在点完成之前也用它
/// 查，规则只写在这里。
pub fn check_display_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("名字不能为空".to_owned());
    }
    if name.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(format!("名字太长：最多 {MAX_DISPLAY_NAME_CHARS} 个字"));
    }
    if name.chars().any(char::is_control) {
        return Err("名字里不能有换行和控制字符".to_owned());
    }
    Ok(())
}

/// 不验证能不能收信，只挡住明显不是邮箱的值：它会原样进 OIDC 声明和请求头。空的邮箱是“没填”，不经过这里。
pub fn check_email(email: &str) -> Result<(), String> {
    let valid = email.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty() && !domain.is_empty() && !domain.contains('@')
    }) && !email.chars().any(|c| c.is_whitespace() || c.is_control());
    if valid {
        Ok(())
    } else {
        Err("不像邮箱：写成 me@example.com 这样，没有空白和控制字符".to_owned())
    }
}

/// 填过的邮箱只能改，不能清空：邮箱清掉后，要邮箱的插件重建时会失败（ADR-0030）。`next` 已经过 [`normalized_email`]。
pub fn check_email_change(previous: Option<&str>, next: Option<&str>) -> Result<(), String> {
    match (previous, next) {
        (Some(_), None) => {
            Err("邮箱填过以后只能改，不能清空：要邮箱的应用重建时会用到它".to_owned())
        }
        (_, Some(email)) => check_email(email),
        (None, None) => Ok(()),
    }
}

/// 去掉首尾空白；空的邮箱算没填。
pub fn normalized_email(email: Option<&str>) -> Option<&str> {
    email.map(str::trim).filter(|email| !email.is_empty())
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

    #[test]
    fn an_update_without_an_email_leaves_it_empty() {
        let update: OwnerProfileUpdate =
            serde_json::from_value(json!({ "display_name": "Ann" })).unwrap();
        assert_eq!(update.email, None);
        assert_eq!(normalized_email(Some("  ")), None);
        assert_eq!(
            normalized_email(Some(" me@example.com ")),
            Some("me@example.com")
        );
    }

    #[test]
    fn emails_are_checked_for_shape_only() {
        assert_eq!(check_email("me@example.com"), Ok(()));
        assert_eq!(check_email("me@localhost"), Ok(()));
        for bad in ["me", "@example.com", "me@", "a@b@c", "a b@c", "a@b\n"] {
            assert!(check_email(bad).is_err(), "{bad:?}");
        }
    }
}
