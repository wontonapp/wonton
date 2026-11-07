//! 引导在本机的记录：两条一次性提示看没看过、候选提问用没用过和发过消息的房间。按台记录，换服务器时
//! 提问的记录清空重来，提示的记录保留。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use wonton_l10n::l10n;
use wonton_protocol::{CoreError, OnboardingTip};

use crate::home::write_private;
use crate::l10n::{CATALOG, language};

const FILE: &str = "onboarding.json";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct Records {
    pub(crate) confirmation_card: bool,
    pub(crate) app_installed: bool,
    pub(crate) prompt_server_id: Option<String>,
    pub(crate) prompts_used: bool,
    #[serde(default)]
    pub(crate) prompt_rooms: Vec<String>,
}

pub(crate) struct OnboardingStorage {
    path: PathBuf,
    records: Mutex<Records>,
    writing: Mutex<()>,
}

impl OnboardingStorage {
    /// 读不懂记录时照常用默认值建起来，原因给调用方放进 ViewModel；写入失败时如实报 `Storage`。
    pub(crate) fn open(data_dir: &Path) -> (Self, Option<CoreError>) {
        let path = data_dir.join(FILE);
        let records = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(records) => records,
                Err(error) => {
                    let problem = storage_error(&path, error);
                    return (Self::fresh(path), Some(problem));
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => Records::default(),
            Err(error) => {
                let problem = storage_error(&path, error);
                return (Self::fresh(path), Some(problem));
            }
        };
        (
            Self {
                path,
                records: Mutex::new(records),
                writing: Mutex::new(()),
            },
            None,
        )
    }

    fn fresh(path: PathBuf) -> Self {
        Self {
            path,
            records: Mutex::new(Records::default()),
            writing: Mutex::new(()),
        }
    }

    pub(crate) fn snapshot(&self) -> Records {
        lock(&self.records).clone()
    }

    pub(crate) fn dismiss_tip(&self, tip: OnboardingTip) -> Result<(), CoreError> {
        self.change(|records| match tip {
            OnboardingTip::ConfirmationCard => records.confirmation_card = true,
            OnboardingTip::AppInstalled => records.app_installed = true,
        })
    }

    pub(crate) fn sync_server(&self, server_id: &str) -> Result<(), CoreError> {
        let _writing = lock(&self.writing);
        let mut records = lock(&self.records).clone();
        if records.prompt_server_id.as_deref() == Some(server_id) {
            return Ok(());
        }
        records.prompts_used = false;
        records.prompt_rooms.clear();
        records.prompt_server_id = Some(server_id.to_owned());
        self.write(&records)?;
        *lock(&self.records) = records;
        Ok(())
    }

    pub(crate) fn record_prompt_room(&self, room_id: &str) -> Result<(), CoreError> {
        let _writing = lock(&self.writing);
        let mut records = lock(&self.records).clone();
        if records.prompt_rooms.iter().any(|room| room == room_id) {
            return Ok(());
        }
        records.prompt_rooms.push(room_id.to_owned());
        self.write(&records)?;
        *lock(&self.records) = records;
        Ok(())
    }

    pub(crate) fn mark_prompts_used(&self) -> Result<(), CoreError> {
        let _writing = lock(&self.writing);
        let mut records = lock(&self.records).clone();
        if records.prompts_used {
            return Ok(());
        }
        records.prompts_used = true;
        self.write(&records)?;
        *lock(&self.records) = records;
        Ok(())
    }

    fn change(&self, apply: impl FnOnce(&mut Records)) -> Result<(), CoreError> {
        let _writing = lock(&self.writing);
        let mut records = lock(&self.records).clone();
        apply(&mut records);
        self.write(&records)?;
        *lock(&self.records) = records;
        Ok(())
    }

    fn write(&self, records: &Records) -> Result<(), CoreError> {
        let bytes = serde_json::to_vec(records).expect("onboarding records always serialize");
        write_private(&self.path, &bytes).map_err(|error| CoreError::Storage {
            reason: l10n!(
                CATALOG,
                language(),
                "onboarding-storage-io-failed",
                path = self.path.display().to_string(),
                reason = error.to_string()
            ),
        })
    }
}

fn storage_error<T: std::fmt::Display>(path: &Path, error: T) -> CoreError {
    CoreError::Storage {
        reason: l10n!(
            CATALOG,
            language(),
            "onboarding-storage-io-failed",
            path = path.display().to_string(),
            reason = error.to_string()
        ),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
