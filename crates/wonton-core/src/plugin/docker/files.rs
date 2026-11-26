//! 经归档接口读写容器里的文件，以及解析归档里的单个普通文件。

use bollard::query_parameters::{
    DownloadFromContainerOptionsBuilder, UploadToContainerOptionsBuilder,
};
use futures_util::StreamExt;

use super::{ARCHIVE_SLACK, ContainerFile, Engine, EngineError, Result, from_bollard};
use crate::l10n;

impl Engine {
    /// 把 tar 归档解到容器里的 `path` 下，容器创建后还没启动时也可以。属主和权限按归档里的记录；
    /// 要把已有的目录换成文件（或反过来）时报错，不覆盖。
    pub async fn upload_archive(
        &self,
        container: &str,
        path: &str,
        archive: Vec<u8>,
    ) -> Result<()> {
        let options = UploadToContainerOptionsBuilder::default()
            .path(path)
            .no_overwrite_dir_non_dir("true")
            .build();
        self.docker
            .upload_to_container(container, Some(options), bollard::body_full(archive.into()))
            .await
            .map_err(|e| {
                self.fail(
                    "plugin-install-docker-context-upload-archive",
                    &[("container", container.into())],
                    e,
                )
            })
    }

    /// 经归档接口读容器里的一个普通文件，最多 `max` 字节。路径由 Docker 在容器里解析，最后一级是软链接时
    /// 得到的是链接本身，不跟随。容器停着也能读；容器不存在时报错，文件不存在时返回 [`ContainerFile::Missing`]。
    pub async fn read_file(
        &self,
        container: &str,
        path: &str,
        max: usize,
    ) -> Result<ContainerFile> {
        let langs = self.owner_languages();
        if self.inspect_container(container).await?.is_none() {
            let missing = l10n::text(
                &langs,
                "plugin-install-docker-context-container-missing",
                &[("container", container.into())],
            );
            return Err(EngineError::NotFound(missing));
        }
        let context = l10n::text(
            &langs,
            "plugin-install-docker-context-read-file",
            &[("container", container.into()), ("path", path.into())],
        );
        let options = DownloadFromContainerOptionsBuilder::default()
            .path(path)
            .build();
        let mut stream = self
            .docker
            .download_from_container(container, Some(options));
        let mut archive = Vec::new();
        while let Some(chunk) = stream.next().await {
            match chunk.map_err(|e| from_bollard(&langs, &context, e)) {
                Ok(bytes) => archive.extend_from_slice(&bytes),
                Err(EngineError::NotFound(_)) => return Ok(ContainerFile::Missing),
                Err(other) => return Err(other),
            }
            if archive.len() > max + ARCHIVE_SLACK {
                break;
            }
        }
        single_file(&archive, max).map_err(|e| EngineError::Rejected {
            context,
            message: l10n::text(
                &langs,
                "plugin-install-docker-archive-unparseable",
                &[("error", e.to_string().into())],
            ),
        })
    }
}

/// 归档接口返回的 tar 里的第一项：只接受不超过 `max` 字节的普通文件。`archive` 可能在内容中间被截断
/// （读到上限就停了），所以先看头部再读内容。
pub(super) fn single_file(archive: &[u8], max: usize) -> std::io::Result<ContainerFile> {
    use std::io::Read;

    let mut archive = tar::Archive::new(archive);
    let Some(entry) = archive.entries()?.next() else {
        return Ok(ContainerFile::Missing);
    };
    let mut entry = entry?;
    if entry.header().entry_type() != tar::EntryType::Regular {
        return Ok(ContainerFile::NotRegular);
    }
    if entry.size() > max as u64 {
        return Ok(ContainerFile::TooBig);
    }
    let mut data = Vec::new();
    entry.read_to_end(&mut data)?;
    Ok(ContainerFile::Regular(data))
}
