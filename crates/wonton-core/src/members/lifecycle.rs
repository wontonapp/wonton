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

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PrivateBotSnapshot {
    pub(crate) status: MemberStatus,
    pub(crate) user_id: String,
    pub(crate) role_id: String,
    pub(crate) wants_private_bot: bool,
    pub(crate) bot: Option<BotRef>,
    pub(crate) reservation: Option<BotRef>,
    pub(crate) removing: bool,
}

impl Members {
    pub(crate) fn lifecycle(&self, username: &str) -> Lifecycle {
        self.lock()
            .lifecycle
            .get(username)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn private_bot_snapshot(&self, username: &str) -> Option<PrivateBotSnapshot> {
        let stored = self.lock();
        let member = stored
            .members
            .iter()
            .find(|member| member.username == username)?;
        let lifecycle = stored.lifecycle.get(username);
        Some(PrivateBotSnapshot {
            status: member.status,
            user_id: member.user_id.clone(),
            role_id: member.role_id.clone(),
            wants_private_bot: member.wants_private_bot,
            bot: member.private_bot.clone(),
            reservation: lifecycle.and_then(|state| state.bot.clone()),
            removing: lifecycle.is_some_and(|state| state.removal.is_some()),
        })
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
