//! 清单里 `webui.oidc` 的字段规则。

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

/// 代登录模板用了 `{email}` 时需要邮箱；compose 变量由渲染时的插值检查处理。
pub fn needs_owner_email(manifest: &Manifest) -> bool {
    super::proxy_auth::uses_email(manifest)
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
