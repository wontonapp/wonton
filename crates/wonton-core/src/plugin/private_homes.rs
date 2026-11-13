//! Private spaces are resolved from live keeper permissions, never from the bot's name.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bollard::models::{Mount, MountType};
use wonton_protocol::{AiLevel, BotKeeper, MemberStatus};

use super::hostdir::Owner;
use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::manifest::{Access, Manifest, Permission};
use super::render::{InstallContext, RenderError, service_err};

pub const HOME_TARGET: &str = "/wonton/home";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeMount {
    pub path: PathBuf,
    pub read_only: bool,
}

impl PluginManager {
    pub fn attach_person_roles(&self, roles: Arc<crate::roles::Roles>) {
        *self
            .person_roles
            .write()
            .expect("person roles lock poisoned") = roles;
    }

    pub fn attach_person_state(
        &self,
        members: Arc<crate::members::Members>,
        roles: Arc<crate::roles::Roles>,
    ) {
        self.attach_person_members(members);
        self.attach_person_roles(roles);
    }

    pub fn private_home_owner(&self) -> Owner {
        self.owner()
    }

    pub(super) fn keeper_home(
        &self,
        keeper: &BotKeeper,
        write: bool,
    ) -> Result<HomeMount, InstallError> {
        let BotKeeper::User { username } = keeper else {
            return Err(self.home_denied());
        };
        let ai_read_only = if username == &self.cfg.owner {
            false
        } else {
            let member = self
                .person_members
                .get()
                .and_then(|members| members.get(username))
                .filter(|member| member.status == MemberStatus::Joined)
                .ok_or_else(|| self.home_denied())?;
            let role = self
                .person_roles
                .read()
                .expect("person roles lock poisoned")
                .get(&member.role_id)
                .filter(|role| role.permissions.private_bot)
                .ok_or_else(|| self.home_denied())?;
            matches!(role.permissions.ai, AiLevel::ReadOnly | AiLevel::Other)
        };
        let path = crate::private_home::ensure(&self.cfg.data_dir, username, self.owner())
            .map_err(InstallError::Storage)?;
        Ok(HomeMount {
            path,
            read_only: !write || ai_read_only,
        })
    }

    pub(super) fn home_denied(&self) -> InstallError {
        InstallError::PermissionNotGranted(crate::l10n::text(
            &self.owner_languages(),
            "plugin-settings-permission-not-granted",
            &[("permissions", "storage.home".into())],
        ))
    }

    pub(super) fn private_home_mounts(
        &self,
        manifest: &Manifest,
        granted: &[Permission],
    ) -> Result<BTreeMap<String, HomeMount>, InstallError> {
        let Some(write) = home_access(granted) else {
            return Ok(BTreeMap::new());
        };
        if manifest.bots.is_none() {
            return Err(self.home_denied());
        }
        let mut homes = BTreeMap::new();
        let entries = self.store.bots().load(&manifest.id)?;
        let initial_install = !entries.bots.contains_key(super::bot_services::DEFAULT_BOT);
        for (bot, entry) in entries.bots {
            match self.keeper_home(&entry.keeper, write) {
                Ok(home) => {
                    homes.insert(bot, home);
                }
                Err(InstallError::PermissionNotGranted(_)) => {}
                Err(error) => return Err(error),
            }
        }
        if initial_install {
            homes.insert(
                super::bot_services::DEFAULT_BOT.to_owned(),
                self.keeper_home(
                    &BotKeeper::User {
                        username: self.cfg.owner.clone(),
                    },
                    write,
                )?,
            );
        }
        Ok(homes)
    }
}

pub(super) fn home_access(permissions: &[Permission]) -> Option<bool> {
    permissions
        .iter()
        .filter_map(|permission| match permission {
            Permission::StorageHome { access } => Some(*access == Access::Write),
            _ => None,
        })
        .reduce(|a, b| a || b)
}

pub(super) fn dry_run_homes(manifest: &Manifest) -> BTreeMap<String, HomeMount> {
    home_access(&manifest.permissions)
        .map(|write| {
            BTreeMap::from([(
                super::bot_services::DEFAULT_BOT.to_owned(),
                HomeMount {
                    path: PathBuf::from("/wonton-dry-run/core/home/owner"),
                    read_only: !write,
                },
            )])
        })
        .unwrap_or_default()
}

pub(super) fn mount_home(
    manifest: &Manifest,
    ctx: &InstallContext,
    bot: Option<&str>,
    mounts: &mut Vec<Mount>,
    service: &super::compose::Service,
    name: &str,
    langs: &Languages,
) -> Result<(), RenderError> {
    if home_access(&ctx.granted).is_none() {
        return Ok(());
    }
    let denied = || {
        service_err(
            langs,
            name,
            crate::l10n::text(
                langs,
                "plugin-settings-permission-not-granted",
                &[("permissions", "storage.home".into())],
            ),
        )
    };
    if manifest.bots.is_none() {
        return Err(denied());
    }
    let Some(bot) = bot else {
        return Ok(());
    };
    let home = ctx.homes.get(bot).ok_or_else(denied)?;
    let target = Path::new(HOME_TARGET);
    if super::mounts::mount_targets(service, mounts)
        .iter()
        .any(|mount| {
            mount
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
                || target.starts_with(mount)
                || mount.starts_with(target)
        })
    {
        return Err(denied());
    }
    mounts.push(Mount {
        target: Some(HOME_TARGET.to_owned()),
        source: Some(home.path.display().to_string()),
        typ: Some(MountType::BIND),
        read_only: Some(home.read_only),
        ..Default::default()
    });
    Ok(())
}
