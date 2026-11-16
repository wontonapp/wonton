//! Pending account reservations and removal tombstones are not active Bot identities.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use wonton_protocol::BotKeeper;

use super::bots::{BotEntry, BotStore, PluginBots};
use super::manifest::PluginId;
use super::store::{StoreError, create_private_dir, read_json, remove_file, write_json};
use crate::matrix::{BotAccount, ProvisionedBot};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct PendingBot {
    pub user_id: String,
    pub home_room: Option<String>,
    pub device_id: Option<String>,
    pub keeper: BotKeeper,
    pub bus_token: String,
}

impl PendingBot {
    pub fn account(&self) -> BotAccount {
        BotAccount {
            user_id: self.user_id.clone(),
            home_room: self.home_room.clone().unwrap_or_default(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct RetiredBot {
    pub account: BotAccount,
    pub keeper: BotKeeper,
}

impl BotStore {
    pub(super) fn pending(
        &self,
        id: &PluginId,
    ) -> Result<BTreeMap<String, PendingBot>, StoreError> {
        Ok(read_json(&self.dir.join("pending").join(format!("{id}.json")))?.unwrap_or_default())
    }

    pub(super) fn save_pending(
        &self,
        id: &PluginId,
        records: &BTreeMap<String, PendingBot>,
    ) -> Result<(), StoreError> {
        let dir = self.dir.join("pending");
        create_private_dir(&dir)?;
        write_json(&dir.join(format!("{id}.json")), records)
    }

    pub(super) fn forget_pending(&self, id: &PluginId, bot: &str) -> Result<(), StoreError> {
        let mut records = self.pending(id)?;
        records.remove(bot);
        self.save_pending(id, &records)
    }

    pub(super) fn publish_pending(
        &self,
        id: &PluginId,
        bot: &str,
        provisioned: &ProvisionedBot,
        intent: PendingBot,
        plugin_bots: &mut PluginBots,
    ) -> Result<(), StoreError> {
        plugin_bots.bots.insert(
            bot.to_owned(),
            BotEntry {
                account: provisioned.account.clone(),
                device_id: Some(provisioned.device_id.clone()),
                keeper: intent.keeper,
                bus_token: intent.bus_token,
            },
        );
        self.save(id, plugin_bots)?;
        self.forget_pending(id, bot)
    }

    pub(super) fn retired(
        &self,
        id: &PluginId,
        bot: &str,
    ) -> Result<Option<RetiredBot>, StoreError> {
        read_json(
            &self
                .dir
                .join("retired")
                .join(id.as_str())
                .join(format!("{bot}.json")),
        )
    }

    pub(super) fn retire(
        &self,
        id: &PluginId,
        bot: &str,
        entry: &BotEntry,
    ) -> Result<(), StoreError> {
        let dir = self.dir.join("retired").join(id.as_str());
        create_private_dir(&dir)?;
        write_json(
            &dir.join(format!("{bot}.json")),
            &RetiredBot {
                account: entry.account.clone(),
                keeper: entry.keeper.clone(),
            },
        )
    }

    pub(super) fn forget_retired(&self, id: &PluginId, bot: &str) -> Result<(), StoreError> {
        remove_file(
            &self
                .dir
                .join("retired")
                .join(id.as_str())
                .join(format!("{bot}.json")),
        )
    }
}

impl BotStore {
    pub(super) fn removal_entry(
        &self,
        id: &PluginId,
        bot: &str,
    ) -> Result<Option<BotEntry>, StoreError> {
        if let Some(entry) = self.entry(id, bot)? {
            return Ok(Some(entry));
        }
        if let Some(intent) = self.pending(id)?.remove(bot) {
            return Ok(Some(BotEntry {
                account: intent.account(),
                device_id: intent.device_id,
                keeper: intent.keeper,
                bus_token: intent.bus_token,
            }));
        }
        Ok(self.retired(id, bot)?.map(|entry| BotEntry {
            account: entry.account,
            keeper: entry.keeper,
            device_id: None,
            bus_token: String::new(),
        }))
    }
}

impl BotStore {
    pub(super) fn pending_localparts(&self) -> Result<Vec<String>, StoreError> {
        let dir = self.dir.join("pending");
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(StoreError::Io { path: dir, source }),
        };
        let mut names = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|source| StoreError::Io {
                    path: dir.clone(),
                    source,
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let intents: BTreeMap<String, PendingBot> = read_json(&path)?.unwrap_or_default();
            for intent in intents.into_values() {
                if let Some((name, _)) = intent
                    .user_id
                    .strip_prefix('@')
                    .and_then(|user| user.split_once(':'))
                {
                    names.push(name.to_owned());
                }
            }
        }
        Ok(names)
    }
}
