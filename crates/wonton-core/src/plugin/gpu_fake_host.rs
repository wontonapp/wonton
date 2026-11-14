//! 测试用的假宿主：在临时目录里摆出 sysfs 和 `/dev` 的相关部分。设备节点用普通文件代替（测试进程不能 mknod），
//! 所属的组就是测试进程的组。

use std::fs;
use std::path::Path;

use super::gpu::HostPaths;

pub struct FakeHost {
    pub dir: tempfile::TempDir,
}

impl FakeHost {
    pub fn empty() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sys/class/drm")).unwrap();
        fs::create_dir_all(dir.path().join("sys/bus/pci/devices")).unwrap();
        fs::create_dir_all(dir.path().join("dev")).unwrap();
        Self { dir }
    }

    /// 带 AMD 集成显卡的主机：amdgpu 驱动、KFD、一个渲染节点和一个显示节点。
    pub fn amd() -> Self {
        let host = Self::empty();
        host.pci("0000:05:00.0", "0x1002", "0x038000", Some("amdgpu"));
        host.render_node("renderD128", "0x1002", "0000:05:00.0");
        fs::create_dir_all(host.sys().join("class/kfd/kfd")).unwrap();
        host.dev_node("kfd");
        host.dev_node("dri/card1");
        host.dev_node("dri/renderD128");
        fs::create_dir_all(host.dev().join("dri/by-path")).unwrap();
        host
    }

    /// 加载了 NVIDIA 官方驱动的主机：有 `nvidia` 内核模块，渲染节点属于 NVIDIA。
    pub fn nvidia() -> Self {
        let host = Self::empty();
        host.pci("0000:01:00.0", "0x10de", "0x030000", Some("nvidia"));
        host.render_node("renderD128", "0x10de", "0000:01:00.0");
        fs::create_dir_all(host.sys().join("module/nvidia")).unwrap();
        host
    }

    pub fn sys(&self) -> std::path::PathBuf {
        self.dir.path().join("sys")
    }

    pub fn dev(&self) -> std::path::PathBuf {
        self.dir.path().join("dev")
    }

    /// 核心直接跑在宿主上：看得到 `/dev`。
    pub fn paths(&self) -> HostPaths {
        HostPaths {
            udev: Some(self.dir.path().join("run/udev")),
            dev: Some(self.dev()),
            sys: self.sys(),
            proc: self.sys().with_file_name("proc"),
        }
    }

    /// 核心在容器里，compose 没有挂载宿主的 `/dev`。
    pub fn paths_without_dev(&self) -> HostPaths {
        HostPaths {
            udev: None,
            dev: None,
            sys: self.sys(),
            proc: self.sys().with_file_name("proc"),
        }
    }

    pub fn pci(&self, address: &str, vendor: &str, class: &str, driver: Option<&str>) {
        let device = self.sys().join("bus/pci/devices").join(address);
        fs::create_dir_all(&device).unwrap();
        fs::write(device.join("vendor"), format!("{vendor}\n")).unwrap();
        fs::write(device.join("class"), format!("{class}\n")).unwrap();
        if let Some(driver) = driver {
            let drivers = self.sys().join("bus/pci/drivers").join(driver);
            fs::create_dir_all(&drivers).unwrap();
            std::os::unix::fs::symlink(&drivers, device.join("driver")).unwrap();
        }
    }

    pub fn render_node(&self, name: &str, vendor: &str, address: &str) {
        let node = self.sys().join("class/drm").join(name);
        fs::create_dir_all(&node).unwrap();
        let device = self.sys().join("bus/pci/devices").join(address);
        std::os::unix::fs::symlink(&device, node.join("device")).unwrap();
        fs::create_dir_all(device.join("drm").join(name)).unwrap();
        fs::write(device.join("vendor"), format!("{vendor}\n")).unwrap();
    }

    /// `/proc/meminfo`，只有总内存。
    pub fn memory_total_kib(&self, kib: u64) {
        let proc = self.sys().with_file_name("proc");
        fs::create_dir_all(&proc).unwrap();
        fs::write(proc.join("meminfo"), format!("MemTotal: {kib} kB\n")).unwrap();
    }

    /// 一个串口：宿主 `/dev/serial/by-id/<by_id>` 指向 `tty/<node>`。
    pub fn serial(&self, by_id: &str, node: &str) {
        self.dev_node(&format!("tty/{node}"));
        let link = self.dev().join("serial/by-id").join(by_id);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(format!("../../tty/{node}"), &link).unwrap();
    }

    /// USB 总线存在。
    pub fn usb(&self) {
        fs::create_dir_all(self.dev().join("bus/usb/001")).unwrap();
        self.dev_node("bus/usb/001/002");
    }

    /// 一块整盘：`/dev/disk/by-id/<name>`（和 `wwn-<wwn>`）指向 `sd<node>`，sysfs 里有型号和序列号。
    pub fn disk(&self, node: &str, by_id: &str, wwn: &str, model: &str, serial: &str, dev: &str) {
        self.dev_node(node);
        let link = self.dev().join("disk/by-id").join(by_id);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(format!("../../{node}"), &link).unwrap();
        std::os::unix::fs::symlink(
            format!("../../{node}"),
            self.dev().join("disk/by-id").join(wwn),
        )
        .unwrap();
        let dir = self.sys().join("block").join(node).join("device");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("model"), format!("{model}\n")).unwrap();
        fs::write(dir.join("serial"), format!("{serial}\n")).unwrap();
        fs::write(
            self.sys().join("block").join(node).join("dev"),
            format!("{dev}\n"),
        )
        .unwrap();
    }

    /// 盘上有一个分区：by-id、`partition` 标记和 sysfs 的设备号。
    pub fn partition(&self, node: &str, by_id: &str, number: u32, dev: &str) {
        let separator = if node.ends_with(|c: char| c.is_ascii_digit()) {
            "p"
        } else {
            ""
        };
        let partition = format!("{node}{separator}{number}");
        let dir = self.sys().join("block").join(node).join(&partition);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("dev"), format!("{dev}\n")).unwrap();
        fs::write(dir.join("partition"), format!("{number}\n")).unwrap();
        self.dev_node(&partition);
        let class = self.sys().join("class/block");
        fs::create_dir_all(&class).unwrap();
        std::os::unix::fs::symlink(&dir, class.join(&partition)).unwrap();
        let link = self
            .dev()
            .join("disk/by-id")
            .join(format!("{by_id}-part{number}"));
        std::os::unix::fs::symlink(format!("../../{partition}"), link).unwrap();
    }

    /// 宿主的 `/run/udev` 存在。
    pub fn udev(&self) {
        fs::create_dir_all(self.dir.path().join("run/udev/data")).unwrap();
    }

    /// `/proc/self/mountinfo`：`挂载点 设备号` 的列表。
    pub fn mounts(&self, entries: &[(&str, &str)]) {
        let proc = self.sys().with_file_name("proc/self");
        fs::create_dir_all(&proc).unwrap();
        let mut table = String::new();
        for (index, (point, device)) in entries.iter().enumerate() {
            let point = point
                .replace('\\', "\\134")
                .replace(' ', "\\040")
                .replace('\t', "\\011")
                .replace('\n', "\\012");
            table.push_str(&format!(
                "{} 0 {device} / {point} rw - ext4 /dev/x rw\n",
                index + 1
            ));
        }
        fs::write(proc.join("mountinfo"), table).unwrap();
    }

    pub fn dev_node(&self, relative: &str) {
        let path = self.dev().join(relative);
        fs::create_dir_all(path.parent().unwrap_or(Path::new("/"))).unwrap();
        fs::write(path, "").unwrap();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::os::unix::fs::MetadataExt;

    use bollard::models::HostConfig;

    use super::FakeHost;

    use crate::plugin::gpu::*;
    use crate::plugin::languages::Languages;
    use crate::plugin::requirements::GpuVendor;
    use wonton_l10n::Arg;

    /// 测试用的语言和期望值都走同一个目录，防止文案漂移。
    fn zh() -> Languages {
        Languages::from_header(Some("zh-Hans"))
    }

    fn say(id: &str, args: &[(&str, Arg<'_>)]) -> String {
        crate::l10n::text(&zh(), id, args)
    }

    fn zh_list(items: &[&str]) -> String {
        crate::l10n::list(&zh(), items)
    }

    #[test]
    fn amd_hosts_hand_over_kfd_dri_and_the_owning_groups() {
        let host = FakeHost::amd();
        let grant = probe(&host.paths(), &zh()).unwrap();
        assert_eq!(grant.devices, ["/dev/kfd", "/dev/dri"]);
        assert_eq!(grant.vendors, BTreeSet::from([GpuVendor::Amd]));
        // 假设备节点是普通文件，属于测试进程的组；真机上是 video、render 的 GID。
        let gid = std::fs::metadata(host.dev().join("kfd")).unwrap().gid();
        let expected: Vec<u32> = if gid == 0 { Vec::new() } else { vec![gid] };
        assert_eq!(grant.groups, expected);
        let text = grant.keep(GpuVendor::Amd).describe(&zh());
        let groups: Vec<String> = expected.iter().map(ToString::to_string).collect();
        let group_refs: Vec<&str> = groups.iter().map(String::as_str).collect();
        assert_eq!(
            text,
            say(
                "plugin-install-gpu-describe-vendors",
                &[
                    ("vendors", "AMD".into()),
                    ("devices", zh_list(&["/dev/kfd", "/dev/dri"]).into()),
                    ("groups", zh_list(&group_refs).into()),
                ],
            ),
        );
    }

    #[test]
    fn render_nodes_without_kfd_still_get_dri() {
        let host = FakeHost::empty();
        host.pci("0000:00:02.0", "0x8086", "0x030000", Some("i915"));
        host.render_node("renderD128", "0x8086", "0000:00:02.0");
        host.dev_node("dri/renderD128");
        let grant = probe(&host.paths(), &zh()).unwrap();
        assert_eq!(grant.devices, ["/dev/dri"]);
        assert_eq!(grant.vendors, BTreeSet::from([GpuVendor::Intel]));
    }

    #[test]
    fn nvidia_needs_the_kernel_module_not_the_binary() {
        let host = FakeHost::nvidia();
        let grant = probe(&host.paths(), &zh()).unwrap();
        assert!(grant.nvidia);
        assert!(grant.devices.is_empty(), "NVIDIA 的设备由运行时放进容器");
        assert_eq!(grant.vendors, BTreeSet::from([GpuVendor::Nvidia]));
        assert_eq!(
            grant.keep(GpuVendor::Nvidia).describe(&zh()),
            say("plugin-install-gpu-describe-nvidia", &[])
        );

        // 同样的显卡，驱动没加载（nvidia-smi 装了也没用）：如实说没有可用的驱动。
        let host = FakeHost::empty();
        host.pci("0000:01:00.0", "0x10de", "0x030000", None);
        let err = probe(&host.paths(), &zh()).unwrap_err();
        assert!(
            matches!(&err, GpuProblem::NoDriver(detail) if detail.contains("NVIDIA") && detail.contains(&say("plugin-install-gpu-no-driver-none", &[]))),
            "{err:?}"
        );
        let host = FakeHost::empty();
        host.pci("0000:01:00.0", "0x10de", "0x030000", Some("nouveau"));
        host.render_node("renderD128", "0x10de", "0000:01:00.0");
        let err = probe(&host.paths(), &zh()).unwrap_err();
        assert!(
            matches!(&err, GpuProblem::NoDriver(detail) if detail.contains("nouveau")),
            "{err:?}"
        );
    }

    #[test]
    fn hosts_without_a_gpu_say_so() {
        let host = FakeHost::empty();
        host.pci("0000:00:1f.0", "0x8086", "0x060100", Some("lpc_ich"));
        assert_eq!(
            probe(&host.paths(), &zh()).unwrap_err(),
            GpuProblem::NoDevice
        );
        // sysfs 里连 drm、pci 目录都没有（比如测试机）也是没有显卡，不是读错。
        let bare = tempfile::tempdir().unwrap();
        let paths = HostPaths {
            udev: None,
            dev: Some(bare.path().join("dev")),
            sys: bare.path().join("sys"),
            proc: bare.path().join("proc"),
        };
        assert_eq!(probe(&paths, &zh()).unwrap_err(), GpuProblem::NoDevice);
    }

    #[test]
    fn a_core_in_a_container_without_the_host_dev_mount_does_not_guess() {
        let host = FakeHost::amd();
        assert_eq!(
            probe(&host.paths_without_dev(), &zh()).unwrap_err(),
            GpuProblem::HostDevUnavailable
        );
        // 有渲染节点、/dev/dri 里却没有设备节点：报读不了，不悄悄给一个空目录。
        let host = FakeHost::empty();
        host.pci("0000:05:00.0", "0x1002", "0x038000", Some("amdgpu"));
        host.render_node("renderD128", "0x1002", "0000:05:00.0");
        std::fs::create_dir_all(host.dev().join("dri")).unwrap();
        assert!(matches!(
            probe(&host.paths(), &zh()).unwrap_err(),
            GpuProblem::Unreadable { .. }
        ));
    }

    #[test]
    fn grants_become_devices_groups_and_device_requests() {
        let grant = GpuGrant {
            vendor: GpuVendor::Amd,
            devices: vec!["/dev/kfd".into(), "/dev/dri".into()],
            groups: vec![983, 987],
            nvidia: true,
            images: Vec::new(),
            arch_unknown: Vec::new(),
        };
        let mut host = HostConfig::default();
        grant.apply(&mut host);
        let devices = host.devices.unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].path_on_host.as_deref(), Some("/dev/kfd"));
        assert_eq!(devices[0].path_in_container.as_deref(), Some("/dev/kfd"));
        assert_eq!(devices[0].cgroup_permissions.as_deref(), Some("rwm"));
        assert_eq!(host.group_add.unwrap(), ["983", "987"]);
        let request = &host.device_requests.unwrap()[0];
        assert_eq!(request.count, Some(-1));
        assert_eq!(
            request.capabilities.as_deref(),
            Some(&[vec!["gpu".to_owned()]][..])
        );
        assert_eq!(
            grant.describe(&zh()),
            format!(
                "{}；{}",
                say(
                    "plugin-install-gpu-describe-vendors",
                    &[
                        ("vendors", "AMD".into()),
                        ("devices", zh_list(&["/dev/kfd", "/dev/dri"]).into()),
                        ("groups", zh_list(&["983", "987"]).into()),
                    ],
                ),
                say("plugin-install-gpu-describe-nvidia", &[]),
            ),
        );
    }
}
