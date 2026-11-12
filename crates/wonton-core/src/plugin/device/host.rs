//! 宿主设备的列举：串口按 `/dev/serial/by-id`、硬盘按 `/dev/disk/by-id` 里的整盘，USB 只看类在不在。
//!
//! 核心看到的是只读挂载进来的宿主 `/dev`（[`HostDevices::dev`]），系统盘按宿主根目录或 Wonton 数据目录所在的盘认。

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::super::gpu::HostPaths;
use super::super::languages::Languages;
use super::{Candidate, Listing};
use crate::l10n::text;

pub struct HostDevices {
    /// 宿主的 `/dev` 在核心里的路径；核心在容器里却没挂载它时为 `None`（探测如实报错）。
    pub dev: Option<PathBuf>,
    /// 宿主的 `/run/udev`：宿主没有（没装 udev 的最小系统）时为 `None`。
    pub udev: Option<PathBuf>,
    /// sysfs 和 procfs，系统盘要从它们认；测试里换成假的目录树。
    pub sys: PathBuf,
    pub proc: PathBuf,
    /// Wonton 数据目录：它所在的盘要标成系统盘。
    pub data_dir: PathBuf,
}

impl HostDevices {
    pub fn new(paths: &HostPaths, data_dir: PathBuf) -> Self {
        Self {
            dev: paths.dev.clone(),
            udev: paths.udev.clone(),
            sys: paths.sys.clone(),
            proc: paths.proc.clone(),
            data_dir,
        }
    }

    /// 这台主机上能选的设备。
    pub fn list(&self, langs: &Languages) -> Result<Listing, String> {
        let dev = self
            .dev
            .as_ref()
            .ok_or_else(|| text(langs, "plugin-install-device-host-dev-missing", &[]))?;
        Ok(Listing {
            serial: self.serial(dev, langs)?,
            disks: self.disks(dev, langs)?,
            usb: directory(&dev.join("bus/usb"))?,
            udev: self
                .udev
                .as_deref()
                .map(directory)
                .transpose()?
                .unwrap_or(false),
        })
    }

    /// `/dev/serial/by-id/*`：by-id 名和它现在指向的节点。
    fn serial(&self, dev: &Path, langs: &Languages) -> Result<Vec<Candidate>, String> {
        let mut items = Vec::new();
        for (name, path) in entries(&dev.join("serial/by-id"))? {
            let node = link_name(&path)?;
            let gid = gid(&path).map_err(|e| unreadable(&path, &e, langs))?;
            items.push(Candidate {
                class: super::DeviceClass::Serial,
                host_path: host_path(dev, &path),
                name: format!("{name}（{node}）"),
                node,
                system: false,
                gid,
            });
        }
        Ok(items)
    }

    /// 同一块盘的多个 by-id 合成一项，优先保留型号名而非 `wwn-*`。
    fn disks(&self, dev: &Path, langs: &Languages) -> Result<Vec<Candidate>, String> {
        let mounted = self.mounted_devices()?;
        let mut by_node: BTreeMap<String, (String, PathBuf)> = BTreeMap::new();
        for (name, path) in entries(&dev.join("disk/by-id"))? {
            if is_partition(&name) {
                continue;
            }
            let node = link_name(&path)?;
            let block = self.sys.join("block").join(&node);
            if optional_text(&self.sys.join("class/block").join(&node).join("partition"))?.is_some()
                || !directory(&block)?
                || optional_text(&block.join("partition"))?.is_some()
                || (!directory(&block.join("device"))?
                    && optional_text(&block.join("nsid"))?.is_none())
            {
                continue;
            }
            match by_node.get(&node) {
                Some((kept, _)) if !kept.starts_with("wwn-") || name.starts_with("wwn-") => {}
                _ => {
                    by_node.insert(node, (name, path));
                }
            }
        }
        let mut items = Vec::new();
        for (node, (_, path)) in by_node {
            let gid = gid(&path).map_err(|e| unreadable(&path, &e, langs))?;
            items.push(Candidate {
                class: super::DeviceClass::Disk,
                host_path: host_path(dev, &path),
                name: self.disk_label(&node)?,
                system: self.is_system_disk(&node, &mounted)?,
                node,
                gid,
            });
        }
        Ok(items)
    }

    /// 型号和序列号；sysfs 没有时退回盘名。
    fn disk_label(&self, node: &str) -> Result<String, String> {
        let block = self.sys.join("block").join(node);
        let read = |name: &str| -> Result<Option<String>, String> {
            let value = match optional_text(&block.join("device").join(name))? {
                Some(value) => Some(value),
                None => optional_text(&block.join(name))?,
            };
            Ok(value
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()))
        };
        Ok(match (read("model")?, read("serial")?) {
            (Some(model), Some(serial)) => format!("{model}（{serial}）"),
            (Some(model), None) => model,
            (None, Some(serial)) => format!("{node}（{serial}）"),
            (None, None) => node.to_owned(),
        })
    }

    /// 保留已经证实的系统盘标记，即使另一个挂载点无法映射到物理盘。
    fn is_system_disk(&self, node: &str, mounted: &BTreeSet<String>) -> Result<bool, String> {
        if mounted.is_empty() {
            return Ok(false);
        }
        let dir = self.sys.join("block").join(node);
        if optional_text(&dir.join("dev"))?.is_some_and(|dev| mounted.contains(dev.trim())) {
            return Ok(true);
        }
        for (_, child) in entries(&dir)? {
            if directory(&child)?
                && optional_text(&child.join("dev"))?
                    .is_some_and(|dev| mounted.contains(dev.trim()))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// 根挂载和包含数据目录的最长挂载点；不存在或无法映射的设备号不冒充已识别的盘。
    fn mounted_devices(&self) -> Result<BTreeSet<String>, String> {
        let path = self.proc.join("self/mountinfo");
        let table = match fs::read(&path) {
            Ok(table) => table,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
            Err(e) => return Err(format!("{}：{e}", path.display())),
        };
        let mut numbers = BTreeSet::new();
        let mut best = (0usize, None::<String>);
        for line in table.split(|byte| *byte == b'\n') {
            let mut fields = line.split(|byte| byte.is_ascii_whitespace());
            let (Some(number), Some(point)) = (fields.nth(2), fields.nth(1)) else {
                continue;
            };
            let Ok(number) = std::str::from_utf8(number) else {
                continue;
            };
            let point = unescape(point);
            if point == Path::new("/") {
                numbers.insert(number.to_owned());
            }
            let length = point.as_os_str().len();
            if self.data_dir.starts_with(&point) && length >= best.0 {
                best = (length, Some(number.to_owned()));
            }
        }
        numbers.extend(best.1);
        Ok(numbers)
    }
}

/// mountinfo 转义的是路径字节，而非 Unicode 字符。
fn unescape(field: &[u8]) -> PathBuf {
    let mut out = Vec::with_capacity(field.len());
    let mut index = 0;
    while index < field.len() {
        if field[index] == b'\\'
            && let Some(digits) = field.get(index + 1..index + 4)
            && digits.iter().all(|digit| (b'0'..=b'7').contains(digit))
        {
            let code = u16::from(digits[0] - b'0') * 64
                + u16::from(digits[1] - b'0') * 8
                + u16::from(digits[2] - b'0');
            if let Ok(code) = u8::try_from(code) {
                out.push(code);
                index += 4;
                continue;
            }
        }
        out.push(field[index]);
        index += 1;
    }
    PathBuf::from(OsString::from_vec(out))
}

fn is_partition(name: &str) -> bool {
    name.rsplit_once("-part")
        .is_some_and(|(_, tail)| !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()))
}

fn optional_text(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}：{e}", path.display())),
    }
}

fn directory(path: &Path) -> Result<bool, String> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_dir()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("{}：{e}", path.display())),
    }
}

fn entries(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let read = match fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}：{e}", dir.display())),
    };
    let mut items = Vec::new();
    for entry in read {
        let entry = entry.map_err(|e| format!("{}：{e}", dir.display()))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|name| format!("{}：invalid device identifier {name:?}", dir.display()))?;
        items.push((name, entry.path()));
    }
    items.sort();
    Ok(items)
}

/// 软链接指向的节点名（`ttyUSB0`、`sda`）。
fn link_name(path: &Path) -> Result<String, String> {
    let target = fs::read_link(path).map_err(|e| format!("{}：{e}", path.display()))?;
    target
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("{}：invalid device node", path.display()))
}

pub(super) fn gid(path: &Path) -> std::io::Result<u32> {
    Ok(fs::metadata(path)?.gid())
}

/// 交给 dockerd 的宿主 by-id 路径：核心看到的那份目录不带上。
fn host_path(dev: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(dev).unwrap_or(path);
    format!("/dev/{}", relative.display())
}

fn unreadable(path: &Path, error: &std::io::Error, langs: &Languages) -> String {
    text(
        langs,
        "plugin-install-device-unreadable",
        &[
            ("path", path.display().to_string().into()),
            ("error", error.to_string().into()),
        ],
    )
}
