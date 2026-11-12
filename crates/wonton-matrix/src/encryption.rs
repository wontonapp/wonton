//! 端到端加密：本设备的验证状态，以及恢复（密钥备份加秘密存储）。
//!
//! 设备密钥、交叉签名的私钥和备份密钥都在 SQLite 存储里，用同一个目录恢复会话后照样可用。恢复密钥能在
//! 别的设备上取回交叉签名的秘密和密钥备份，让那台设备也被验证、能解密历史消息。

use futures_util::{Stream, StreamExt, future, stream};
use matrix_sdk::encryption::{
    CrossSigningResetAuthType, VerificationState,
    recovery::{RecoveryError, RecoveryState},
    secret_storage::SecretStorageError,
};
#[cfg(feature = "testkit")]
use matrix_sdk::ruma::DeviceId;
use matrix_sdk::ruma::api::client::uiaa::AuthData;

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
    /// 现在的加密状态。
    pub fn encryption_state(&self) -> EncryptionState {
        let encryption = self.client.encryption();
        EncryptionState {
            verification: encryption.verification_state().get().into(),
            recovery: encryption.recovery().state().into(),
        }
    }

    /// 再向自己的其他设备要一次本机还缺的秘密（交叉签名私钥、备份密钥），马上发出；返回这次有没有发新请求。
    ///
    /// 自我验证完成时 SDK 已经要过一次，但请求比对方把本机记成“已验证”早到时，对方直接丢掉、不补发
    /// （matrix-sdk 0.19.1），只能再要。只排队不发的话要等下一次同步，可能是 30 秒的长轮询，所以补丁方法当场发出。
    /// 同一个秘密还有没发出的请求时不重复要。用的是 `vendor/matrix-sdk` 的补丁方法。
    pub async fn request_missing_secrets(&self) -> Result<bool> {
        self.client
            .encryption()
            .query_missing_secrets_from_other_sessions()
            .await
            .map_err(Error::from)
    }

    /// 加密状态流：第一项是当前状态，之后验证状态或恢复状态变化时给出新状态。
    pub fn encryption_states(&self) -> impl Stream<Item = EncryptionState> + use<> {
        let encryption = self.client.encryption();
        let verification = encryption.verification_state();
        let recovery = encryption.recovery();
        let current = self.encryption_state();
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
    /// [`Error::NotReady`]：再设置一次会换掉秘密存储的密钥，主人手里的旧恢复密钥就作废了。本设备未验证时也一样：
    /// 它没有交叉签名的私钥，存不进秘密存储，之后别的设备输对恢复密钥也一直不完整（matrix-sdk 0.19 实测）。
    /// “已验证”但手里没有交叉签名私钥的设备（SAS 确认过但对方给不出秘密）同样拒绝：SDK 的验证状态只看签名链，
    /// 不代表持有私钥（matrix-sdk 0.19.1 的 `VerificationState::Verified` 语义）。
    pub async fn enable_recovery(&self) -> Result<String> {
        let encryption = self.client.encryption();
        let verified = !matches!(
            encryption.verification_state().get(),
            VerificationState::Unverified
        );
        let has_keys = encryption
            .cross_signing_status()
            .await
            .is_some_and(|status| {
                status.has_master && status.has_self_signing && status.has_user_signing
            });
        if !verified || !has_keys {
            return Err(Error::NotReady(
                "这台设备没有账号的身份密钥，要到最早配对的那台设备上开启".to_owned(),
            ));
        }
        let recovery = encryption.recovery();
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
        let recovery = self.client.encryption().recovery();
        if let Err(error) = recovery.recover(recovery_key.trim()).await {
            // SDK 导入完秘密还要再读一次账号数据算恢复状态，这一步的网络错误是顶层的 `Sdk`（开秘密存储、导入秘密的错误
            // 包在 `SecretStorage` 里）。导入秘密触发的 keys/query 往往已先把状态算成 Enabled，这时秘密已经齐了，
            // 只是刷新那一下断在半路（matrix-sdk 0.19.1，隧道正好断开时实测）。报失败的话界面把状态退回恢复前，
            // 还留着一条“连不上”的错误。
            let only_refresh_failed = matches!(error, RecoveryError::Sdk(_))
                && matches!(recovery.state(), RecoveryState::Enabled);
            if !only_refresh_failed {
                return Err(recovery_error(&error));
            }
            tracing::info!("恢复已完成，之后刷新恢复状态失败：{error}");
        }
        // 密钥对、秘密存储却缺东西时 SDK 也算成功，这台设备其实什么也没取回。
        if matches!(recovery.state(), RecoveryState::Incomplete) {
            return Err(Error::NotReady(
                "恢复密钥是对的，但备份不完整，取不回以前的消息。可以重置备份，重置后旧消息可能读不了"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    /// 重置恢复：删掉账号的密钥备份和秘密存储，换新的交叉签名身份，本设备成为新身份下已验证的设备，再开启备份，
    /// 返回新的恢复密钥。这是残缺备份和丢了恢复密钥时唯一的出路，所以不像 [`Self::enable_recovery`] 那样挑设备。
    ///
    /// 换身份要向服务器确认本人。Tuwunel 只给登录用的 JWT 这一个阶段（v1.9 实测），`login_type` 和 `jwt` 是一次
    /// 性的登录令牌。SDK 一上来就删备份，令牌要在调用前要好。
    pub async fn reset_recovery(&self, login_type: &str, jwt: &str) -> Result<String> {
        let recovery = self.client.encryption().recovery();
        let handle = recovery
            .reset_identity()
            .await
            .map_err(|error| recovery_error(&error))?;
        if let Some(handle) = handle {
            let auth = reset_auth(handle.auth_type(), login_type, jwt)?;
            handle
                .reset(Some(auth))
                .await
                .map_err(|error| recovery_error(&error))?;
        }
        recovery
            .enable()
            .wait_for_backups_to_upload()
            .await
            .map_err(|error| recovery_error(&error))
    }

    /// 绕过 [`Self::enable_recovery`] 的检查，造出旧版本留下的残缺备份：未验证的设备开启后，秘密存储里没有交叉签名
    /// 私钥。
    #[cfg(feature = "testkit")]
    pub async fn enable_recovery_unchecked(&self) -> Result<String> {
        self.client
            .encryption()
            .recovery()
            .enable()
            .wait_for_backups_to_upload()
            .await
            .map_err(|error| recovery_error(&error))
    }

    /// 给同账号的另一台设备做本地手工签名：用本机的 self-signing 私钥签那台设备并上传签名，不通知对端、
    /// 不转秘密。对端拿到签名链就算“已验证”，手里照样没有交叉签名私钥（[`Self::enable_recovery`]
    /// 的第二道检查挡的正是这种设备）。规范里没有这种方法（ADR-0076 调研的“直接批准”），只有测试用它制造。
    #[cfg(feature = "testkit")]
    pub async fn sign_device(&self, device_id: &DeviceId) -> Result<()> {
        let device = self
            .client
            .encryption()
            .get_device(self.user_id(), device_id)
            .await
            .map_err(|error| Error::Upstream(error.to_string()))?
            .ok_or_else(|| Error::NotReady(format!("本机看不到设备 {device_id}")))?;
        device
            .verify()
            .await
            .map_err(|error| Error::Upstream(error.to_string()))
    }

    /// 本机是否已经看到同账号的另一台设备：设备密钥要经对方的 keys/query 慢一拍才到，签名前先等它。
    #[cfg(feature = "testkit")]
    pub async fn sees_device(&self, device_id: &DeviceId) -> bool {
        self.client
            .encryption()
            .get_device(self.user_id(), device_id)
            .await
            .ok()
            .flatten()
            .is_some()
    }
}

fn reset_auth(
    auth_type: &CrossSigningResetAuthType,
    login_type: &str,
    jwt: &str,
) -> Result<AuthData> {
    let unsupported = || Error::Upstream("服务器要求用别的方式确认身份，重置不了".to_owned());
    let CrossSigningResetAuthType::Uiaa(info) = auth_type else {
        return Err(unsupported());
    };
    let offered = info
        .flows
        .iter()
        .any(|flow| flow.stages.iter().any(|stage| stage.as_str() == login_type));
    if !offered {
        tracing::warn!(flows = ?info.flows, "服务器确认身份不接受登录令牌");
        return Err(unsupported());
    }
    let mut data = serde_json::Map::new();
    data.insert("token".to_owned(), jwt.into());
    AuthData::new(login_type, info.session.clone(), data)
        .map_err(|error| Error::Upstream(error.to_string()))
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
            tracing::info!("恢复密钥校验失败：{error}");
            Error::InvalidInput("恢复密钥不对，检查后再输一次".to_owned())
        }
        SecretStorageError::Sdk(sdk) => sdk.into(),
        SecretStorageError::MissingKeyInfo { .. } => {
            Error::NotReady(format!("账号还没有设置恢复：{error}"))
        }
        _ => Error::Upstream(error.to_string()),
    }
}
