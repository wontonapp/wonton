//! 读主机的情况：目录树照 2026-11 在真实主机上看到的样子搭（一台 AMD，一台 NVIDIA 加 ASPEED 管理显卡）。

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use super::super::docker::HostInfo;
use super::super::fake_docker::FakeDocker;
use super::super::gpu::HostPaths;
use super::super::host_facts::{ContainerAccess, GpuVendor, HostFacts, Known, NvidiaMemory, read};
use super::super::languages::Languages;
use super::super::names::Instance;

/// 读出的原因串（`Known::Unknown` 的 why）跟着目录走，测试用同一种语言对期望。
fn zh() -> Languages {
    Languages::from_header(Some("zh-Hans"))
}

struct Host {
    dir: tempfile::TempDir,
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

impl Host {
    fn new() -> Self {
        let host = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        fs::create_dir_all(host.path("sys/bus/pci/devices")).unwrap();
        fs::create_dir_all(host.path("dev/dri")).unwrap();
        write(
            &host.path("proc/meminfo"),
            "MemTotal:       65457028 kB\nMemFree:         1000 kB\nMemAvailable:   42827492 kB\n",
        );
        host
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    fn paths(&self) -> HostPaths {
        HostPaths {
            udev: None,
            dev: Some(self.path("dev")),
            sys: self.path("sys"),
            proc: self.path("proc"),
        }
    }

    /// 一个 PCI 设备，`drm` 是它名下的节点。
    fn card(
        &self,
        address: &str,
        class: &str,
        vendor: &str,
        driver: Option<&str>,
        drm: &[&str],
    ) -> PathBuf {
        let device = self.path("sys/bus/pci/devices").join(address);
        write(&device.join("class"), &format!("{class}\n"));
        write(&device.join("vendor"), &format!("{vendor}\n"));
        write(&device.join("device"), "0x1234\n");
        if let Some(driver) = driver {
            let target = self.path("sys/bus/pci/drivers").join(driver);
            fs::create_dir_all(&target).unwrap();
            symlink(target, device.join("driver")).unwrap();
        }
        for node in drm {
            fs::create_dir_all(device.join("drm").join(node)).unwrap();
        }
        device
    }
}

fn docker(runtimes: &[&str]) -> Result<HostInfo, String> {
    Ok(HostInfo {
        name: None,
        os_type: "linux".to_owned(),
        arch: "x86_64".to_owned(),
        runtimes: runtimes.iter().map(|&r| r.to_owned()).collect(),
        memory_limit: None,
        swap_limit: None,
        cpu_shares: None,
        pids_limit: None,
        cgroup_driver: None,
        ncpu: None,
        cgroup_version: None,
    })
}

fn only_gpu(facts: &HostFacts) -> &super::super::host_facts::Gpu {
    let Known::Known(gpus) = &facts.gpus else {
        panic!("{:?}", facts.gpus);
    };
    let [gpu] = &gpus[..] else {
        panic!("{gpus:?}");
    };
    gpu
}

#[test]
fn memory_comes_from_meminfo_and_a_missing_file_is_unknown_not_zero() {
    let host = Host::new();
    let facts = read(&host.paths(), docker(&[]), &zh());
    assert_eq!(facts.memory_total, Known::Known(65_457_028 * 1024));
    assert_eq!(facts.memory_available, Known::Known(42_827_492 * 1024));
    assert_eq!(facts.os, Known::Known("linux".to_owned()));
    assert_eq!(facts.arch, Known::Known("x86_64".to_owned()));

    fs::remove_file(host.path("proc/meminfo")).unwrap();
    let facts = read(&host.paths(), docker(&[]), &zh());
    assert!(matches!(facts.memory_total, Known::Unknown(_)));
    assert!(matches!(facts.memory_available, Known::Unknown(_)));
}

#[test]
fn an_amd_card_reports_its_vram_and_device_nodes() {
    let host = Host::new();
    let card = host.card(
        "0000:05:00.0",
        "0x038000",
        "0x1002",
        Some("amdgpu"),
        &["card1", "renderD128"],
    );
    write(&card.join("mem_info_vram_total"), "68719476736\n");
    write(&host.path("dev/dri/renderD128"), "");
    write(&host.path("dev/kfd"), "");

    let facts = read(&host.paths(), docker(&[]), &zh());
    let gpu = only_gpu(&facts);
    assert_eq!(gpu.vendor, GpuVendor::Amd);
    assert_eq!(gpu.vram, Known::Known(68_719_476_736));
    assert!(
        matches!(gpu.model, Known::Unknown(_)),
        "没有 product_name 就不编型号"
    );
    assert_eq!(
        gpu.in_containers,
        ContainerAccess::Usable {
            devices: vec!["/dev/dri/renderD128".to_owned(), "/dev/kfd".to_owned()],
        }
    );
}

#[test]
fn a_render_card_without_the_host_dev_mounted_is_unknown() {
    let host = Host::new();
    host.card(
        "0000:05:00.0",
        "0x030000",
        "0x8086",
        Some("i915"),
        &["renderD128"],
    );
    let paths = HostPaths {
        udev: None,
        dev: None,
        ..host.paths()
    };
    let facts = read(&paths, docker(&[]), &zh());
    let gpu = only_gpu(&facts);
    assert!(
        matches!(gpu.in_containers, ContainerAccess::Unknown(_)),
        "{gpu:?}"
    );
    assert!(
        matches!(gpu.vram, Known::Unknown(_)),
        "集显没有独立显存：{gpu:?}"
    );
}

#[test]
fn nvidia_needs_its_driver_and_the_docker_runtime() {
    let host = Host::new();
    let address = "0000:c7:00.0";
    host.card(
        address,
        "0x030000",
        "0x10de",
        Some("nvidia"),
        &["card1", "renderD128"],
    );
    write(
        &host
            .path("proc/driver/nvidia/gpus")
            .join(address)
            .join("information"),
        "Model: \t\t NVIDIA GeForce RTX 5090\nIRQ:   \t\t 1\n",
    );

    let facts = read(&host.paths(), docker(&["nvidia", "runc"]), &zh());
    let gpu = only_gpu(&facts);
    assert_eq!(
        gpu.model,
        Known::Known("NVIDIA GeForce RTX 5090".to_owned())
    );
    assert!(matches!(gpu.vram, Known::Unknown(_)));
    assert!(
        matches!(gpu.in_containers, ContainerAccess::Usable { .. }),
        "{gpu:?}"
    );

    let facts = read(&host.paths(), docker(&["runc"]), &zh());
    let access = &only_gpu(&facts).in_containers;
    assert!(
        matches!(access, ContainerAccess::Unusable(why) if why.contains("nvidia 运行时")),
        "{access:?}"
    );

    let facts = read(&host.paths(), Err("连不上 Docker".to_owned()), &zh());
    let access = &only_gpu(&facts).in_containers;
    assert!(
        matches!(access, ContainerAccess::Unknown(why) if why.contains("连不上 Docker")),
        "{access:?}"
    );
    assert!(matches!(facts.arch, Known::Unknown(_)));
}

#[test]
fn nvidia_hardware_without_its_driver_cannot_be_used() {
    for driver in [None, Some("nouveau")] {
        let host = Host::new();
        host.card("0000:01:00.0", "0x030000", "0x10de", driver, &[]);
        let facts = read(&host.paths(), docker(&["nvidia"]), &zh());
        let gpu = only_gpu(&facts);
        assert!(
            matches!(gpu.in_containers, ContainerAccess::Unusable(_)),
            "{gpu:?}"
        );
    }
}

#[test]
fn a_management_display_is_listed_but_cannot_compute_and_other_devices_are_not_gpus() {
    let host = Host::new();
    host.card(
        "0000:02:00.0",
        "0x030000",
        "0x1a03",
        Some("ast"),
        &["card0"],
    );
    host.card("0000:03:00.0", "0x020000", "0x8086", Some("igb"), &[]);

    let facts = read(&host.paths(), docker(&[]), &zh());
    let gpu = only_gpu(&facts);
    assert_eq!(gpu.vendor, GpuVendor::Other("0x1a03".to_owned()));
    assert!(
        matches!(gpu.in_containers, ContainerAccess::Unusable(_)),
        "{gpu:?}"
    );
}

#[test]
fn unreadable_pci_devices_are_unknown_and_an_empty_bus_is_no_gpus() {
    let host = Host::new();
    assert_eq!(
        read(&host.paths(), docker(&[]), &zh()).gpus,
        Known::Known(Vec::new())
    );
    fs::remove_dir(host.path("sys/bus/pci/devices")).unwrap();
    assert!(matches!(
        read(&host.paths(), docker(&[]), &zh()).gpus,
        Known::Unknown(_)
    ));
}

#[tokio::test]
async fn nvidia_memory_comes_from_a_one_shot_container_with_the_cores_image() {
    let (fake, engine) = FakeDocker::start().await;
    fake.add_image_container("core", "wonton-core:test");
    let instance = Instance::default();
    let probe = instance.gpu_probe();
    let host = Host::new();
    host.card(
        "0000:c7:00.0",
        "0x030000",
        "0x10de",
        Some("nvidia"),
        &["renderD128"],
    );
    let usable = read(&host.paths(), docker(&["nvidia"]), &zh());

    fake.set_output(&probe, 0, "00000000:C7:00.0, 32607\n", "");
    let mut facts = usable.clone();
    NvidiaMemory::default()
        .fill(&mut facts, &engine, Some("core"), &instance, &zh())
        .await;
    assert_eq!(only_gpu(&facts).vram, Known::Known(32607 * 1024 * 1024));
    assert!(matches!(
        only_gpu(&facts).in_containers,
        ContainerAccess::Usable { .. }
    ));
    assert!(fake.containers().iter().all(|c| *c != probe), "用完删掉");

    fake.set_output(&probe, 9, "", "Failed to initialize NVML");
    let mut facts = usable.clone();
    NvidiaMemory::default()
        .fill(&mut facts, &engine, Some("core"), &instance, &zh())
        .await;
    let gpu = only_gpu(&facts);
    assert!(
        matches!(&gpu.vram, Known::Unknown(why) if why.contains("NVML")),
        "{gpu:?}"
    );
    assert!(
        matches!(gpu.in_containers, ContainerAccess::Unknown(_)),
        "{gpu:?}"
    );

    let mut facts = usable;
    NvidiaMemory::default()
        .fill(&mut facts, &engine, None, &instance, &zh())
        .await;
    assert!(matches!(only_gpu(&facts).vram, Known::Unknown(_)));
    assert!(matches!(
        only_gpu(&facts).in_containers,
        ContainerAccess::Usable { .. }
    ));
}
