//! 宿主设备的列举：串口按 `/dev/serial/by-id`、硬盘按 `/dev/disk/by-id` 里的整盘，USB 只看类在不在。
//!
//! 核心看到的是只读挂载进来的宿主 `/dev`（[`HostDevices::dev`]），系统盘按宿主根目录或 Wonton 数据目录所在的盘认。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
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
            usb: dev.join("bus/usb").is_dir(),
            udev: self.udev.as_deref().is_some_and(Path::is_dir),
        })
    }

    /// `/dev/serial/by-id/*`：by-id 名和它现在指向的节点。
    fn serial(&self, dev: &Path, langs: &Languages) -> Result<Vec<Candidate>, String> {
        let mut items = Vec::new();
        for (name, path) in entries(&dev.join("serial/by-id"))? {
            let Some(node) = link_name(&path) else {
                continue;
            };
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

    /// `/dev/disk/by-id` 里的整盘：同一块盘的 `wwn-*` 和型号名合成一项，只留 sysfs 里认得的块设备。
    fn disks(&self, dev: &Path, langs: &Languages) -> Result<Vec<Candidate>, String> {
        let mounted = self.mounted_devices();
        let mut by_node: BTreeMap<String, (String, PathBuf)> = BTreeMap::new();
        for (name, path) in entries(&dev.join("disk/by-id"))? {
            if is_partition(&name) {
                continue;
            }
            let Some(node) = link_name(&path) else {
                continue;
            };
            if !self.sys.join("block").join(&node).join("device").is_dir() {
                continue;
            }
            match by_node.get(&node) {
                // `wwn-*` 和型号名是同一块盘，显示用型号名那一项。
                Some((kept, _)) if !kept.starts_with("wwn-") || name.starts_with("wwn-") => {}
                _ => {
                    by_node.insert(node, (name, path));
                }
            }
        }
        let mut items = Vec::new();
        for (node, (_, path)) in by_node {
            let gid = gid(&path).map_err(|e| unreadable(&path, &e, langs))?;
            let system = self.is_system_disk(&node, &mounted);
            items.push(Candidate {
                class: super::DeviceClass::Disk,
                host_path: host_path(dev, &path),
                name: self.disk_label(&node),
                node,
                system,
                gid,
            });
        }
        Ok(items)
    }

    /// 型号和序列号；sysfs 没有时退回盘名。
    fn disk_label(&self, node: &str) -> String {
        let device = self.sys.join("block").join(node);
        let read = |name: &str| {
            fs::read_to_string(device.join("device").join(name))
                .or_else(|_| fs::read_to_string(device.join(name)))
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        match (read("model"), read("serial")) {
            (Some(model), Some(serial)) => format!("{model}（{serial}）"),
            (Some(model), None) => model,
            _ => node.to_owned(),
        }
    }

    /// 这块盘的分区（或盘本身）是不是宿主根目录或 Wonton 数据目录所在的盘。
    fn is_system_disk(&self, node: &str, mounted: &BTreeSet<String>) -> bool {
        if mounted.is_empty() {
            return false;
        }
        let dir = self.sys.join("block").join(node);
        let mut numbers = Vec::new();
        if let Ok(dev) = fs::read_to_string(dir.join("dev")) {
            numbers.push(dev.trim().to_owned());
        }
        for (name, _) in entries(&dir).unwrap_or_default() {
            if let Ok(dev) = fs::read_to_string(dir.join(&name).join("dev")) {
                numbers.push(dev.trim().to_owned());
            }
        }
        numbers.iter().any(|number| mounted.contains(number))
    }

    /// `/proc/self/mountinfo` 里宿主根目录和 Wonton 数据目录所用的设备号（`主:次`）。数据目录不是单独的挂载时
    /// 认不出，返回空、一块盘都不标。
    fn mounted_devices(&self) -> BTreeSet<String> {
        let Ok(table) = fs::read_to_string(self.proc.join("self/mountinfo")) else {
            return BTreeSet::new();
        };
        let mut numbers = BTreeSet::new();
        let mut best = (0usize, None::<String>);
        for line in table.lines() {
            let fields: Vec<&str> = line.split(' ').collect();
            let (Some(number), Some(point)) = (fields.get(2), fields.get(4)) else {
                continue;
            };
            let point = unescape(point);
            if point == "/" {
                numbers.insert((*number).to_owned());
            }
            let length = point.trim_end_matches('/').len();
            if self.data_dir.starts_with(&point) && length >= best.0 {
                best = (length, Some((*number).to_owned()));
            }
        }
        numbers.extend(best.1);
        numbers
    }
}

/// `mountinfo` 里空格等字符写成 `\040` 这样的八进制。
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = String::with_capacity(field.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && index + 3 < bytes.len()
            && let Ok(code) = u8::from_str_radix(&field[index + 1..index + 4], 8)
        {
            out.push(char::from(code));
            index += 4;
            continue;
        }
        out.push(char::from(bytes[index]));
        index += 1;
    }
    out.trim_end_matches('/').to_owned()
}

fn is_partition(name: &str) -> bool {
    name.rsplit_once("-part")
        .is_some_and(|(_, tail)| !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()))
}

fn entries(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let read = match fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}：{e}", dir.display())),
    };
    let mut items: Vec<(String, PathBuf)> = read
        .filter_map(Result::ok)
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            )
        })
        .collect();
    items.sort();
    Ok(items)
}

/// 软链接指向的节点名（`ttyUSB0`、`sda`）。
fn link_name(path: &Path) -> Option<String> {
    fs::read_link(path).ok().and_then(|target| {
        target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    })
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
