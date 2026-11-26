//! MSC4155 邀请过滤（ADR-0051 的“挡垃圾”、ADR-0088 第 9 节）。
//!
//! 名单归账号自己的客户端（wonton-client 写 `blocked_users`、`blocked_servers`，别家客户端写的字段原样保留），
//! 核心写两种内容：账号还没有账号数据时补“过滤开着、名单为空”，保证账号（包括之后用 Element 这类别家客户端
//! 登录的）一出生就是过滤着的；“加好友”要管理员批或不能的成员，核心把名单改成“本服务器的照常、家外的一律
//! 挡住”，角色放开时再把自己写的两份名单拿掉，成员自己写的（个人屏蔽）原样保留。
//!
//! 管理员批准过的外人记在 `allowed_users`（[`Matrix::allow_invitees`]）：这是核心写的第四份内容，角色放开时
//! 不拿掉，回到受限角色照样生效，同一个人不用再批一次。Tuwunel 的求值顺序里 `allowed_users` 在个人屏蔽之前，
//! 所以批准过的人连成员自己的 `blocked_users` 也挡不住，解屏蔽不是批准的前提。
//!
//! 事件类型用不稳定名：Tuwunel 只在这个名称上读名单（`org.matrix.msc4155.invite_permission_config` 的六份
//! 名单，Tuwunel v2.0.0 `src/service/users/invite_filter.rs` 的 `LISTS_TYPE`），稳定的
//! `m.invite_permission_config` 只认 MSC4380 的一刀切 `default_action`。六份名单的求值顺序是
//! allowed_users、ignored_users、blocked_users、allowed_servers、ignored_servers、blocked_servers，服务器的
//! 通配按去掉端口后的服务器名匹配（MSC4155 的处理逻辑），所以 `allowed_servers` 放本服务器、
//! `blocked_servers` 放 `*` 挡得住家外、挡不住家里人；`*` 匹配所有服务器名由 Tuwunel 自己的单元测试钉住。

use std::collections::BTreeMap;

use hyper::Method;
use serde_json::{Value, json};
use wonton_protocol::Gate;

use super::{Matrix, MatrixError, api};

pub use wonton_protocol::INVITE_FILTER_EVENT as INVITE_FILTER_TYPE;

/// 临时替账号登录的设备名，写完就吊销。
const DEVICE_NAME: &str = "Wonton 核心（邀请过滤）";

/// 默认内容：过滤开着，名单为空。
fn default_content() -> Value {
    json!({"enabled": true})
}

/// “加好友”受管角色的内容：本服务器的照常，家外的一律挡住。
fn restricted_content(server: &str) -> Value {
    json!({"enabled": true, "allowed_servers": [server], "blocked_servers": ["*"]})
}

/// 内容里的一个名单；缺了或不是字符串数组都按空算（MSC4155 规定整个事件非法时按没有）。
fn list_of<'a>(content: &'a Value, field: &str) -> Vec<&'a str> {
    content[field]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .collect()
}

/// 把限制合进现有内容：成员自己写的保留，只补核心要的三样。已有的不重复加。
fn restricted_from(existing: &Value, server: &str) -> Value {
    let mut merged = existing.as_object().cloned().unwrap_or_default();
    let mut allowed: Vec<String> = list_of(existing, "allowed_servers")
        .into_iter()
        .map(str::to_owned)
        .collect();
    if !allowed.iter().any(|name| name == server) {
        allowed.push(server.to_owned());
    }
    let mut blocked: Vec<String> = list_of(existing, "blocked_servers")
        .into_iter()
        .map(str::to_owned)
        .collect();
    if !blocked.iter().any(|glob| glob == "*") {
        blocked.push("*".to_owned());
    }
    merged.insert("enabled".into(), json!(true));
    merged.insert("allowed_servers".into(), json!(allowed));
    merged.insert("blocked_servers".into(), json!(blocked));
    Value::Object(merged)
}

/// 拿掉核心写的限制（本服务器的 allowed 和 `*` 的 blocked），成员自己写的保留。
/// 核心什么都没写过时返回没有：内容不用改，也不用登录。
fn relaxed_from(existing: &Value, server: &str) -> Option<Value> {
    let ours = list_of(existing, "allowed_servers").contains(&server)
        || list_of(existing, "blocked_servers").contains(&"*");
    if !ours {
        return None;
    }
    let mut merged = existing.as_object().cloned().unwrap_or_default();
    let mut keep = |field: &str, drop: &str| {
        let kept: Vec<&str> = list_of(existing, field)
            .into_iter()
            .filter(|name| name != &drop)
            .collect();
        if kept.is_empty() {
            merged.remove(field);
        } else {
            merged.insert(field.into(), json!(kept));
        }
    };
    keep("allowed_servers", server);
    keep("blocked_servers", "*");
    Some(Value::Object(merged))
}

/// 这个角色此刻该有的内容；返回没有表示记录里已经是它，不用重写。
fn wanted_content(stored: Option<&Value>, restricted: bool, server: &str) -> Option<Value> {
    match (stored, restricted) {
        (None, false) => None,
        (None, true) => Some(restricted_content(server)),
        (Some(existing), false) => relaxed_from(existing, server),
        (Some(existing), true) => {
            let merged = restricted_from(existing, server);
            (merged != *existing).then_some(merged)
        }
    }
}

/// 批准过的人合进现有内容：核心要的三样照写，名单里追加，别的原样保留。
/// 已经在名单里的不重复加；账号还没有账号数据时从受限内容起步。
fn approved_from(existing: Option<&Value>, server: &str, users: &[String]) -> Value {
    let mut merged = existing.map_or_else(
        || restricted_content(server),
        |existing| restricted_from(existing, server),
    );
    let mut allowed: Vec<String> = list_of(&merged, "allowed_users")
        .into_iter()
        .map(str::to_owned)
        .collect();
    for user in users {
        if !allowed.iter().any(|present| present == user) {
            allowed.push(user.clone());
        }
    }
    merged["allowed_users"] = json!(allowed);
    merged
}

impl Matrix {
    /// 管理员批准了这些人：记进账号的 `allowed_users`，之后他们发来的邀请就是放行的。
    /// 要在这个成员还在管理员批的状态下调用（角色放开的不用记）；账号还没有就顺带建号。
    pub async fn allow_invitees(
        &self,
        localpart: &str,
        users: &[String],
    ) -> Result<(), MatrixError> {
        if users.is_empty() {
            return Ok(());
        }
        let server = self.server_name.clone();
        let users = users.to_vec();
        self.login_write(localpart, move |stored| {
            Ok(Some(approved_from(stored.as_ref(), &server, &users)))
        })
        .await
    }

    /// 已有的按“加好友”项改。写不进去就不发令牌，账号不能带着没过滤的状态上线。
    pub async fn ensure_invite_filter(
        &self,
        localpart: &str,
        add_friends: Gate,
    ) -> Result<(), MatrixError> {
        self.align(localpart, add_friends, true).await
    }

    /// 成员重同步：只对齐已有账号的过滤。账号还没有的收不到邀请，等发登录令牌时再写，不在这里建号。
    pub async fn sync_invite_filter(
        &self,
        localpart: &str,
        add_friends: Gate,
    ) -> Result<(), MatrixError> {
        self.align(localpart, add_friends, false).await
    }

    async fn align(
        &self,
        localpart: &str,
        add_friends: Gate,
        create: bool,
    ) -> Result<(), MatrixError> {
        let restricted = !matches!(add_friends, Gate::Allow);
        let server = self.server_name.clone();
        let user_id = self.user_id(localpart);
        if !self.user_exists(&user_id).await? {
            if !create {
                return Ok(());
            }
            // 建号的那次登录顺带写：哪怕角色不限制也写默认，账号一出生就是过滤着的（ADR-0051）。
            return self
                .login_write(localpart, move |stored| {
                    Ok(match stored {
                        None => Some(if restricted {
                            restricted_content(&server)
                        } else {
                            default_content()
                        }),
                        Some(existing) => wanted_content(Some(&existing), restricted, &server),
                    })
                })
                .await;
        }
        // 先用管理接口读一遍，该写的才临时登录：平时的重同步只读不登录、不建设备。
        let stored = self.global_account_data(&user_id).await?;
        if wanted_content(stored.get(INVITE_FILTER_TYPE), restricted, &server).is_none() {
            return Ok(());
        }
        // 写的时候以账号会话里刚读到的一份为准：和自己客户端的并发写不打架。
        self.login_write(localpart, move |stored| {
            Ok(wanted_content(stored.as_ref(), restricted, &server))
        })
        .await
    }

    /// 临时替账号登录一台设备，按账号会话里最新读到的内容决定写什么，写完吊销设备。
    /// 返回没有表示不用写。要先判断要不要写时用管理接口读，别为一次只读登录建设备。
    async fn login_write(
        &self,
        localpart: &str,
        wanted: impl Fn(Option<Value>) -> Result<Option<Value>, MatrixError>,
    ) -> Result<(), MatrixError> {
        let user_id = self.user_id(localpart);
        let login = self.login(localpart, DEVICE_NAME).await?;
        let session = self.session(&login);
        let written = async {
            match wanted(session.account_data(INVITE_FILTER_TYPE).await?)? {
                Some(content) => session.put_account_data(INVITE_FILTER_TYPE, &content).await,
                None => Ok(()),
            }
        }
        .await;
        let revoked = self.delete_device(&user_id, &login.device_id).await;
        written.and(revoked)
    }

    /// 账号的全部全局账号数据（类型 → 内容）：Synapse 兼容的 `GET /_synapse/admin/v1/users/{user}/accountdata`。
    pub(super) async fn global_account_data(
        &self,
        user_id: &str,
    ) -> Result<BTreeMap<String, Value>, MatrixError> {
        let path = format!(
            "/_synapse/admin/v1/users/{}/accountdata",
            api::encode(user_id)
        );
        let value = self.admin(Method::GET, &path, None, "读账号数据").await?;
        Ok(value["account_data"]["global"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(kind, content)| (kind.clone(), content.clone()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn restricted_content_allows_local_and_blocks_everything_else() {
        assert_eq!(
            restricted_content("filter.test"),
            json!({"enabled": true, "allowed_servers": ["filter.test"], "blocked_servers": ["*"]})
        );
    }

    #[test]
    fn restriction_merges_into_what_the_member_wrote() {
        let existing = json!({"enabled": true, "blocked_users": ["@spam:elsewhere.test"]});
        assert_eq!(
            restricted_from(&existing, "home.test"),
            json!({
                "enabled": true,
                "blocked_users": ["@spam:elsewhere.test"],
                "allowed_servers": ["home.test"],
                "blocked_servers": ["*"],
            })
        );
        let already = restricted_content("home.test");
        assert_eq!(
            wanted_content(Some(&already), true, "home.test"),
            None,
            "已经是受限内容就不重写"
        );
    }

    #[test]
    fn relaxation_removes_only_what_core_wrote() {
        let restricted = restricted_content("home.test");
        assert_eq!(
            relaxed_from(&restricted, "home.test"),
            Some(default_content())
        );

        let merged = restricted_from(
            &json!({"enabled": true, "blocked_users": ["@spam:elsewhere.test"]}),
            "home.test",
        );
        assert_eq!(
            relaxed_from(&merged, "home.test"),
            Some(json!({"enabled": true, "blocked_users": ["@spam:elsewhere.test"]})),
            "成员自己的屏蔽保留"
        );

        let member_wrote = json!({"enabled": true, "blocked_servers": ["spam.example"]});
        assert_eq!(
            wanted_content(Some(&member_wrote), false, "home.test"),
            None,
            "核心没写过的内容不动"
        );
    }

    #[test]
    fn approval_appends_to_the_allow_list_and_keeps_blocks() {
        let existing =
            json!({"enabled": true, "blocked_users": ["@spam:x"], "allowed_users": ["@a:x"]});
        assert_eq!(
            approved_from(
                Some(&existing),
                "home.test",
                &["@b:y".to_owned(), "@a:x".to_owned()]
            ),
            json!({
                "enabled": true,
                "blocked_users": ["@spam:x"],
                "allowed_users": ["@a:x", "@b:y"],
                "allowed_servers": ["home.test"],
                "blocked_servers": ["*"],
            }),
            "已经在名单里的不重复加，个人屏蔽不动"
        );
    }

    #[test]
    fn relaxation_keeps_approvals() {
        let approved = approved_from(None, "home.test", &["@b:y".to_owned()]);
        assert_eq!(
            relaxed_from(&approved, "home.test"),
            Some(json!({"enabled": true, "allowed_users": ["@b:y"]})),
            "角色放开时批准过的人留着"
        );
    }
}
