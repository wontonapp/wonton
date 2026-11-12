//! 清单里 `webui.oidc` 的字段规则。

use std::collections::BTreeSet;

use super::manifest::{ManifestError, OidcClient, RoleClaim};

/// `admin_groups` 最多这么多个。
pub const MAX_ADMIN_GROUPS: usize = 8;

/// 组名的最长字符数。
const MAX_GROUP_NAME: usize = 64;

/// 应用自己的声明不能占用的名字：核心已经在发的，和 OIDC 规定的。
const RESERVED_CLAIMS: &[&str] = &[
    "sub",
    "iss",
    "aud",
    "exp",
    "iat",
    "nbf",
    "jti",
    "nonce",
    "auth_time",
    "acr",
    "amr",
    "azp",
    "at_hash",
    "c_hash",
    "sid",
    "name",
    "preferred_username",
    "email",
    "email_verified",
    "groups",
    "locale",
];

/// 校验 serde 表达不了的规则：组名的字符、长度、去重和个数上限；`role_claim` 的名字和取值。
pub(super) fn validate(client: &OidcClient) -> Result<(), ManifestError> {
    if let Some(claim) = &client.role_claim {
        validate_role_claim(claim)?;
    }
    if client.admin_groups.len() > MAX_ADMIN_GROUPS {
        return Err(ManifestError::Invalid(format!(
            "webui.oidc.admin_groups allows at most {MAX_ADMIN_GROUPS} entries"
        )));
    }
    let mut seen = BTreeSet::new();
    for name in &client.admin_groups {
        if !is_group_name(name) {
            return Err(ManifestError::Invalid(format!(
                "webui.oidc.admin_groups entry `{name}` is invalid: 1 to {MAX_GROUP_NAME} characters, \
                 only ASCII letters, digits, and _-.:"
            )));
        }
        if !seen.insert(name.as_str()) {
            return Err(ManifestError::Invalid(format!(
                "webui.oidc.admin_groups entry `{name}` is duplicated"
            )));
        }
    }
    Ok(())
}

fn validate_role_claim(claim: &RoleClaim) -> Result<(), ManifestError> {
    if !is_group_name(&claim.name) || RESERVED_CLAIMS.contains(&claim.name.as_str()) {
        return Err(ManifestError::Invalid(format!(
            "webui.oidc.role_claim.name `{}` is invalid: 1 to {MAX_GROUP_NAME} characters, only ASCII \
             letters, digits, and _-.:, and not a claim the core already sends",
            claim.name
        )));
    }
    for (field, value) in [("admin", &claim.admin), ("user", &claim.user)] {
        if !is_group_name(value) {
            return Err(ManifestError::Invalid(format!(
                "webui.oidc.role_claim.{field} `{value}` is invalid: 1 to {MAX_GROUP_NAME} characters, \
                 only ASCII letters, digits, and _-.:"
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

#[cfg(test)]
mod tests {
    use super::*;

    fn client(admin_groups: &[&str]) -> OidcClient {
        OidcClient {
            redirect_paths: vec!["/cb".into()],
            post_logout_paths: Vec::new(),
            token_endpoint_auth_method: Default::default(),
            admin_groups: admin_groups.iter().map(|s| s.to_string()).collect(),
            role_claim: None,
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

    #[test]
    fn a_role_claim_needs_a_free_name_and_plain_values() {
        let mut immich = client(&[]);
        immich.role_claim = Some(RoleClaim {
            name: "immich_role".into(),
            admin: "admin".into(),
            user: "user".into(),
        });
        validate(&immich).unwrap();
        for (name, admin) in [
            ("groups", "admin"),
            ("sub", "admin"),
            ("immich_role", "管理员"),
        ] {
            immich.role_claim = Some(RoleClaim {
                name: name.into(),
                admin: admin.into(),
                user: "user".into(),
            });
            let error = validate(&immich).unwrap_err().to_string();
            assert!(error.contains("role_claim"), "{error}");
        }
    }
}
