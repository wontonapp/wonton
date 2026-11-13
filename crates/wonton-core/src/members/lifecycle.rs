use serde::{Deserialize, Serialize};
use wonton_protocol::{BotRef, MemberRemoval, MemberStatus};

use super::{Members, MembersError, Stored};

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Lifecycle {
    #[serde(default)]
    pub bot: Option<BotRef>,
    #[serde(default)]
    pub removal: Option<MemberRemoval>,
}

impl Members {
    pub(crate) fn lifecycle(&self, username: &str) -> Lifecycle {
        self.lock()
            .lifecycle
            .get(username)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn reserve_private_bot(
        &self,
        username: &str,
        bot: &BotRef,
    ) -> Result<(), MembersError> {
        self.change_lifecycle(username, |stored| {
            let member = stored
                .members
                .iter()
                .find(|member| member.username == username)
                .unwrap();
            if member.status != MemberStatus::Joined {
                return Err(MembersError::Invalid(
                    "Private bot needs a joined member".to_owned(),
                ));
            }
            let lifecycle = stored.lifecycle.entry(username.to_owned()).or_default();
            if lifecycle
                .bot
                .as_ref()
                .is_some_and(|reserved| reserved != bot)
            {
                return Err(MembersError::Invalid(
                    "Private bot already reserved".to_owned(),
                ));
            }
            lifecycle.bot = Some(bot.clone());
            Ok(())
        })
    }

    pub(crate) fn private_bot_ready(
        &self,
        username: &str,
        bot: &BotRef,
    ) -> Result<(), MembersError> {
        self.change_lifecycle(username, |stored| {
            let member = stored
                .members
                .iter_mut()
                .find(|member| member.username == username)
                .unwrap();
            if member.status != MemberStatus::Joined {
                return Err(MembersError::Removed(username.to_owned()));
            }
            member.private_bot = Some(bot.clone());
            Ok(())
        })
    }

    pub(crate) fn removal_finished(&self, username: &str) -> Result<(), MembersError> {
        self.change_lifecycle(username, |stored| {
            let member = stored
                .members
                .iter_mut()
                .find(|member| member.username == username)
                .unwrap();
            if member.status != MemberStatus::Removed {
                return Err(MembersError::Invalid("Member is not removed".to_owned()));
            }
            member.private_bot = None;
            if let Some(lifecycle) = stored.lifecycle.get_mut(username) {
                lifecycle.removal = None;
            }
            Ok(())
        })
    }

    fn change_lifecycle(
        &self,
        username: &str,
        change: impl FnOnce(&mut Stored) -> Result<(), MembersError>,
    ) -> Result<(), MembersError> {
        let mut stored = self.lock();
        if !stored
            .members
            .iter()
            .any(|member| member.username == username)
        {
            return Err(MembersError::NotFound(username.to_owned()));
        }
        let mut next = stored.clone();
        change(&mut next)?;
        self.commit(&mut stored, next)
    }
}
