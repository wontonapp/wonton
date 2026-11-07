//! 插件包的默认状态头像只补缺失字段；空表也表示主人已作选择（ADR-0044）。

use wonton_protocol::{AvatarMedia, AvatarMediaProblem, BotActivity, StateAvatars};

use super::manager::{InstallError, PluginManager};
use super::manifest::{Manifest, ManifestError};
use super::package::PluginPackage;
use super::package_files;
use crate::l10n;
use crate::matrix::{ProvisionedBot, Session};

pub(super) fn validate_manifest(manifest: &Manifest) -> Result<(), ManifestError> {
    if let Some(avatars) = manifest.bot.as_ref().and_then(|bot| bot.avatars.as_ref()) {
        for state in avatars.keys() {
            if BotActivity::from_key(state).is_none() {
                return Err(ManifestError::Invalid(format!(
                    "bot.avatars 的状态 `{state}` 不受支持"
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn poster_path(path: &str) -> String {
    std::path::Path::new(path)
        .with_extension("jpg")
        .to_string_lossy()
        .into_owned()
}

struct Checked<'a> {
    state: &'a str,
    filename: String,
    data: &'a [u8],
    media: AvatarMedia,
    poster: Option<(String, &'a [u8], AvatarMedia)>,
}

fn inspect(path: &str, data: &[u8], still: bool) -> Result<AvatarMedia, InstallError> {
    let result = crate::bot_media::inspect(data).and_then(|media| {
        if still
            && (media.duration_ms.is_some()
                || !matches!(
                    media.mimetype.as_str(),
                    "image/png" | "image/jpeg" | "image/webp"
                ))
        {
            return Err(AvatarMediaProblem::Format);
        }
        Ok(media)
    });
    result
        .map_err(|problem| InstallError::Invalid(format!("bot.avatars 文件 `{path}`：{problem:?}")))
}

fn checked(package: &PluginPackage) -> Result<Vec<Checked<'_>>, InstallError> {
    let mut checked = Vec::new();
    if let Some(avatars) = package
        .manifest
        .bot
        .as_ref()
        .and_then(|bot| bot.avatars.as_ref())
    {
        for (state, raw) in avatars {
            let path = package_files::normalize(raw).map_err(InstallError::Invalid)?;
            let data = package.files().get(&path).ok_or_else(|| {
                InstallError::Invalid(format!("bot.avatars.{state} 指向的 `{path}` 不在插件包里"))
            })?;
            let media = inspect(&path, data, false)?;
            let poster = poster_path(&path);
            let poster = if poster != path {
                package
                    .files()
                    .get(&poster)
                    .map(|data| {
                        let media = inspect(&poster, data, true)?;
                        Ok::<_, InstallError>((poster, data.as_slice(), media))
                    })
                    .transpose()?
            } else {
                None
            };
            checked.push(Checked {
                state,
                filename: path,
                data,
                media,
                poster,
            });
        }
    }
    Ok(checked)
}

async fn upload(
    session: &Session<'_>,
    files: Vec<Checked<'_>>,
) -> Result<StateAvatars, InstallError> {
    let mut avatars = StateAvatars::default();
    for file in files {
        let mut media = file.media;
        media.url = session
            .upload_media(&media.mimetype, &file.filename, file.data.to_vec())
            .await?;
        if let Some((filename, data, info)) = file.poster {
            media.poster = Some(
                session
                    .upload_media(&info.mimetype, &filename, data.to_vec())
                    .await?,
            );
        }
        avatars.states.insert(file.state.to_owned(), media);
    }
    Ok(avatars)
}

impl PluginManager {
    pub(super) async fn initialize_default_avatars(
        &self,
        package: &PluginPackage,
        bot: &ProvisionedBot,
    ) -> Result<(), InstallError> {
        if let Err(error) = self
            .fill_default_avatars(package, &bot.account.user_id)
            .await
        {
            // provision_bot 尚未返回，安装回滚还不知道这台设备；账号记录必须留下供重试。
            return Err(
                match self.revoke_bot_device(&package.manifest.id, bot).await {
                    Ok(()) => error,
                    Err(cleanup) => {
                        let langs = self.owner_languages();
                        error.with_note(
                            &langs,
                            &l10n::text(
                                &langs,
                                "plugin-settings-note-revoke-device-failed",
                                &[("error", cleanup.into())],
                            ),
                        )
                    }
                },
            );
        }
        Ok(())
    }

    pub(super) async fn fill_default_avatars(
        &self,
        package: &PluginPackage,
        user_id: &str,
    ) -> Result<(), InstallError> {
        if package
            .manifest
            .bot
            .as_ref()
            .and_then(|bot| bot.avatars.as_ref())
            .is_none()
        {
            return Ok(());
        }
        let lock = self.profile_lock(&package.manifest.id);
        let _editing = lock.lock().await;
        self.matrix()?
            .as_bot(user_id, async |session| {
                let result = async {
                    if session.has_avatars().await? {
                        return Ok(());
                    }
                    let files = checked(package)?;
                    let avatars = upload(session, files).await?;
                    session.set_avatars(&avatars).await?;
                    self.profile_revisions().bump(package.manifest.id.as_str());
                    Ok::<_, InstallError>(())
                }
                .await;
                Ok(result)
            })
            .await??;
        Ok(())
    }
}
