//! AI 动手级别按当前记录的解析（ADR-0088 第 5 节）：总线的每次调用封顶和 Bot 容器的注入共用这一套。
//! 人或角色查不到、取值不认识时按只读并记日志，不按最松的放行。

use wonton_protocol::{AiLevel, MemberStatus};

use crate::members::Members;
use crate::roles::Roles;

/// 一个人的级别：主人不受角色约束；其余按他的角色。成员记录和角色记录任一查不到、状态不是已邀请或
/// 已加入时按只读并记日志。
pub fn person_ai(owner: &str, records: Option<(&Members, &Roles)>, username: &str) -> AiLevel {
    if username == owner {
        return AiLevel::SelfConfirms;
    }
    let level = records
        .and_then(|(members, roles)| {
            let member = members.get(username)?;
            if !matches!(member.status, MemberStatus::Invited | MemberStatus::Joined) {
                return None;
            }
            roles.get(&member.role_id).map(|role| role.permissions.ai)
        })
        .unwrap_or_else(|| {
            tracing::warn!(person = username, "认不出这个人的角色，AI 动手按只读封顶");
            AiLevel::ReadOnly
        });
    strict_ai(level)
}

/// 共享 Bot 的级别：能用人（角色“用共享 Bot”为“能”）里最低的，主人算在内。
pub fn shared_bot_ai(records: Option<(&Members, &Roles)>) -> AiLevel {
    let Some((members, roles)) = records else {
        tracing::warn!("没有家人角色记录，共享 Bot 的 AI 动手按只读封顶");
        return AiLevel::ReadOnly;
    };
    let mut floor = AiLevel::SelfConfirms;
    for member in members.list() {
        if !matches!(member.status, MemberStatus::Invited | MemberStatus::Joined) {
            continue;
        }
        let Some(role) = roles.get(&member.role_id) else {
            tracing::warn!(
                person = member.username.as_str(),
                "认不出这个人的角色，AI 动手按只读封顶"
            );
            floor = AiLevel::ReadOnly;
            continue;
        };
        if role.permissions.shared_bots {
            floor = lower(floor, strict_ai(role.permissions.ai));
        }
    }
    floor
}

/// 认不出的取值按最严算（ADR-0088 第 2 节）。
pub(crate) fn strict_ai(level: AiLevel) -> AiLevel {
    match level {
        AiLevel::Other => AiLevel::ReadOnly,
        level => level,
    }
}

/// 两级里低的那个：封顶取进行中的轮次和能用人里最低的级别。
pub(crate) fn lower(a: AiLevel, b: AiLevel) -> AiLevel {
    let rank = |level: AiLevel| match level {
        AiLevel::ReadOnly | AiLevel::Other => 0,
        AiLevel::AdminApprovesDeletes => 1,
        AiLevel::SelfConfirms => 2,
    };
    if rank(a) <= rank(b) { a } else { b }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::members::MemberRecord;
    use wonton_protocol::RoleDraft;

    struct Records {
        /// 临时目录只在字段里占位：成员记录的落盘路径在这里面，删了路径就没了。
        _dir: tempfile::TempDir,
        members: Members,
        roles: Roles,
    }

    impl Records {
        fn start() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let members = Members::open(dir.path().join("members.json")).unwrap();
            let roles = Roles::initial();
            Self {
                _dir: dir,
                members,
                roles,
            }
        }

        fn pair(&self) -> Option<(&Members, &Roles)> {
            Some((&self.members, &self.roles))
        }

        /// 加一个指定角色的成员，状态已加入。
        fn joined(&self, username: &str, role_id: &str) {
            let mut member = MemberRecord::for_test(username);
            member.status = MemberStatus::Joined;
            member.role_id = role_id.to_owned();
            self.members.invite(member).unwrap();
            self.members.mark_joined(username).unwrap();
        }
    }

    #[test]
    fn owner_is_outside_the_roles() {
        let records = Records::start();
        assert_eq!(
            person_ai("owner", records.pair(), "owner"),
            AiLevel::SelfConfirms
        );
    }

    #[test]
    fn member_level_follows_the_role() {
        let records = Records::start();
        records.joined("kid", "child");
        assert_eq!(
            person_ai("owner", records.pair(), "kid"),
            AiLevel::AdminApprovesDeletes
        );
        records.joined("pal", "friend");
        assert_eq!(person_ai("owner", records.pair(), "pal"), AiLevel::ReadOnly);
    }

    #[test]
    fn unknown_person_role_or_value_is_read_only() {
        let records = Records::start();
        assert_eq!(
            person_ai("owner", records.pair(), "ghost"),
            AiLevel::ReadOnly
        );
        assert_eq!(person_ai("owner", None, "ghost"), AiLevel::ReadOnly);
        // 记录里指向不存在的角色，和取值不认识的级别一样按只读。
        records.joined("ghost-role", "gone");
        assert_eq!(
            person_ai("owner", records.pair(), "ghost-role"),
            AiLevel::ReadOnly
        );
        let replacement = Roles::initial();
        let mut permissions = replacement.get("family").unwrap().permissions;
        permissions.ai = AiLevel::Other;
        replacement
            .update(
                "family",
                &RoleDraft {
                    name: "Family".into(),
                    permissions,
                },
            )
            .unwrap();
        records.joined("odd", "family");
        assert_eq!(
            person_ai("owner", Some((&records.members, &replacement)), "odd"),
            AiLevel::ReadOnly
        );
    }

    #[test]
    fn invited_members_count_removed_do_not() {
        let records = Records::start();
        records
            .members
            .invite(MemberRecord::for_test("waiter"))
            .unwrap();
        assert_eq!(
            person_ai("owner", records.pair(), "waiter"),
            AiLevel::ReadOnly,
            "默认邀请角色是“朋友”，只读"
        );
        records.joined("leaver", "family");
        records.members.remove("leaver", false, true).unwrap();
        assert_eq!(
            person_ai("owner", records.pair(), "leaver"),
            AiLevel::ReadOnly
        );
    }

    #[test]
    fn shared_bot_floor_is_the_lowest_among_users() {
        let records = Records::start();
        records.joined("kid", "child");
        assert_eq!(shared_bot_ai(records.pair()), AiLevel::AdminApprovesDeletes);
        // 角色里“用共享 Bot”关了的人不算：朋友是只读但用不了共享 Bot，就不封顶。
        records.joined("pal", "friend");
        assert_eq!(shared_bot_ai(records.pair()), AiLevel::AdminApprovesDeletes);
        // 把朋友的“用共享 Bot”打开：能用的人里最低的级别生效。
        let replacement = Roles::initial();
        let mut permissions = replacement.get("friend").unwrap().permissions;
        permissions.shared_bots = true;
        replacement
            .update(
                "friend",
                &RoleDraft {
                    name: "Friend".into(),
                    permissions,
                },
            )
            .unwrap();
        assert_eq!(
            shared_bot_ai(Some((&records.members, &replacement))),
            AiLevel::ReadOnly
        );
    }

    #[test]
    fn shared_bot_without_records_is_read_only() {
        assert_eq!(shared_bot_ai(None), AiLevel::ReadOnly);
    }
}
