//! `read_only` 服务的 `configs` 交付：核心数据目录里的文件，以只读单文件绑定挂载给容器。
//!
//! 归档接口写不进只读根（Docker 在容器启动前后都直接拒绝），而核心不以 root 运行时自己写文件也给不了
//! 声明的属主（chown 需要 CAP_CHOWN）。所以内容不自己写：按编号命名的文件打成 tar，经归档接口解进一个
//! 一次性容器的绑定挂载里，属主和权限由 daemon（root，和普通服务把归档写进容器根是同一个组件）按归档
//! 里的记录写，实测 Docker 29.7.2 对停着的容器也解析绑定挂载。
//!
//! 文件放在核心数据目录（`<数据目录>/core/configs/<实例>/<插件>/<服务>/<编号>`），插件容器写不到那里，
//! 路径上不会有能换成软链接的目录，挂载源因此不用逐级检查（ADR-0011 的"绑定挂载"约束管不到）。
//! 归档接口覆盖同名文件时换 inode，所以内容更新只在重建容器后可见——安装和升级本来就会重建。

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use bollard::models::{ContainerCreateBody, HostConfig, Mount, MountType};

use super::configs::ConfigFile;
use super::docker::Engine;
use super::manager::InstallError;
use super::names::Instance;

/// 一次性容器里挂 store 目录的位置。
const HELPER_MOUNT: &str = "/wonton-configs";

/// 插件的配置文件目录（含它的各服务）。
pub(super) fn plugin_dir(root: &Path, instance: &Instance, slug: &str) -> PathBuf {
    let dir = match instance.name() {
        None => root.to_path_buf(),
        Some(name) => root.join(name),
    };
    dir.join(slug)
}

/// 服务的配置文件在核心数据目录里的位置。
pub(super) fn service_dir(root: &Path, instance: &Instance, slug: &str, service: &str) -> PathBuf {
    plugin_dir(root, instance, slug).join(service)
}

/// 每个配置文件一个只读绑定挂载：源是 store 目录里按编号排的文件（和 [`archive`] 的条目一一对应），
/// 目标是容器里的路径。
pub(super) fn bind_mounts(dir: &Path, files: &[ConfigFile]) -> Vec<Mount> {
    files
        .iter()
        .enumerate()
        .map(|(index, file)| Mount {
            target: Some(file.target.display().to_string()),
            source: Some(dir.join(index.to_string()).display().to_string()),
            typ: Some(MountType::BIND),
            read_only: Some(true),
            ..Default::default()
        })
        .collect()
}

/// 一次性容器的名字：插件容器名加 `.cfgseed`。服务名不含 `.`，不会重名。
pub(super) fn helper_name(container: &str) -> String {
    format!("{container}.cfgseed")
}

/// 打包成按编号命名的归档，条目顺序和 `files` 一致，就是绑定挂载源里的编号。
pub(super) fn archive(files: &[ConfigFile]) -> io::Result<Vec<u8>> {
    let mut builder = tar::Builder::new(Vec::new());
    for (index, file) in files.iter().enumerate() {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(file.data.len() as u64);
        header.set_uid(file.uid.into());
        header.set_gid(file.gid.into());
        header.set_mode(file.mode);
        header.set_mtime(super::manager::now_secs());
        builder.append_data(&mut header, index.to_string(), file.data.as_slice())?;
    }
    builder.into_inner()
}

/// 建好 store 目录，经一次性容器把归档解进去：容器从不运行，daemon 按归档里的记录写宿主文件。
/// 失败时也删掉一次性容器；删不掉的按标签由下次安装、卸载清理。
pub(super) async fn seed(
    engine: &Engine,
    container: &str,
    image: &str,
    dir: &Path,
    archive: Vec<u8>,
    labels: &HashMap<String, String>,
) -> Result<(), InstallError> {
    fs::create_dir_all(dir).map_err(|e| {
        InstallError::Storage(format!("创建配置文件目录 {} 失败：{e}", dir.display()))
    })?;
    let helper = helper_name(container);
    let body = ContainerCreateBody {
        image: Some(image.to_owned()),
        labels: Some(labels.clone()),
        host_config: Some(HostConfig {
            binds: Some(vec![format!("{}:{HELPER_MOUNT}", dir.display())]),
            cap_drop: Some(vec!["ALL".to_owned()]),
            security_opt: Some(vec!["no-new-privileges:true".to_owned()]),
            ..Default::default()
        }),
        ..Default::default()
    };
    engine.create_container(&helper, body).await?;
    let uploaded = engine.upload_archive(&helper, HELPER_MOUNT, archive).await;
    let removed = engine.remove_container(&helper).await;
    match (uploaded, removed) {
        (Ok(()), Ok(())) => Ok(()),
        // 归档写失败优先报告；容器删不掉时 daemon 里还有残留，报错让安装停下来。
        (Err(e), _) | (_, Err(e)) => Err(e.into()),
    }
}

/// 卸载插件时删掉它的 store 目录（整个插件，含各服务的）。文件可能不属于核心的用户，
/// 删除只看父目录权限，store 目录由核心创建，删得掉。
pub(super) fn remove_tree(root: &Path, instance: &Instance, slug: &str) -> io::Result<()> {
    let dir = plugin_dir(root, instance, slug);
    match fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(target: &str, uid: u32, gid: u32, mode: u32, data: &[u8]) -> ConfigFile {
        ConfigFile {
            target: PathBuf::from(target),
            uid,
            gid,
            mode,
            data: data.to_vec(),
        }
    }

    #[test]
    fn service_dirs_are_separated_by_instance_slug_and_service() {
        let root = Path::new("/data/core/configs");
        assert_eq!(
            service_dir(root, &Instance::default(), "app.test", "web"),
            Path::new("/data/core/configs/app.test/web")
        );
        let instance = Instance::parse(Some("dev")).unwrap();
        assert_eq!(
            service_dir(root, &instance, "app.test", "web"),
            Path::new("/data/core/configs/dev/app.test/web")
        );
        assert_eq!(
            service_dir(root, &instance, "app.test", "db"),
            Path::new("/data/core/configs/dev/app.test/db")
        );
    }

    #[test]
    fn helper_names_cannot_collide_with_service_containers() {
        assert_eq!(
            helper_name("wonton-app-test--web"),
            "wonton-app-test--web.cfgseed"
        );
    }

    #[test]
    fn bind_mounts_map_staged_files_to_targets_read_only() {
        let dir = Path::new("/data/core/configs/app.test/web");
        let files = [
            file("/etc/a.conf", 65532, 65532, 0o440, b"one"),
            file("/b.conf", 0, 0, 0o444, b"two"),
        ];
        let mounts = bind_mounts(dir, &files);
        let describe = |m: &Mount| {
            (
                m.source.clone().unwrap(),
                m.target.clone().unwrap(),
                m.typ,
                m.read_only,
            )
        };
        assert_eq!(
            mounts.iter().map(describe).collect::<Vec<_>>(),
            [
                (
                    "/data/core/configs/app.test/web/0".to_owned(),
                    "/etc/a.conf".to_owned(),
                    Some(MountType::BIND),
                    Some(true)
                ),
                (
                    "/data/core/configs/app.test/web/1".to_owned(),
                    "/b.conf".to_owned(),
                    Some(MountType::BIND),
                    Some(true)
                ),
            ]
        );
    }

    #[test]
    fn archives_are_named_by_index_and_carry_owner_and_mode() {
        let files = [
            file("/etc/a.conf", 65532, 65532, 0o440, b"one"),
            file("/b.conf", 0, 0, 0o444, b"two"),
        ];
        let archive = archive(&files).unwrap();
        let mut reader = tar::Archive::new(archive.as_slice());
        let mut entries = reader.entries().unwrap().map(|entry| {
            let entry = entry.unwrap();
            let header = entry.header();
            (
                entry.path().unwrap().to_path_buf(),
                header.uid().unwrap(),
                header.gid().unwrap(),
                header.mode().unwrap(),
            )
        });
        assert_eq!(
            entries.next(),
            Some((PathBuf::from("0"), 65532, 65532, 0o440))
        );
        assert_eq!(entries.next(), Some((PathBuf::from("1"), 0, 0, 0o444)));
        assert_eq!(entries.next(), None);
    }

    #[test]
    fn removing_a_missing_tree_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        remove_tree(dir.path(), &Instance::default(), "app.test").unwrap();
        let instance = Instance::parse(Some("dev")).unwrap();
        let web = service_dir(dir.path(), &instance, "app.test", "web");
        fs::create_dir_all(&web).unwrap();
        fs::write(web.join("0"), "x").unwrap();
        remove_tree(dir.path(), &instance, "app.test").unwrap();
        assert!(!plugin_dir(dir.path(), &instance, "app.test").exists());
    }
}
