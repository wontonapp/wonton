#[cfg(test)]
#[path = "people_tests.rs"]
mod people_tests;

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use iroh::EndpointId;
use wonton_protocol::{FileFailure, MemberStatus};

use super::{Accessor, CoreHost, Failure, FileHost, NoticeRoom, notice_failure};
use crate::{device::Devices, matrix::Matrix, members::Members, roles::Roles};

pub(super) struct People {
    members: Arc<Members>,
    roles: Arc<Roles>,
    devices: Arc<Devices>,
    owner: String,
    matrix: Option<Arc<Matrix>>,
    state: PathBuf,
    notices: Mutex<HashMap<String, Arc<NoticeRoom>>>,
}

fn denied() -> Failure {
    Failure::new(
        FileFailure::NotShared,
        crate::l10n::text(
            &crate::plugin::languages::Languages::default(),
            "files-not-shared",
            &[],
        ),
    )
}

impl CoreHost {
    pub fn with_people(
        mut self,
        members: Arc<Members>,
        roles: Arc<Roles>,
        devices: Arc<Devices>,
        owner: String,
        matrix: Option<Arc<Matrix>>,
        state: PathBuf,
    ) -> Self {
        self.people = Some(People {
            members,
            roles,
            devices,
            owner,
            matrix,
            state,
            notices: Mutex::new(HashMap::new()),
        });
        self
    }

    fn people(&self) -> Result<&People, Failure> {
        self.people.as_ref().ok_or_else(|| {
            Failure::new(
                FileFailure::ServiceUnavailable,
                crate::l10n::text(
                    &self.plugins.owner_languages(),
                    "files-service-disabled",
                    &[],
                ),
            )
        })
    }

    pub(super) fn check_file_person(&self, who: &Accessor) -> Result<(), Failure> {
        if let Accessor::Bot(bot) = who {
            self.bot_file_person(bot)?;
            return Ok(());
        }
        let Accessor::User(username) = who else {
            return Ok(());
        };
        let people = self.people()?;
        let record = people.members.get(username).ok_or_else(denied)?;
        if record.status != MemberStatus::Joined
            || !people
                .roles
                .get(&record.role_id)
                .is_some_and(|role| role.permissions.private_bot)
        {
            return Err(denied());
        }
        Ok(())
    }

    /// 向 Matrix 核对 `who` 是不是这些房间中某一间的当前成员。核心自己不在的房间（不是它
    /// 管的家庭群）查不到成员，跳过；其它查询失败都是查不到，不是不在。后面的房间已经有答案时
    /// 以答案为准，只有一间都没命中、又至少失败过一次才报错。
    pub(super) fn member_of_file_rooms<'a>(
        &'a self,
        who: &'a Accessor,
        rooms: &'a BTreeSet<String>,
    ) -> BoxFuture<'a, Result<bool, Failure>> {
        async move {
            let people = self.people()?;
            let username = match who {
                Accessor::Owner => people.owner.clone(),
                Accessor::User(username) => username.clone(),
                Accessor::Bot(_) => return Ok(false),
            };
            let langs = self.plugins.owner_languages();
            let unknown = |error: String| {
                Failure::new(
                    FileFailure::ServiceUnavailable,
                    crate::l10n::text(
                        &langs,
                        "files-room-members-unknown",
                        &[("error", error.into())],
                    ),
                )
            };
            let matrix = people.matrix.as_ref().ok_or_else(|| {
                Failure::new(
                    FileFailure::ServiceUnavailable,
                    crate::l10n::text(&langs, "files-room-members-unconfigured", &[]),
                )
            })?;
            let wanted = matrix.user_id(&username);
            let core = matrix.core_session();
            let mut failed = None;
            for room in rooms {
                match core.joined_members(room).await {
                    Ok(members) if members.iter().any(|id| id == &wanted) => return Ok(true),
                    Ok(_) => {}
                    Err(e) if matches!(e.errcode(), "M_FORBIDDEN" | "M_NOT_FOUND") => {}
                    Err(e) => {
                        failed.get_or_insert_with(|| e.to_string());
                    }
                }
            }
            match failed {
                Some(error) => Err(unknown(error)),
                None => Ok(false),
            }
        }
        .boxed()
    }

    pub(super) fn upload_file_person(&self, device: &str) -> Result<(String, Accessor), Failure> {
        let people = self.people()?;
        let id = device.parse::<EndpointId>().map_err(|_| denied())?;
        let device = people.devices.get(&id).ok_or_else(denied)?;
        let (username, who) = match device.user {
            None => (people.owner.clone(), Accessor::Owner),
            Some(username) => {
                if people.members.pending_endpoint(id, &username) {
                    return Err(denied());
                }
                (username.clone(), Accessor::User(username))
            }
        };
        self.check_file_person(&who)?;
        Ok((username, who))
    }
    pub(super) fn bot_file_person(&self, bot: &str) -> Result<Accessor, Failure> {
        let people = self.people()?;
        let listings = self.plugins.bot_listings().map_err(Failure::internal)?;
        // Accessor::Bot stores the authenticated Matrix localpart, not the per-plugin bot key.
        // Refuse corrupt duplicate identities rather than selecting an arbitrary plugin's keeper.
        let mut matching = listings.iter().filter(|entry| {
            entry
                .user_id
                .strip_prefix('@')
                .and_then(|id| id.split_once(':'))
                .is_some_and(|(localpart, _)| localpart == bot)
        });
        let listing = matching.next().ok_or_else(denied)?;
        if matching.next().is_some() {
            return Err(denied());
        }
        let username = self.plugins.keeper_username(&listing.keeper);
        let who = if username == people.owner {
            Accessor::Owner
        } else {
            Accessor::User(username)
        };
        self.check_file_person(&who)?;
        Ok(who)
    }

    pub(super) async fn notify_file_person(
        &self,
        who: &Accessor,
        text: &str,
    ) -> Result<(), Failure> {
        self.check_file_person(who)?;
        let Accessor::User(username) = who else {
            return if matches!(who, Accessor::Owner) {
                self.notify_owner(text).await
            } else {
                Err(denied())
            };
        };
        let people = self.people()?;
        let notices = {
            let mut cache = people.notices.lock().expect("file notice lock poisoned");
            if let Some(notices) = cache.get(username) {
                notices.clone()
            } else {
                let matrix = people.matrix.clone().ok_or_else(|| {
                    Failure::new(
                        FileFailure::ServiceUnavailable,
                        crate::l10n::text(
                            &self.plugins.owner_languages(),
                            "files-notify-no-matrix",
                            &[],
                        ),
                    )
                })?;
                let plugins = self.plugins.clone();
                let person = username.clone();
                let notices = Arc::new(NoticeRoom::new(
                    matrix,
                    username,
                    people.state.join(format!("matrix-notice-room-{username}")),
                    Arc::new(move || {
                        crate::plugin::languages::Languages::from_header(
                            plugins.person_languages(&person).as_deref(),
                        )
                    }),
                ));
                cache.insert(username.clone(), notices.clone());
                notices
            }
        };
        notices
            .send(&serde_json::json!({"msgtype":"m.text", "body":text}))
            .await
            .map(|_| ())
            .map_err(|e| notice_failure(e, &self.plugins.owner_languages()))
    }
}
