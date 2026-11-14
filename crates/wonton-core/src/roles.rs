//! 角色（ADR-0088）：记录在 `core/roles.json`，写法照 `members.json`：整份重写、0600、启动时读回。
//! 主人有全部权限、在角色之外；其余每个人一个角色。四个模板是新建角色的起点，谁在用、是默认邀请角色时删不掉。

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use wonton_protocol::{
    AiLevel, AppScope, Gate, Permissions, RoleDraft, RoleTemplate, check_display_name,
};

/// 同时存在的角色上限：家里人一只手数得过来，给壳一张能滚动的完整列表。
pub const MAX_ROLES: usize = 64;

/// 默认邀请角色（ADR-0088 第 4 节）：别的管理员发邀请时新人拿它，权限最少，删不掉。
pub const DEFAULT_INVITE_ROLE: &str = "friend";

/// 一个角色在 `roles.json` 里的写法。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleRecord {
    /// 核心分配、改名也不变的 ID。
    pub id: String,
    /// 主人起的名字；模板没改过名时为空，壳按 [`Self::template`] 本地化。
    #[serde(default)]
    pub name: String,
    /// 从哪个模板来；主人新建的角色没有。
    #[serde(default)]
    pub template: Option<RoleTemplate>,
    pub permissions: Permissions,
}

impl RoleRecord {
    fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RolesError {
    #[error("读写角色记录 {path} 失败：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("角色记录 {path} 已损坏：{message}")]
    Corrupt { path: PathBuf, message: String },
    #[error("{0}")]
    Invalid(String),
    #[error("累计的角色已经到上限 {MAX_ROLES} 个")]
    TooMany,
    #[error("已经有同名的角色：{0}")]
    DuplicateName(String),
    #[error("没有这个角色：{0}")]
    NotFound(String),
    #[error("角色 {0} 还有人在用，先给别人换掉")]
    InUse(String),
    #[error("默认邀请角色不能删，先把默认换成别的角色")]
    DefaultRole,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stored {
    roles: Vec<RoleRecord>,
    /// 别的管理员发邀请时新人拿的角色（[`RoleRecord::id`]）。
    default_invite_role: String,
}

/// 角色记录。整份重写、随数据目录备份恢复；四个模板在第一次启动时写进去（ADR-0088 第 3 节）。
pub struct Roles {
    /// 内存里的记录没有落盘路径（[`Self::initial`]），改动只留在进程里。
    path: Option<PathBuf>,
    state: Mutex<Stored>,
}

/// 四个模板的初始值（ADR-0088 第 3 节）。模板名留空：壳按模板本地化，主人改过名才存名字。
fn templates() -> Vec<RoleRecord> {
    let selected = || AppScope::Selected {
        plugin_ids: Vec::new(),
    };
    let base = |open_apps: AppScope, template: RoleTemplate| RoleRecord {
        id: String::new(),
        name: String::new(),
        template: Some(template),
        permissions: Permissions {
            open_apps,
            ..Permissions::strictest()
        },
    };
    let admin = RoleRecord {
        permissions: Permissions::owner(),
        ..base(AppScope::All, RoleTemplate::Admin)
    };
    let family = RoleRecord {
        permissions: Permissions {
            install_apps: Gate::AskAdmin,
            add_friends: Gate::Allow,
            private_bot: true,
            shared_bots: true,
            lend_own_bot: true,
            ..base(selected(), RoleTemplate::Family).permissions
        },
        ..base(selected(), RoleTemplate::Family)
    };
    let child = RoleRecord {
        permissions: Permissions {
            install_apps: Gate::AskAdmin,
            add_friends: Gate::AskAdmin,
            ai: AiLevel::AdminApprovesDeletes,
            private_bot: true,
            shared_bots: true,
            ..base(selected(), RoleTemplate::Child).permissions
        },
        ..base(selected(), RoleTemplate::Child)
    };
    vec![
        admin.with_id("admin"),
        family.with_id("family"),
        child.with_id("child"),
        base(selected(), RoleTemplate::Friend).with_id(DEFAULT_INVITE_ROLE),
    ]
}

fn seeded() -> Stored {
    Stored {
        roles: templates(),
        default_invite_role: DEFAULT_INVITE_ROLE.to_owned(),
    }
}

impl Roles {
    /// 读取记录；文件不存在时从四个模板开始。已损坏时启动失败，不覆盖。
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, RolesError> {
        let path = path.into();
        let stored = match fs::read_to_string(&path) {
            Ok(text) => {
                let stored: Stored =
                    serde_json::from_str(&text).map_err(|e| RolesError::Corrupt {
                        path: path.clone(),
                        message: e.to_string(),
                    })?;
                validate(&path, &stored)?;
                stored
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => seeded(),
            Err(source) => return Err(RolesError::Io { path, source }),
        };
        Ok(Self {
            path: Some(path),
            state: Mutex::new(stored),
        })
    }

    /// 只有模板、改动不落盘的一份：网关自己没接上角色记录时用它（测试和最小装配）。
    pub fn initial() -> Self {
        Self {
            path: None,
            state: Mutex::new(seeded()),
        }
    }

    /// 全部角色，按记录顺序（模板在前）。
    pub fn list(&self) -> Vec<RoleRecord> {
        self.lock().roles.clone()
    }

    pub fn get(&self, id: &str) -> Option<RoleRecord> {
        self.lock().roles.iter().find(|r| r.id == id).cloned()
    }

    pub fn default_invite_role(&self) -> String {
        self.lock().default_invite_role.clone()
    }

    /// 新建一个角色；ID 由核心分配，从 `role1` 起取最小的空号。
    pub fn create(&self, draft: &RoleDraft) -> Result<RoleRecord, RolesError> {
        let name = checked_name(&draft.name)?;
        let mut stored = self.lock();
        let mut roles = stored.roles.clone();
        if roles
            .iter()
            .any(|r| r.name.trim().eq_ignore_ascii_case(&name))
        {
            return Err(RolesError::DuplicateName(name));
        }
        if roles.len() >= MAX_ROLES {
            return Err(RolesError::TooMany);
        }
        let id = (1..)
            .map(|n| format!("role{n}"))
            .find(|id| !roles.iter().any(|r| &r.id == id))
            .expect("角色数有上限，空号总能找到");
        let record = RoleRecord {
            id,
            name,
            template: None,
            permissions: draft.permissions.clone(),
        };
        roles.push(record.clone());
        self.commit(&mut stored, roles)?;
        Ok(record)
    }

    /// 改名、改权限；模板标记和 ID 不变。
    pub fn update(&self, id: &str, draft: &RoleDraft) -> Result<RoleRecord, RolesError> {
        let name = checked_name(&draft.name)?;
        let mut stored = self.lock();
        let mut roles = stored.roles.clone();
        let Some(index) = roles.iter().position(|r| r.id == id) else {
            return Err(RolesError::NotFound(id.to_owned()));
        };
        if roles
            .iter()
            .enumerate()
            .any(|(i, r)| i != index && r.name.trim().eq_ignore_ascii_case(&name))
        {
            return Err(RolesError::DuplicateName(name));
        }
        roles[index].name = name;
        roles[index].permissions = draft.permissions.clone();
        let record = roles[index].clone();
        self.commit(&mut stored, roles)?;
        Ok(record)
    }

    /// 删一个角色。还有人在用、或者是默认邀请角色时拒绝；用的人数由调用方按当前记录算好传进来。
    pub fn delete(&self, id: &str, people: u32) -> Result<(), RolesError> {
        let mut stored = self.lock();
        if stored.default_invite_role == id {
            return Err(RolesError::DefaultRole);
        }
        if people > 0 {
            return Err(RolesError::InUse(id.to_owned()));
        }
        let mut roles = stored.roles.clone();
        let Some(index) = roles.iter().position(|r| r.id == id) else {
            return Err(RolesError::NotFound(id.to_owned()));
        };
        roles.remove(index);
        self.commit(&mut stored, roles)
    }

    /// 换默认邀请角色。
    pub fn set_default(&self, id: &str) -> Result<(), RolesError> {
        let mut stored = self.lock();
        if !stored.roles.iter().any(|r| r.id == id) {
            return Err(RolesError::NotFound(id.to_owned()));
        }
        let previous = std::mem::replace(&mut stored.default_invite_role, id.to_owned());
        let state = stored.clone();
        if let Err(e) = self.save(&state) {
            stored.default_invite_role = previous;
            return Err(e);
        }
        Ok(())
    }

    /// 插件的开放范围是角色授权的反向视图，只有一个事实源。
    pub fn plugin_audience(&self, plugin_id: &str) -> wonton_protocol::PluginAudience {
        wonton_protocol::PluginAudience {
            role_ids: self
                .lock()
                .roles
                .iter()
                .filter(|role| role.permissions.open_apps.allows_plugin(plugin_id))
                .map(|role| role.id.clone())
                .collect(),
        }
    }

    pub fn check_plugin_audience(
        &self,
        audience: &wonton_protocol::PluginAudience,
    ) -> Result<(), RolesError> {
        let stored = self.lock();
        check_audience(&stored.roles, audience)
    }

    /// 同一次写盘只改这个插件在各角色里的勾选，其他应用与 All 不变。
    pub fn set_plugin_audience(
        &self,
        plugin_id: &str,
        audience: &wonton_protocol::PluginAudience,
    ) -> Result<wonton_protocol::PluginAudience, RolesError> {
        let mut stored = self.lock();
        check_audience(&stored.roles, audience)?;
        let mut roles = stored.roles.clone();
        for role in &mut roles {
            if let AppScope::Selected { plugin_ids } = &mut role.permissions.open_apps {
                plugin_ids.retain(|id| id != plugin_id);
                if audience.role_ids.contains(&role.id) {
                    plugin_ids.push(plugin_id.to_owned());
                }
            }
        }
        let result = wonton_protocol::PluginAudience {
            role_ids: roles
                .iter()
                .filter(|role| role.permissions.open_apps.allows_plugin(plugin_id))
                .map(|role| role.id.clone())
                .collect(),
        };
        self.commit(&mut stored, roles)?;
        Ok(result)
    }

    fn commit(&self, stored: &mut Stored, roles: Vec<RoleRecord>) -> Result<(), RolesError> {
        let previous = std::mem::replace(&mut stored.roles, roles);
        let state = stored.clone();
        if let Err(e) = self.save(&state) {
            stored.roles = previous;
            return Err(e);
        }
        Ok(())
    }

    fn save(&self, state: &Stored) -> Result<(), RolesError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let io = |source| RolesError::Io {
            path: path.clone(),
            source,
        };
        let text = serde_json::to_string_pretty(state).map_err(|e| RolesError::Corrupt {
            path: path.clone(),
            message: e.to_string(),
        })?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io)?;
        }
        let tmp = path.with_extension("json.tmp");
        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)
                .map_err(io)?;
            file.write_all(text.as_bytes()).map_err(io)?;
            file.sync_all().map_err(io)?;
        }
        fs::rename(&tmp, path).map_err(io)?;
        fs::File::open(path)
            .and_then(|file| file.set_permissions(fs::Permissions::from_mode(0o600)))
            .map_err(io)?;
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, Stored> {
        self.state.lock().expect("角色记录锁中毒")
    }
}

fn check_audience(
    roles: &[RoleRecord],
    audience: &wonton_protocol::PluginAudience,
) -> Result<(), RolesError> {
    for id in &audience.role_ids {
        let role = roles
            .iter()
            .find(|role| &role.id == id)
            .ok_or_else(|| RolesError::NotFound(id.clone()))?;
        if role.permissions.open_apps == AppScope::Other {
            return Err(RolesError::Invalid(format!("角色 {id} 的应用范围不受支持")));
        }
    }
    Ok(())
}

/// 名字去掉首尾空白后按资料的规则查：不能空、不超过 64 个字、不带控制字符。
fn checked_name(name: &str) -> Result<String, RolesError> {
    let name = name.trim();
    check_display_name(name).map_err(|problem| RolesError::Invalid(problem.to_string()))?;
    Ok(name.to_owned())
}

/// 读回来的记录自检：ID 不重复、默认角色在、个数没超上限。坏在这里就别启动，免得带病运行。
fn validate(path: &std::path::Path, stored: &Stored) -> Result<(), RolesError> {
    let corrupt = |message: String| RolesError::Corrupt {
        path: path.to_owned(),
        message,
    };
    if stored.roles.len() > MAX_ROLES {
        return Err(corrupt(format!("角色超过 {MAX_ROLES} 个")));
    }
    for (index, role) in stored.roles.iter().enumerate() {
        if role.id.is_empty() {
            return Err(corrupt("有角色没有 ID".to_owned()));
        }
        if stored.roles[index + 1..].iter().any(|r| r.id == role.id) {
            return Err(corrupt(format!("角色 ID 重复：{}", role.id)));
        }
    }
    if !stored
        .roles
        .iter()
        .any(|r| r.id == stored.default_invite_role)
    {
        return Err(corrupt(format!(
            "默认邀请角色 {} 不在记录里",
            stored.default_invite_role
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(name: &str) -> RoleDraft {
        RoleDraft {
            name: name.to_owned(),
            permissions: Permissions::strictest(),
        }
    }

    #[test]
    fn a_fresh_store_has_the_four_templates_under_the_least_permissive_default() {
        let dir = tempfile::tempdir().unwrap();
        let roles = Roles::open(dir.path().join("roles.json")).unwrap();
        let all = roles.list();
        let templates: Vec<_> = all.iter().map(|r| r.template).collect();
        assert_eq!(
            templates,
            vec![
                Some(RoleTemplate::Admin),
                Some(RoleTemplate::Family),
                Some(RoleTemplate::Child),
                Some(RoleTemplate::Friend),
            ]
        );
        assert!(all.iter().all(|r| r.name.is_empty()), "模板名由壳本地化");
        assert_eq!(roles.default_invite_role(), DEFAULT_INVITE_ROLE);
        // 模板的取值照 ADR-0088 第 3 节的表。
        assert_eq!(all[0].permissions, Permissions::owner());
        assert_eq!(all[1].permissions.install_apps, Gate::AskAdmin);
        assert_eq!(all[2].permissions.ai, AiLevel::AdminApprovesDeletes);
        assert_eq!(all[3].permissions.install_apps, Gate::Deny);
    }

    #[test]
    fn created_roles_get_the_smallest_free_id_and_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roles.json");
        let roles = Roles::open(&path).unwrap();
        let first = roles.create(&draft(" 客人 ")).unwrap();
        let second = roles.create(&draft("邻居")).unwrap();
        assert_eq!((first.id.as_str(), second.id.as_str()), ("role1", "role2"));
        assert_eq!(first.name, "客人", "名字去掉首尾空白");

        let reopened = Roles::open(&path).unwrap();
        assert_eq!(reopened.get("role1").unwrap().name, "客人");
        assert_eq!(reopened.get("role2").unwrap().name, "邻居");
    }

    #[test]
    fn names_must_be_non_empty_unique_and_short() {
        let dir = tempfile::tempdir().unwrap();
        let roles = Roles::open(dir.path().join("roles.json")).unwrap();
        roles.create(&draft("客人")).unwrap();
        assert!(matches!(
            roles.create(&draft("   ")),
            Err(RolesError::Invalid(_))
        ));
        assert!(matches!(
            roles.create(&draft("客人")),
            Err(RolesError::DuplicateName(_))
        ));
        let long = "长".repeat(65);
        assert!(matches!(
            roles.create(&draft(&long)),
            Err(RolesError::Invalid(_))
        ));
        assert_eq!(
            roles.create(&draft(&"长".repeat(64))).unwrap().name.len(),
            64 * 3
        );
    }

    #[test]
    fn the_default_role_and_roles_in_use_cannot_be_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let roles = Roles::open(dir.path().join("roles.json")).unwrap();
        assert!(matches!(
            roles.delete(DEFAULT_INVITE_ROLE, 0),
            Err(RolesError::DefaultRole)
        ));
        let spare = roles.create(&draft("客人")).unwrap();
        assert!(matches!(
            roles.delete(&spare.id, 2),
            Err(RolesError::InUse(_))
        ));
        roles.delete(&spare.id, 0).unwrap();
        assert!(roles.get(&spare.id).is_none());
        assert!(matches!(
            roles.delete("role99", 0),
            Err(RolesError::NotFound(_))
        ));
    }

    #[test]
    fn a_broken_file_stops_startup_and_unknown_values_read_as_the_strictest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roles.json");
        fs::write(
            &path,
            "{\"roles\": [], \"default_invite_role\": \"nobody\"}",
        )
        .unwrap();
        assert!(matches!(
            Roles::open(&path),
            Err(RolesError::Corrupt { .. })
        ));

        let mut stored = serde_json::to_value(seeded()).unwrap();
        stored["roles"][1]["permissions"]["install_apps"] = serde_json::json!("ask_parent");
        stored["roles"][1]["permissions"]["watch_tv"] = serde_json::json!(true);
        fs::write(&path, stored.to_string()).unwrap();
        let roles = Roles::open(&path).unwrap();
        assert_eq!(
            roles.get("family").unwrap().permissions.install_apps,
            Gate::Other
        );
        assert_eq!(
            roles.get("family").unwrap().permissions.add_friends,
            Gate::Allow
        );
    }

    #[test]
    fn an_update_keeps_the_id_and_the_template_mark() {
        let dir = tempfile::tempdir().unwrap();
        let roles = Roles::open(dir.path().join("roles.json")).unwrap();
        let record = roles.update("child", &draft("小朋友")).unwrap();
        assert_eq!(record.id, "child");
        assert_eq!(record.template, Some(RoleTemplate::Child));
        assert_eq!(record.permissions.install_apps, Gate::Deny);
        assert_eq!(record.permissions, Permissions::strictest());
        assert!(matches!(
            roles.update("nobody", &draft("X")),
            Err(RolesError::NotFound(_))
        ));
    }

    #[test]
    fn the_default_invite_role_can_be_changed() {
        let dir = tempfile::tempdir().unwrap();
        let roles = Roles::open(dir.path().join("roles.json")).unwrap();
        roles.set_default("child").unwrap();
        assert_eq!(roles.default_invite_role(), "child");
        assert!(matches!(
            roles.set_default("nobody"),
            Err(RolesError::NotFound(_))
        ));
        assert_eq!(roles.delete("friend", 0).map_err(|e| e.to_string()), Ok(()));
    }
}
