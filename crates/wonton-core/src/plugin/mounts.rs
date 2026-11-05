//! 把 compose 的 `volumes` 变成 Docker 挂载，并检查宿主路径：只能是应用数据目录（或其子目录）和已授权
//! 共享文件夹的根目录，同一插件的绑定挂载源不能嵌套（ADR-0011 的“插件隔离的约束”）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use bollard::models::{Mount, MountType};

use super::compose::{LongVolume, Service, UserSpec, VolumeEntry, VolumeType};
use super::hostdir::Owner;
use super::render::{InstallContext, RenderError, list, service_err};

/// 把 compose 的 `volumes` 变成 Docker 挂载，并检查宿主路径是否在允许范围内。
pub(super) struct Mounts<'a> {
    pub ctx: &'a InstallContext,
    pub read_only_shares: &'a [PathBuf],
    pub named: &'a BTreeSet<String>,
    pub service: &'a str,
    pub allow_docker_socket: bool,
}

impl Mounts<'_> {
    /// `binds` 收集每个绑定挂载的（服务名，宿主路径），供跨服务的嵌套检查。
    pub fn build(
        &self,
        entries: &[VolumeEntry],
        binds: &mut Vec<(String, PathBuf)>,
    ) -> Result<Vec<Mount>, RenderError> {
        entries
            .iter()
            .map(|entry| {
                let long = match entry {
                    VolumeEntry::Long(long) => long.clone(),
                    VolumeEntry::Short(text) => self.parse_short(text)?,
                };
                self.mount(long, binds)
            })
            .collect()
    }

    fn parse_short(&self, text: &str) -> Result<LongVolume, RenderError> {
        let parts: Vec<&str> = text.split(':').collect();
        let (source, target, mode) = match parts.as_slice() {
            [target] => (None, *target, ""),
            [source, target] => (Some(*source), *target, ""),
            [source, target, mode] => (Some(*source), *target, *mode),
            _ => return Err(self.err(format!("卷 `{text}` 格式不合法"))),
        };
        let modes: Vec<&str> = mode.split(',').filter(|m| !m.is_empty()).collect();
        if let Some(bad) = modes
            .iter()
            .find(|m| !matches!(**m, "ro" | "rw" | "z" | "Z"))
        {
            return Err(self.err(format!("卷选项 `{bad}` 不受支持")));
        }
        let kind = match source {
            None => VolumeType::Volume,
            Some(s) if s.starts_with('/') => VolumeType::Bind,
            Some(s) if s.starts_with('.') || s.starts_with('~') => {
                return Err(self.err(format!(
                    "不支持相对路径 `{s}`，请使用 ${{WONTON_APP_DATA}} 下的路径"
                )));
            }
            Some(_) => VolumeType::Volume,
        };
        Ok(LongVolume {
            kind,
            source: source.map(str::to_owned),
            target: target.to_owned(),
            read_only: modes.contains(&"ro"),
        })
    }

    fn mount(
        &self,
        volume: LongVolume,
        binds: &mut Vec<(String, PathBuf)>,
    ) -> Result<Mount, RenderError> {
        if !volume.target.starts_with('/') {
            return Err(self.err(format!("挂载目标 `{}` 必须是绝对路径", volume.target)));
        }
        let source = match (volume.kind, volume.source) {
            (VolumeType::Tmpfs, _) => None,
            (VolumeType::Volume, None) => None,
            (VolumeType::Volume, Some(name)) => {
                if !self.named.contains(&name) {
                    return Err(self.err(format!("命名卷 `{name}` 没有在顶层 volumes 中声明")));
                }
                Some(self.ctx.instance.volume(&self.ctx.slug, &name))
            }
            (VolumeType::Bind, None) => return Err(self.err("绑定挂载缺少 source")),
            (VolumeType::Bind, Some(path))
                if self.allow_docker_socket
                    && path == super::privileges::DOCKER_SOCKET
                    && volume.target == super::privileges::DOCKER_SOCKET =>
            {
                Some(super::privileges::DOCKER_SOCKET.to_owned())
            }
            (VolumeType::Bind, Some(path)) => {
                let path = self.check_bind(&path, volume.read_only)?;
                binds.push((self.service.to_owned(), path.clone()));
                Some(path.display().to_string())
            }
        };
        let typ = match volume.kind {
            VolumeType::Bind => MountType::BIND,
            VolumeType::Volume => MountType::VOLUME,
            VolumeType::Tmpfs => MountType::TMPFS,
        };
        Ok(Mount {
            target: Some(volume.target),
            source,
            typ: Some(typ),
            read_only: Some(volume.read_only),
            ..Default::default()
        })
    }

    /// 宿主路径只能是应用数据目录（或其子目录）或已授权共享文件夹的根目录；只读共享必须以只读挂载。
    /// 返回去掉 `.`、重复的 `/` 和末尾 `/` 之后的路径。
    fn check_bind(&self, raw: &str, read_only: bool) -> Result<PathBuf, RenderError> {
        let raw = Path::new(raw);
        if raw.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(self.err(format!("宿主路径 `{}` 不能包含 ..", raw.display())));
        }
        let path: PathBuf = raw.components().collect();
        if path.starts_with(&self.ctx.app_data) {
            return Ok(path);
        }
        let Some(root) = self.ctx.shares.values().find(|root| path.starts_with(root)) else {
            return Err(self.err(format!(
                "宿主路径 `{}` 不在应用数据目录或已授权的共享文件夹里",
                path.display()
            )));
        };
        // 共享文件夹由多个插件共用，有写权限的插件能把其中的子目录换成软链接。
        if path != *root {
            return Err(self.err(format!(
                "共享文件夹只能整个挂载，不能只挂其中的 `{}`",
                path.display()
            )));
        }
        if self.read_only_shares.contains(root) && !read_only {
            return Err(self.err(format!(
                "共享文件夹 `{}` 只授权了读，必须以 :ro 挂载",
                root.display()
            )));
        }
        Ok(path)
    }

    fn err(&self, message: impl Into<String>) -> RenderError {
        service_err(self.service, message)
    }
}

/// 同一插件的绑定挂载源不能嵌套：能写外层目录的容器可以把内层的源换成软链接，Docker 下次启动容器时
/// 跟随它，就把宿主上的任意目录挂了进来。同一个目录挂进多个服务没有问题。
pub(super) fn check_not_nested(binds: &[(String, PathBuf)]) -> Result<(), RenderError> {
    for (service, inner) in binds {
        let outer = binds
            .iter()
            .find(|(_, outer)| outer != inner && inner.starts_with(outer));
        if let Some((outer_service, outer)) = outer {
            return Err(service_err(
                service,
                format!(
                    "宿主路径 `{}` 在服务 `{outer_service}` 挂载的 `{}` 里面；同一插件的绑定挂载不能嵌套\
                     （容器能把内层目录换成软链接，逃出应用数据目录），请改用并列的子目录",
                    inner.display(),
                    outer.display()
                ),
            ));
        }
    }
    Ok(())
}

/// 服务所有挂载的目标（卷、绑定挂载、`tmpfs`）。
pub(super) fn mount_targets(service: &Service, mounts: &[Mount]) -> Vec<PathBuf> {
    let tmpfs = service.tmpfs.as_ref().map(list).unwrap_or_default();
    mounts
        .iter()
        .filter_map(|m| m.target.clone())
        .chain(
            tmpfs
                .into_iter()
                .map(|entry| entry.split(':').next().unwrap_or_default().to_owned()),
        )
        .map(|target| Path::new(&target).components().collect())
        .collect()
}

/// 应用数据目录下的绑定挂载源交给谁：看挂载它的服务的 `user`。没写或是 root 时交给 PUID:PGID（以 root
/// 启动、再按 PUID/PGID 降权的镜像）；数字形式的 `uid:gid` 交给它。用户名或只有 uid 时看不出是谁，
/// 几个服务以不同的属主挂载同一个目录时也说不清交给谁，都拒绝。
pub(super) fn dir_owners(
    binds: &[(String, PathBuf)],
    services: &BTreeMap<String, Service>,
    ctx: &InstallContext,
) -> Result<BTreeMap<PathBuf, Owner>, RenderError> {
    let mut owners: BTreeMap<PathBuf, (Owner, &str)> = BTreeMap::new();
    for (service, path) in binds.iter().filter(|(_, p)| p.starts_with(&ctx.app_data)) {
        let owner = match UserSpec::parse(services[service].user.as_ref()) {
            UserSpec::Root => Owner {
                uid: ctx.puid,
                gid: ctx.pgid,
            },
            UserSpec::Ids { uid, gid } => Owner { uid, gid },
            UserSpec::Uid(uid) => return Err(numeric_user(service, &uid.to_string())),
            UserSpec::Named(user) => return Err(numeric_user(service, &user)),
        };
        match owners.get(path) {
            Some((other, other_service)) if *other != owner => {
                return Err(service_err(
                    service,
                    format!(
                        "和服务 `{other_service}` 以不同的属主挂载 `{}`，核心不知道把它交给谁",
                        path.display()
                    ),
                ));
            }
            _ => {
                owners.insert(path.clone(), (owner, service));
            }
        }
    }
    Ok(owners
        .into_iter()
        .map(|(path, (owner, _))| (path, owner))
        .collect())
}

fn numeric_user(service: &str, user: &str) -> RenderError {
    service_err(
        service,
        format!(
            "挂载了应用数据目录，user `{user}` 要写成数字形式的 uid:gid：核心据此决定目录交给谁"
        ),
    )
}
