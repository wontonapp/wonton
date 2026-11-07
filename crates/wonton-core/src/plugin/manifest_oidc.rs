//! 清单里 `webui.oidc` 的字段规则，和“装这个插件要不要主人的邮箱”的统一判断。

use std::collections::BTreeSet;

use super::manifest::{Manifest, ManifestError, OidcClient};

/// `owner_groups` 最多这么多个。
pub const MAX_OWNER_GROUPS: usize = 8;

/// 组名的最长字符数。
const MAX_GROUP_NAME: usize = 64;

/// 校验 serde 表达不了的规则：组名的字符、长度、去重和个数上限。
pub(super) fn validate(client: &OidcClient) -> Result<(), ManifestError> {
    if client.owner_groups.len() > MAX_OWNER_GROUPS {
        return Err(ManifestError::Invalid(format!(
            "webui.oidc.owner_groups allows at most {MAX_OWNER_GROUPS} entries"
        )));
    }
    let mut seen = BTreeSet::new();
    for name in &client.owner_groups {
        if !is_group_name(name) {
            return Err(ManifestError::Invalid(format!(
                "webui.oidc.owner_groups entry `{name}` is invalid: 1 to {MAX_GROUP_NAME} characters, \
                 only ASCII letters, digits, and _-.:"
            )));
        }
        if !seen.insert(name.as_str()) {
            return Err(ManifestError::Invalid(format!(
                "webui.oidc.owner_groups entry `{name}` is duplicated"
            )));
        }
    }
    Ok(())
}

/// 组名放进 ID 令牌和 userinfo 的 `groups` 里，应用可能拿它建内部角色，限定在安全的字符里。
fn is_group_name(name: &str) -> bool {
    (1..=MAX_GROUP_NAME).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
}

/// 装这个插件要不要主人的邮箱：代登录模板用了 `{email}`（compose 里引用
/// `WONTON_OWNER_EMAIL` 的由渲染时的插值检查拦下），或 OIDC 声明了 `requires_email`。
/// OIDC 用 `email` 声明认人的应用不引用变量，声明 `requires_email` 让预览和安装时就报，不用等登录才失败。
pub fn needs_owner_email(manifest: &Manifest) -> bool {
    super::proxy_auth::uses_email(manifest)
        || manifest
            .webui
            .as_ref()
            .and_then(|webui| webui.oidc.as_ref())
            .is_some_and(|client| client.requires_email)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(owner_groups: &[&str]) -> OidcClient {
        OidcClient {
            redirect_paths: vec!["/cb".into()],
            post_logout_paths: Vec::new(),
            token_endpoint_auth_method: Default::default(),
            owner_groups: owner_groups.iter().map(|s| s.to_string()).collect(),
            requires_email: false,
        }
    }

    #[test]
    fn accepts_plain_names_and_rejects_bad_ones() {
        validate(&client(&["admin", "app.admin", "editors-2", "a:b._-"])).unwrap();
        for name in ["", "带中文", "has space", "sl/ash", &"x".repeat(65)] {
            let error = validate(&client(&[name])).unwrap_err().to_string();
            assert!(error.contains(name), "{error}");
        }
    }

    #[test]
    fn rejects_duplicates_and_more_than_eight() {
        let error = validate(&client(&["admin", "admin"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("duplicated"), "{error}");
        let error = validate(&client(&[
            "g0", "g1", "g2", "g3", "g4", "g5", "g6", "g7", "g8",
        ]))
        .unwrap_err()
        .to_string();
        assert!(error.contains("at most 8"), "{error}");
    }
}
