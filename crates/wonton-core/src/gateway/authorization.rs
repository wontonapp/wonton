//! 这个身份能做什么：权限解析、插件管理员判断和角色人数。

use wonton_protocol::{MemberStatus, Permissions};

use crate::roles::RolesError;

use super::*;

impl Gateway {
    /// 这个人现在的权限：每次请求按记录里的角色解析，角色改了下一个请求就生效。主人短路为全部权限；
    /// 记录被移除、或者记录里的角色不在了，按最严的 [`Permissions::strictest`] 算，不冒充能给。
    pub(crate) fn permissions(&self, person: &Person) -> Permissions {
        if person.is_owner() {
            return Permissions::owner();
        }
        let Some(record) = self.members.get(&person.username) else {
            return Permissions::strictest();
        };
        if record.status == MemberStatus::Removed {
            return Permissions::strictest();
        }
        self.roles
            .get(&record.role_id)
            .map_or_else(Permissions::strictest, |role| role.permissions)
    }
    /// 这个人是否在指定应用里具备插件管理员能力。应用未对他开放时，管理员范围也不生效。
    pub(crate) fn plugin_admin(&self, person: &Person, plugin_id: &str) -> bool {
        if person.is_owner() {
            return true;
        }
        if person.role == Role::Friend {
            return false;
        }
        let permissions = self.permissions(person);
        permissions.open_apps.allows_plugin(plugin_id)
            && permissions.plugin_admin.allows_plugin(plugin_id)
    }

    /// 这个角色有几人在用：没移除的家人和还在名单里的好友。角色接口接上之前只有测试用。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn role_people(&self, role_id: &str) -> u32 {
        let members = self
            .members
            .list()
            .iter()
            .filter(|m| m.status != MemberStatus::Removed && m.role_id == role_id)
            .count();
        let friends = self.friends.as_ref().map_or(0, |friends| {
            friends
                .list_active()
                .iter()
                .filter(|f| f.role_id == role_id)
                .count()
        });
        u32::try_from(members + friends).unwrap_or(u32::MAX)
    }

    /// 删一个角色。用的人数按这一刻的记录算：先把人换走，再删。角色接口接上之前只有测试用。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn delete_role(&self, role_id: &str) -> Result<(), RolesError> {
        self.roles.delete(role_id, self.role_people(role_id))
    }
}
