//! `wonton-core manual-rollback`：手动退回时 `install.sh` 在目标镜像里执行（ADR-0025 的“手动兜底”）。那份备份就是这个
//! 镜像的升级器做的，所以它认得。运行中的核心可能已经坏了，这里只靠 Docker。

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};

use super::host::{Host, Mount, OneShot};
use super::state::Layout;
use super::{CORE_SERVICE, PROJECT, backup, components};

/// 恢复一个卷的时限：卷可能很大，但主人就在终端前。
const VOLUME_RESTORE: Duration = Duration::from_secs(3600);

pub struct ManualRollback {
    pub layout: Layout,
    /// 退回到的镜像。
    pub image: String,
    /// 有这份备份时恢复核心数据和安装目录。
    pub backup: Option<PathBuf>,
    /// 同时恢复备份里的卷。
    pub restore_volumes: bool,
}

/// 按参数解析：`<安装目录> <数据目录> <镜像> <备份目录或 -> [--volumes]`。
pub fn parse(args: &[String]) -> anyhow::Result<ManualRollback> {
    let [install, data, image, backup, rest @ ..] = args else {
        bail!(
            "用法：wonton-core manual-rollback <安装目录> <数据目录> <镜像> <备份目录或 -> [--volumes]"
        );
    };
    let restore_volumes = match rest {
        [] => false,
        [flag] if flag == "--volumes" => true,
        _ => bail!("认不出参数 {rest:?}"),
    };
    Ok(ManualRollback {
        layout: Layout {
            install_dir: PathBuf::from(install),
            data_dir: PathBuf::from(data),
        },
        image: image.clone(),
        backup: (backup != "-").then(|| PathBuf::from(backup)),
        restore_volumes,
    })
}

impl ManualRollback {
    pub async fn run(&self, host: &dyn Host) -> anyhow::Result<()> {
        // 主机终端命令，文字不迁（E 类）；里面的库函数要一个语言，按基础语言给。
        let langs = crate::plugin::languages::Languages::from_header(None);
        let containers = host.project_containers(PROJECT).await?;
        let volumes = self.backed_up_volumes()?;
        for container in containers.iter().filter(|c| {
            c.service == CORE_SERVICE
                || volumes
                    .iter()
                    .any(|v| c.volumes.contains(&format!("{PROJECT}_{v}")))
        }) {
            host.stop(&container.name).await?;
        }
        if let Some(dir) = &self.backup {
            backup::restore_files(&self.layout, dir, &langs)?;
            for volume in &volumes {
                self.restore_volume(host, dir, volume).await?;
            }
        }
        components::prepare(host, &self.image, &langs).await?;
        Ok(())
    }

    fn backed_up_volumes(&self) -> anyhow::Result<Vec<String>> {
        let Some(dir) = self.backup.as_deref().filter(|_| self.restore_volumes) else {
            return Ok(Vec::new());
        };
        let root = dir.join(backup::VOLUMES);
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).with_context(|| format!("读取 {}", root.display())),
        };
        let mut volumes = Vec::new();
        for entry in entries {
            volumes.push(entry?.file_name().to_string_lossy().into_owned());
        }
        Ok(volumes)
    }

    async fn restore_volume(
        &self,
        host: &dyn Host,
        dir: &Path,
        volume: &str,
    ) -> anyhow::Result<()> {
        let job = OneShot {
            name: "wonton-updater-volume-restore".to_owned(),
            image: self.image.clone(),
            entrypoint: Some(vec!["sh".to_owned()]),
            cmd: vec![
                "-c".to_owned(),
                "find /volume -mindepth 1 -delete && cp -a /backup/. /volume/".to_owned(),
            ],
            mounts: vec![
                Mount::Volume {
                    name: format!("{PROJECT}_{volume}"),
                    target: PathBuf::from("/volume"),
                    read_only: false,
                },
                Mount::Bind {
                    source: dir.join(backup::VOLUMES).join(volume),
                    target: PathBuf::from("/backup"),
                    read_only: true,
                },
            ],
            timeout: VOLUME_RESTORE,
        };
        let output = host.run_once(&job).await?;
        if output.exit_code != 0 {
            bail!("恢复卷 {volume} 失败：{}", output.stderr);
        }
        println!("已恢复卷 {volume}");
        Ok(())
    }
}
