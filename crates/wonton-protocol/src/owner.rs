//! 你的资料（ADR-0030）：显示名和邮箱在 App 里填，存在家里的服务器上，单点登录和要身份的插件从那里取。
//! 读写的接口是 [`ME_PATH`](crate::ME_PATH)（ADR-0045 起按人：主人写 [`OwnerProfileUpdate`]，家人改自己的）。

use serde::{Deserialize, Serialize};

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

pub const MAX_DISPLAY_NAME_CHARS: usize = 64;

/// 资料哪里不合规则。给人看的话由用的一方按自己的语言写（ADR-0043）；`Display` 是英文原文，进日志和“详情”。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProfileProblem {
    #[error("the name is empty")]
    NameEmpty,
    #[error("the name is longer than {MAX_DISPLAY_NAME_CHARS} characters")]
    NameTooLong,
    #[error("the name contains line breaks or control characters")]
    NameControlChars,
    #[error(
        "not an email address: write it like me@example.com, without spaces or control characters"
    )]
    EmailInvalid,
    #[error("an email address that was set can be changed but not cleared")]
    EmailCleared,
}

/// 显示名会原样进 OIDC 声明和代登录的请求头，有控制字符时写不进 HTTP。家里的服务器保存时查，引导在点完成之前也用它
/// 查，规则只写在这里。
pub fn check_display_name(name: &str) -> Result<(), ProfileProblem> {
    if name.is_empty() {
        return Err(ProfileProblem::NameEmpty);
    }
    if name.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(ProfileProblem::NameTooLong);
    }
    if name.chars().any(char::is_control) {
        return Err(ProfileProblem::NameControlChars);
    }
    Ok(())
}

/// 不验证能不能收信，只挡住明显不是邮箱的值：它会原样进 OIDC 声明和请求头。空的邮箱是“没填”，不经过这里。
pub fn check_email(email: &str) -> Result<(), ProfileProblem> {
    let valid = email.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty() && !domain.is_empty() && !domain.contains('@')
    }) && !email.chars().any(|c| c.is_whitespace() || c.is_control());
    if valid {
        Ok(())
    } else {
        Err(ProfileProblem::EmailInvalid)
    }
}

/// 填过的邮箱只能改，不能清空：邮箱清掉后，要邮箱的插件重建时会失败（ADR-0030）。`next` 已经过 [`normalized_email`]。
pub fn check_email_change(
    previous: Option<&str>,
    next: Option<&str>,
) -> Result<(), ProfileProblem> {
    match (previous, next) {
        (Some(_), None) => Err(ProfileProblem::EmailCleared),
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
