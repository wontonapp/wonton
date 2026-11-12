//! compose 的 `configs`：核心把内容打成 tar，交给 Docker 的归档接口写进容器。
//!
//! `read_only` 的服务根文件系统只读，归档接口写不进去（Docker 直接拒绝，见
//! [`super::configs_store`]）：它们的核心管理文件改为只读单文件绑定挂载交付。`resolve` 两种做法共用：
//! `read_only` 服务的文件由 [`super::configs_store`] 落在核心数据目录，其余的沿用归档接口，每次创建
//! 容器都重新写入。
//!
//! 不用绑定挂载交付普通服务：插件包存在核心自己的数据目录里，插件的容器访问不到它。所以 `uid`、
//! `gid`、`mode` 对 `file` 和 `content` 都生效。

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use bollard::models::Mount;
use bollard::query_parameters::DownloadFromContainerOptionsBuilder;
use futures_util::StreamExt;

use super::compose::{Scalar, Service, UserSpec};
use super::docker::{Engine, EngineError, from_bollard};
use super::interpolate::{UsedVars, Vars};
use super::languages::Languages;
use super::manager::InstallError;
use super::mounts::mount_targets;
use super::package::{PackageError, PluginPackage};
use super::render::{ContainerPlan, InstallContext, RenderError, service_err};

/// 补建的上级目录的权限：所有用户可穿过、可读。
const DIR_MODE: u32 = 0o755;
/// 判断路径是什么时最多读多少归档字节：只要第一项的头部。
const KIND_PROBE_BYTES: usize = 16 << 10;

/// 默认权限：所有用户可读，和 compose 一致。
const DEFAULT_MODE: u32 = 0o444;
/// 写权限位被忽略：写进容器的配置每次重建都会被覆盖，应用要改写的配置由一次性服务复制进数据目录。
const WRITE_BITS: u32 = 0o222;

/// 要写进容器的一个文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFile {
    /// 容器里的绝对路径。
    pub target: PathBuf,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// 取配置内容失败，如 `content` 插值失败。
    #[error(transparent)]
    Package(#[from] PackageError),
    /// 服务的 `configs` 不合规则。
    #[error("{0}")]
    Rule(String),
}

/// 解析服务的 `configs`。`mounts` 是这个服务所有挂载的目标：配置不能写进挂载点，否则在卷、应用数据
/// 目录或共享文件夹里留下文件（tmpfs 则在启动时把它盖住）。
pub fn resolve(
    package: &PluginPackage,
    service: &Service,
    vars: &Vars,
    mounts: &[PathBuf],
    used: &mut UsedVars,
    langs: &Languages,
) -> Result<Vec<ConfigFile>, ConfigError> {
    let placed = place(package, service, mounts, langs).map_err(ConfigError::Rule)?;
    placed
        .into_iter()
        .map(|(source, file)| {
            Ok(ConfigFile {
                data: package.config_data(&source, vars, used, langs)?,
                ..file
            })
        })
        .collect()
}

/// 服务的 `configs` 交付：解析内容，算好交付方式。`read_only` 的服务（归档接口写不进只读根）改为从
/// 核心数据目录只读挂载：配置文件以只读绑定挂载加进 `mounts`，返回 store 目录
/// （[`super::configs_store`]）；其余服务返回 `None`，创建容器后、启动前把归档写进容器根。
#[allow(clippy::too_many_arguments)] // 交付配置要同时知道包、服务、变量和挂载去向，逐个都要。
pub(super) fn resolve_service(
    package: &PluginPackage,
    service: &Service,
    ctx: &InstallContext,
    name: &str,
    vars: &Vars,
    mounts: &mut Vec<Mount>,
    used: &mut UsedVars,
    langs: &Languages,
) -> Result<(Vec<ConfigFile>, Option<PathBuf>), RenderError> {
    let targets = mount_targets(service, mounts);
    let files = resolve(package, service, vars, &targets, used, langs).map_err(|e| match e {
        ConfigError::Package(e) => e.into(),
        ConfigError::Rule(m) => service_err(langs, name, m),
    })?;
    let store_dir = (service.read_only == Some(true) && !files.is_empty()).then(|| {
        super::configs_store::service_dir(&ctx.config_store, &ctx.instance, &ctx.slug, name)
    });
    if let Some(dir) = &store_dir {
        mounts.extend(super::configs_store::bind_mounts(dir, &files));
    }
    Ok((files, store_dir))
}

/// 检查服务的每个配置引用，定下目标、属主和权限：（配置名，还没有内容的文件）。
fn place(
    package: &PluginPackage,
    service: &Service,
    mounts: &[PathBuf],
    langs: &Languages,
) -> Result<Vec<(String, ConfigFile)>, String> {
    let (default_uid, default_gid) = default_owner(service.user.as_ref());
    let mut placed: Vec<(String, ConfigFile)> = Vec::new();
    for entry in &service.configs {
        let config = entry.long();
        if !package.has_config(&config.source) {
            return Err(crate::l10n::text(
                langs,
                "plugin-install-cfg-undeclared",
                &[("name", config.source.as_str().into())],
            ));
        }
        let raw = config
            .target
            .clone()
            .unwrap_or_else(|| format!("/{}", config.source));
        let target = target_path(&raw, langs)?;
        if placed.iter().any(|(_, f)| f.target == target) {
            return Err(crate::l10n::text(
                langs,
                "plugin-install-cfg-target-duplicate",
                &[("raw", raw.as_str().into())],
            ));
        }
        if let Some(mount) = mounts
            .iter()
            .find(|m| target.starts_with(m) || m.starts_with(&target))
        {
            return Err(crate::l10n::text(
                langs,
                "plugin-install-cfg-target-overlaps-mount",
                &[
                    ("raw", raw.as_str().into()),
                    ("mount", mount.display().to_string().into()),
                ],
            ));
        }
        let file = ConfigFile {
            target,
            uid: id(config.uid.as_ref(), "uid", langs)?.unwrap_or(default_uid),
            gid: id(config.gid.as_ref(), "gid", langs)?.unwrap_or(default_gid),
            mode: config
                .mode
                .as_ref()
                .map_or(Ok(DEFAULT_MODE), |m| mode(m, langs))?,
            data: Vec::new(),
        };
        placed.push((config.source, file));
    }
    Ok(placed)
}

/// 配置的目标：容器里规范化的绝对路径，不能是 `/` 也不能含 `..`。
fn target_path(raw: &str, langs: &Languages) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if !path.is_absolute() || path.components().any(|c| c == Component::ParentDir) {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-cfg-target-absolute",
            &[("raw", raw.into())],
        ));
    }
    let path: PathBuf = path.components().collect();
    if path.parent().is_none() {
        return Err(crate::l10n::text(
            langs,
            "plugin-install-cfg-target-root",
            &[],
        ));
    }
    Ok(path)
}

/// 没写 `uid`、`gid` 时的属主：服务的 `user` 是数字时就是它（只写了 uid 时组是 0，和 Docker 找不到
/// 这个用户时一样），否则是 root。默认权限所有用户可读，只有收紧了 `mode` 才需要关心属主。
fn default_owner(user: Option<&Scalar>) -> (u32, u32) {
    match UserSpec::parse(user) {
        UserSpec::Ids { uid, gid } => (uid, gid),
        UserSpec::Uid(uid) => (uid, 0),
        UserSpec::Root | UserSpec::Named(_) => (0, 0),
    }
}

fn id(value: Option<&Scalar>, field: &str, langs: &Languages) -> Result<Option<u32>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let parsed = match value {
        Scalar::Int(n) => u32::try_from(*n).ok(),
        Scalar::Str(s) => s.parse().ok(),
        Scalar::Float(_) | Scalar::Bool(_) => None,
    };
    parsed.map(Some).ok_or_else(|| {
        crate::l10n::text(
            langs,
            "plugin-install-cfg-id-invalid",
            &[("field", field.into()), ("value", value.as_string().into())],
        )
    })
}

/// `mode` 按八进制理解：字符串按八进制解析；`0o640` 这样的 YAML 八进制数原样使用；`0640` 被 YAML 1.2
/// 解析成十进制浮点数 640，按它的数字当八进制，和 docker compose 一样得到 0640。写权限位被忽略。
fn mode(value: &Scalar, langs: &Languages) -> Result<u32, String> {
    let parsed = match value {
        Scalar::Int(n) => u32::try_from(*n).ok(),
        Scalar::Float(f) if f.fract() == 0.0 && *f >= 0.0 => {
            u32::from_str_radix(&format!("{f:.0}"), 8).ok()
        }
        Scalar::Str(s) => u32::from_str_radix(s.strip_prefix("0o").unwrap_or(s), 8).ok(),
        Scalar::Float(_) | Scalar::Bool(_) => None,
    };
    match parsed {
        Some(mode) if mode <= 0o777 => Ok(mode & !WRITE_BITS),
        _ => Err(crate::l10n::text(
            langs,
            "plugin-install-cfg-mode-invalid",
            &[("value", value.as_string().into())],
        )),
    }
}

/// 容器里不存在的目标上级目录：要写进容器根里的文件，先跟着归档补建这些目录，否则归档接口往不存在的目录里
/// 解不出来（见 [`archive`]）。同一个目录只建一次，由浅到深。建出来的目录属主是 root：只要能穿过。
pub async fn missing_parents(
    engine: &Engine,
    container: &str,
    files: &[ConfigFile],
) -> Result<Vec<PathBuf>, InstallError> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for file in files {
        let mut parents: Vec<&Path> = file.target.ancestors().skip(1).collect();
        parents.reverse();
        for parent in parents.into_iter().filter(|p| p != &Path::new("/")) {
            if dirs.iter().any(|known| known == parent) {
                continue;
            }
            match path_kind(engine, container, &parent.display().to_string()).await? {
                PathKind::Directory => {}
                PathKind::File => {
                    let langs = engine.owner_languages();
                    return Err(InstallError::Invalid(crate::l10n::text(
                        &langs,
                        "plugin-install-cfg-parent-is-file",
                        &[
                            ("service", container.into()),
                            ("parent", parent.display().to_string().into()),
                        ],
                    )));
                }
                PathKind::Missing => dirs.push(parent.to_path_buf()),
            }
        }
    }
    dirs.sort();
    Ok(dirs)
}

/// 把服务声明的 `configs` 交给 Docker 刚建好的容器（创建之后、启动之前）：只读根的服务先经一次性容器写进
/// 核心数据目录，再由 [只读绑定挂载](super::configs_store)交给容器；其余服务写进容器根，目标上级目录不存在
/// 时先补建。
pub(super) async fn deliver(
    engine: &Engine,
    container: &ContainerPlan,
    labels: &HashMap<String, String>,
) -> Result<(), InstallError> {
    let pack_failed = |e: &dyn std::fmt::Display| {
        InstallError::Invalid(crate::l10n::text(
            &engine.owner_languages(),
            "plugin-install-configs-pack-failed",
            &[
                ("service", container.name.as_str().into()),
                ("error", e.to_string().into()),
            ],
        ))
    };
    match &container.store_dir {
        Some(dir) => {
            let archive =
                super::configs_store::archive(&container.files).map_err(|e| pack_failed(&e))?;
            super::configs_store::seed(
                engine,
                &container.name,
                &container.image,
                dir,
                archive,
                labels,
            )
            .await
        }
        None if !container.files.is_empty() => {
            // 目标上级目录在镜像里可能不存在：先补建，不然归档解不出来，装完才发现应用读不到配置。
            let dirs = missing_parents(engine, &container.name, &container.files).await?;
            let archive = archive(&container.files, &dirs).map_err(|e| pack_failed(&e))?;
            engine
                .upload_archive(&container.name, "/", archive)
                .await
                .map_err(InstallError::from)
        }
        None => Ok(()),
    }
}

/// 容器里一个路径是什么。
enum PathKind {
    Missing,
    /// 目录，或者指向它的软链接。
    Directory,
    /// 普通文件：往下写要先建目录，写它会覆盖。
    File,
}

/// 经归档接口看容器里的一个路径：容器停着也行，只看归档的第一项，不读文件内容。
async fn path_kind(engine: &Engine, container: &str, path: &str) -> Result<PathKind, InstallError> {
    let langs = engine.owner_languages();
    let context = crate::l10n::text(
        &langs,
        "plugin-install-docker-context-read-file",
        &[("container", container.into()), ("path", path.into())],
    );
    let options = DownloadFromContainerOptionsBuilder::default()
        .path(path)
        .build();
    let mut stream = engine
        .docker
        .download_from_container(container, Some(options));
    let mut archive = Vec::new();
    while archive.len() < KIND_PROBE_BYTES {
        match stream.next().await {
            None => break,
            Some(Err(e)) => match from_bollard(&langs, &context, e) {
                EngineError::NotFound(_) => return Ok(PathKind::Missing),
                other => return Err(other.into()),
            },
            Some(Ok(bytes)) => archive.extend_from_slice(&bytes),
        }
    }
    let unparseable = |error: std::io::Error| EngineError::Rejected {
        context: context.clone(),
        message: crate::l10n::text(
            &langs,
            "plugin-install-docker-archive-unparseable",
            &[("error", error.to_string().into())],
        ),
    };
    let mut parsed = tar::Archive::new(archive.as_slice());
    let mut entries = parsed.entries().map_err(unparseable)?;
    let Some(entry) = entries.next() else {
        return Ok(PathKind::Missing);
    };
    let entry = entry.map_err(unparseable)?;
    Ok(match entry.header().entry_type() {
        tar::EntryType::Regular => PathKind::File,
        _ => PathKind::Directory,
    })
}

/// 把文件打成 tar，交给 Docker 解到容器的 `/` 下；属主和权限按归档里记录的写入。`dirs` 是要先补建的上级目录
/// （见 [`missing_parents`]），按顺序写在文件前面。
pub fn archive(files: &[ConfigFile], dirs: &[PathBuf]) -> std::io::Result<Vec<u8>> {
    let mut builder = tar::Builder::new(Vec::new());
    for dir in dirs {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_size(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mode(DIR_MODE);
        header.set_mtime(super::manager::now_secs());
        builder.append_data(
            &mut header,
            dir.strip_prefix("/").unwrap_or(dir),
            std::io::empty(),
        )?;
    }
    for file in files {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(file.data.len() as u64);
        header.set_uid(file.uid.into());
        header.set_gid(file.gid.into());
        header.set_mode(file.mode);
        header.set_mtime(super::manager::now_secs());
        let relative = file.target.strip_prefix("/").unwrap_or(&file.target);
        builder.append_data(&mut header, relative, file.data.as_slice())?;
    }
    builder.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_are_octal_and_keep_write_bits() {
        let float = |f: f64| Scalar::Float(f);
        for (value, expected) in [
            (float(440.0), 0o440),
            (Scalar::Int(0o440), 0o440),
            (Scalar::Str("0640".into()), 0o640),
            (Scalar::Str("0o755".into()), 0o755),
            (float(644.0), 0o644),
        ] {
            assert_eq!(
                mode(&value, &Languages::default()).unwrap(),
                expected,
                "{value:?}"
            );
        }
        for bad in [
            float(4.5),
            float(999.0),
            Scalar::Str("rw".into()),
            Scalar::Int(0o4755),
            Scalar::Bool(true),
        ] {
            assert!(mode(&bad, &Languages::default()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn default_owner_follows_a_numeric_user() {
        let user = |u: &str| Some(Scalar::Str(u.to_owned()));
        assert_eq!(default_owner(user("65532:65532").as_ref()), (65532, 65532));
        assert_eq!(default_owner(user("1000").as_ref()), (1000, 0));
        assert_eq!(default_owner(Some(&Scalar::Int(1000))), (1000, 0));
        for other in [user("nobody"), user("app:app"), None] {
            assert_eq!(default_owner(other.as_ref()), (0, 0), "{other:?}");
        }
    }

    #[test]
    fn targets_must_be_absolute_and_inside_the_container() {
        assert_eq!(
            target_path(
                "/etc//app/./x.yaml",
                &Languages::from_header(Some("zh-Hans"))
            )
            .unwrap(),
            Path::new("/etc/app/x.yaml")
        );
        for bad in ["etc/x", "/", "/etc/../x", "./x"] {
            assert!(target_path(bad, &Languages::default()).is_err(), "{bad}");
        }
    }
}
