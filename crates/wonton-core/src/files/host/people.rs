#[cfg(test)]
#[path = "people_tests.rs"]
mod people_tests;

use super::{Accessor, CoreHost, Failure, FileHost, NoticeRoom, notice_failure};
use crate::{device::Devices, matrix::Matrix, members::Members, roles::Roles};
use iroh::EndpointId;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use wonton_protocol::{FileFailure, MemberStatus};

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
