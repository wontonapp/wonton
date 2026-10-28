//! compose 的 `configs`：核心在创建容器之后、启动之前，用 Docker 的归档接口把文件写进容器。
//!
//! 不用绑定挂载：插件包存在核心自己的数据目录里，插件的容器访问不到它。所以 `uid`、`gid`、`mode`
//! 对 `file` 和 `content` 都生效，文件每次创建容器都重新写入。

use std::path::{Component, Path, PathBuf};

use super::compose::{Scalar, Service, UserSpec};
use super::interpolate::Vars;
use super::package::{PackageError, PluginPackage};

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
) -> Result<Vec<ConfigFile>, ConfigError> {
    let placed = place(package, service, mounts).map_err(ConfigError::Rule)?;
    placed
        .into_iter()
        .map(|(source, file)| {
            Ok(ConfigFile {
                data: package.config_data(&source, vars)?,
                ..file
            })
        })
        .collect()
}

/// 检查服务的每个配置引用，定下目标、属主和权限：（配置名，还没有内容的文件）。
fn place(
    package: &PluginPackage,
    service: &Service,
    mounts: &[PathBuf],
) -> Result<Vec<(String, ConfigFile)>, String> {
    if !service.configs.is_empty() && service.read_only == Some(true) {
        return Err(
            "read_only 的服务不能用 configs：Docker 不往只读的根文件系统写文件；\
                    可以由一次性服务把它复制进数据目录"
                .to_owned(),
        );
    }
    let (default_uid, default_gid) = default_owner(service.user.as_ref());
    let mut placed: Vec<(String, ConfigFile)> = Vec::new();
    for entry in &service.configs {
        let config = entry.long();
        if !package.has_config(&config.source) {
            return Err(format!(
                "配置 `{}` 没有在顶层 configs 中声明",
                config.source
            ));
        }
        let raw = config
            .target
            .clone()
            .unwrap_or_else(|| format!("/{}", config.source));
        let target = target_path(&raw)?;
        if placed.iter().any(|(_, f)| f.target == target) {
            return Err(format!("configs 的目标 `{raw}` 重复"));
        }
        if let Some(mount) = mounts
            .iter()
            .find(|m| target.starts_with(m) || m.starts_with(&target))
        {
            return Err(format!(
                "configs 的目标 `{raw}` 和挂载点 `{}` 重叠：配置写在容器自己的文件系统里，不能写进挂载点",
                mount.display()
            ));
        }
        let file = ConfigFile {
            target,
            uid: id(config.uid.as_ref(), "uid")?.unwrap_or(default_uid),
            gid: id(config.gid.as_ref(), "gid")?.unwrap_or(default_gid),
            mode: config.mode.as_ref().map_or(Ok(DEFAULT_MODE), mode)?,
            data: Vec::new(),
        };
        placed.push((config.source, file));
    }
    Ok(placed)
}

/// 配置的目标：容器里规范化的绝对路径，不能是 `/` 也不能含 `..`。
fn target_path(raw: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if !path.is_absolute() || path.components().any(|c| c == Component::ParentDir) {
        return Err(format!(
            "configs 的目标 `{raw}` 必须是绝对路径，且不能包含 .."
        ));
    }
    let path: PathBuf = path.components().collect();
    if path.parent().is_none() {
        return Err("configs 的目标不能是 /".to_owned());
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

fn id(value: Option<&Scalar>, field: &str) -> Result<Option<u32>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let parsed = match value {
        Scalar::Int(n) => u32::try_from(*n).ok(),
        Scalar::Str(s) => s.parse().ok(),
        Scalar::Float(_) | Scalar::Bool(_) => None,
    };
    parsed
        .map(Some)
        .ok_or_else(|| format!("configs 的 {field} `{}` 必须是非负整数", value.as_string()))
}

/// `mode` 按八进制理解：字符串按八进制解析；`0o440` 这样的 YAML 八进制数原样使用；`0440` 被 YAML 1.2
/// 解析成十进制浮点数 440，按它的数字当八进制，和 docker compose 一样得到 0440。写权限位被忽略。
fn mode(value: &Scalar) -> Result<u32, String> {
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
        _ => Err(format!(
            "configs 的 mode `{}` 不合法：须为 0 到 0777 的八进制权限",
            value.as_string()
        )),
    }
}

/// 把文件打成 tar，交给 Docker 解到容器的 `/` 下；属主和权限按归档里记录的写入。
pub fn archive(files: &[ConfigFile]) -> std::io::Result<Vec<u8>> {
    let mut builder = tar::Builder::new(Vec::new());
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
    fn modes_are_octal_and_never_writable() {
        let float = |f: f64| Scalar::Float(f);
        for (value, expected) in [
            (float(440.0), 0o440),
            (Scalar::Int(0o440), 0o440),
            (Scalar::Str("0440".into()), 0o440),
            (Scalar::Str("0o755".into()), 0o555),
            (float(644.0), 0o444),
        ] {
            assert_eq!(mode(&value).unwrap(), expected, "{value:?}");
        }
        for bad in [
            float(4.5),
            float(999.0),
            Scalar::Str("rw".into()),
            Scalar::Int(0o4755),
            Scalar::Bool(true),
        ] {
            assert!(mode(&bad).is_err(), "{bad:?}");
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
            target_path("/etc//app/./x.yaml").unwrap(),
            Path::new("/etc/app/x.yaml")
        );
        for bad in ["etc/x", "/", "/etc/../x", "./x"] {
            assert!(target_path(bad).is_err(), "{bad}");
        }
    }
}
