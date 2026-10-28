//! 端到端加密：本设备的验证状态，以及恢复（密钥备份加秘密存储）。
//!
//! 设备密钥、交叉签名的私钥和备份密钥都在 SQLite 存储里，用同一个目录恢复会话后照样可用。恢复密钥能在
//! 别的设备上取回交叉签名的秘密和密钥备份，让那台设备也被验证、能解密历史消息。

use futures_util::{Stream, StreamExt, future, stream};
use matrix_sdk::encryption::{
    VerificationState,
    recovery::{RecoveryError, RecoveryState},
    secret_storage::SecretStorageError,
};

use crate::{Error, MatrixSession, Result};

/// 加密状态，供 ViewModel 展示。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncryptionState {
    pub verification: Verification,
    pub recovery: Recovery,
}

/// 本设备有没有被自己账号的交叉签名身份验证过。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verification {
    /// 还不知道，比如还没同步到账号的交叉签名身份。
    Unknown,
    Verified,
    Unverified,
}

/// 恢复的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// 还不知道，比如还没同步到账号数据。
    Unknown,
    /// 账号还没设置恢复。
    Disabled,
    /// 已设置，本设备也拿到了全部秘密。
    Enabled,
    /// 账号设置过恢复，但本设备还缺秘密，要输入恢复密钥。
    Incomplete,
}

impl From<VerificationState> for Verification {
    fn from(state: VerificationState) -> Self {
        match state {
            VerificationState::Unknown => Self::Unknown,
            VerificationState::Verified => Self::Verified,
            VerificationState::Unverified => Self::Unverified,
        }
    }
}

impl From<RecoveryState> for Recovery {
    fn from(state: RecoveryState) -> Self {
        match state {
            RecoveryState::Unknown => Self::Unknown,
            RecoveryState::Enabled => Self::Enabled,
            RecoveryState::Disabled => Self::Disabled,
            RecoveryState::Incomplete => Self::Incomplete,
        }
    }
}

impl MatrixSession {
    /// 加密状态流：第一项是当前状态，之后验证状态或恢复状态变化时给出新状态。
    pub fn encryption_states(&self) -> impl Stream<Item = EncryptionState> + use<> {
        let encryption = self.client.encryption();
        let verification = encryption.verification_state();
        let recovery = encryption.recovery();
        let current = EncryptionState {
            verification: verification.get().into(),
            recovery: recovery.state().into(),
        };
        enum Change {
            Verification(VerificationState),
            Recovery(RecoveryState),
        }
        let changes = stream::select(
            verification.map(Change::Verification),
            recovery.state_stream().map(Change::Recovery),
        );
        let updates = changes.scan(current, |state, change| {
            match change {
                Change::Verification(verification) => state.verification = verification.into(),
                Change::Recovery(recovery) => state.recovery = recovery.into(),
            }
            future::ready(Some(*state))
        });
        stream::once(future::ready(current)).chain(updates)
    }

    /// 设置恢复：账号还没有交叉签名身份时先建，再建密钥备份和秘密存储，返回恢复密钥。
    ///
    /// 恢复密钥只在这里给出一次，调用方要存好，主人在别的设备上恢复时要用。账号已经设置过恢复时返回
    /// [`Error::NotReady`]：再设置一次会换掉秘密存储的密钥，主人手里的旧恢复密钥就作废了。
    pub async fn enable_recovery(&self) -> Result<String> {
        let recovery = self.client.encryption().recovery();
        if matches!(
            recovery.state(),
            RecoveryState::Enabled | RecoveryState::Incomplete
        ) {
            return Err(Error::NotReady(
                "账号已经设置过恢复，请用已有的恢复密钥".to_owned(),
            ));
        }
        recovery
            .enable()
            .wait_for_backups_to_upload()
            .await
            .map_err(|error| recovery_error(&error))
    }

    /// 用恢复密钥恢复：取回交叉签名的秘密和密钥备份，本设备随之被验证。
    pub async fn recover(&self, recovery_key: &str) -> Result<()> {
        self.client
            .encryption()
            .recovery()
            .recover(recovery_key.trim())
            .await
            .map_err(|error| recovery_error(&error))
    }
}

fn recovery_error(error: &RecoveryError) -> Error {
    match error {
        RecoveryError::Sdk(sdk) => sdk.into(),
        RecoveryError::BackupExistsOnServer => {
            Error::NotReady("服务器上已经有密钥备份，请用已有的恢复密钥恢复".to_owned())
        }
        RecoveryError::SecretStorage(storage) => secret_storage_error(storage),
    }
}

/// 秘密存储的错误按原因分：隧道断开时输入了正确的恢复密钥，不能说成密钥不对。
fn secret_storage_error(error: &SecretStorageError) -> Error {
    match error {
        // 格式不对或者 MAC 校验失败，都是恢复密钥不对。
        SecretStorageError::SecretStorageKey(_) => {
            Error::InvalidInput(format!("恢复密钥不对：{error}"))
        }
        SecretStorageError::Sdk(sdk) => sdk.into(),
        SecretStorageError::MissingKeyInfo { .. } => {
            Error::NotReady(format!("账号还没有设置恢复：{error}"))
        }
        _ => Error::Upstream(error.to_string()),
    }
}
