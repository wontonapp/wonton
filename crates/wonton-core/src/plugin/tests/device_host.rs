//! Linux host device discovery contracts, using only temporary fake filesystem trees.

use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::PathBuf;

use crate::plugin::device::Listing;
use crate::plugin::device::host::HostDevices;
use crate::plugin::gpu_fake_host::FakeHost;
use crate::plugin::manifest::DeviceClass;

use super::langs;

fn list(host: &FakeHost, data: impl Into<PathBuf>) -> Listing {
    HostDevices::new(&host.paths(), data.into())
        .list(&langs())
        .unwrap()
}

#[test]
fn root_and_longest_data_mount_mark_their_whole_disks() {
    let host = FakeHost::empty();
    host.disk("sda", "ata-root", "wwn-root", "Root", "R", "8:0");
    host.disk("sdb", "ata-data", "wwn-data", "Data", "D", "8:16");
    host.disk("sdc", "ata-other", "wwn-other", "Other", "O", "8:32");
    host.partition("sda", "ata-root", 1, "8:1");
    host.partition("sdb", "ata-data", 2, "8:18");
    host.mounts(&[("/", "8:1"), ("/srv", "8:32"), ("/srv/wonton", "8:18")]);

    let disks = list(&host, "/srv/wonton/data").disks;
    assert_eq!(disks.len(), 3);
    assert_eq!(
        disks
            .iter()
            .map(|disk| (disk.node.as_str(), disk.system))
            .collect::<Vec<_>>(),
        [("sda", true), ("sdb", true), ("sdc", false)]
    );
}

#[test]
fn root_mount_alone_covers_data_and_remains_a_system_disk() {
    let host = FakeHost::empty();
    host.disk("sda", "ata-root", "wwn-root", "Root", "R", "8:0");
    host.mounts(&[("/", "8:0")]);
    assert!(list(&host, "/var/lib/wonton").disks[0].system);

    // A non-block root does not erase a separately proven data disk.
    host.mounts(&[("/", "0:99"), ("/var/lib/wonton", "8:0")]);
    assert!(list(&host, "/var/lib/wonton/data").disks[0].system);
    host.mounts(&[("/", "0:99")]);
    assert!(!list(&host, "/var/lib/wonton/data").disks[0].system);
}

#[test]
fn mount_paths_preserve_unicode_escapes_and_component_boundaries() {
    let host = FakeHost::empty();
    host.disk("sda", "ata-data", "wwn-data", "Data", "D", "8:0");
    host.mounts(&[("/", "0:99"), ("/数据 disk\\backup\tfile", "8:0")]);
    assert!(list(&host, "/数据 disk\\backup\tfile/wonton").disks[0].system);
    assert!(!list(&host, "/数据 disk\\backup\tfiles/wonton").disks[0].system);

    let proc = host.dir.path().join("proc/self/mountinfo");
    fs::write(proc, b"1 0 8:0 / /data\\377 rw - ext4 /dev/x rw\n").unwrap();
    let data = PathBuf::from(std::ffi::OsString::from_vec(b"/data\xff/wonton".to_vec()));
    assert!(list(&host, data).disks[0].system);
}

#[test]
fn aliases_deduplicate_and_sysfs_rejects_partitions_without_part_suffixes() {
    let host = FakeHost::empty();
    host.disk("sda", "ata-drive", "wwn-drive", "Drive", "S1", "8:0");
    host.partition("sda", "ata-drive", 1, "8:1");
    host.disk("dm-0", "dm-name-volume", "dm-uuid-volume", "Volume", "V", "253:0");
    fs::remove_dir_all(host.sys().join("block/dm-0/device")).unwrap();
    let aliases = host.dev().join("disk/by-id");
    symlink("../../sda1", aliases.join("custom-partition-name")).unwrap();
    symlink("../../sda", aliases.join("usb-another-alias")).unwrap();
    // Also reject a partition exposed under /sys/block by an unusual host tree.
    symlink(
        host.sys().join("block/sda/sda1"),
        host.sys().join("block/sda1"),
    )
    .unwrap();

    let disks = list(&host, "/data").disks;
    assert_eq!(disks.len(), 1);
    let disk = &disks[0];
    assert_eq!(disk.host_path, "/dev/disk/by-id/ata-drive");
    assert_eq!(disk.node, "sda");
    assert_eq!(disk.name, "Drive（S1）");
    assert_eq!(disk.class, DeviceClass::Disk);
    assert_eq!(
        disk.gid,
        fs::metadata(host.dev().join("sda")).unwrap().gid()
    );
}

#[test]
fn nvme_namespace_keeps_identity_and_partition_system_marker() {
    let host = FakeHost::empty();
    host.disk(
        "nvme2n1",
        "nvme-drive",
        "wwn-drive",
        "Fast drive",
        "NV123",
        "259:0",
    );
    fs::write(host.sys().join("block/nvme2n1/nsid"), "1\n").unwrap();
    host.partition("nvme2n1", "nvme-drive", 3, "259:3");
    host.mounts(&[("/", "259:3")]);
    // NVMe namespaces may expose attributes directly rather than under device/.
    let block = host.sys().join("block/nvme2n1");
    fs::rename(block.join("device/model"), block.join("model")).unwrap();
    fs::rename(block.join("device/serial"), block.join("serial")).unwrap();
    fs::remove_dir(block.join("device")).unwrap();

    let disks = list(&host, "/data").disks;
    assert_eq!(disks.len(), 1);
    assert_eq!(disks[0].node, "nvme2n1");
    assert_eq!(disks[0].host_path, "/dev/disk/by-id/nvme-drive");
    assert_eq!(disks[0].name, "Fast drive（NV123）");
    assert!(disks[0].system);
}

#[test]
fn serial_identity_survives_reenumeration_and_usb_is_a_bus_class() {
    let host = FakeHost::empty();
    let initial = list(&host, "/data");
    assert!(initial.serial.is_empty());
    assert!(initial.disks.is_empty());
    assert!(!initial.usb);
    host.serial("usb-coordinator", "ttyUSB0");
    let initial = list(&host, "/data");
    assert_eq!(initial.serial[0].class, DeviceClass::Serial);
    assert_eq!(
        initial.serial[0].host_path,
        "/dev/serial/by-id/usb-coordinator"
    );
    assert_eq!(initial.serial[0].node, "ttyUSB0");
    assert!(!initial.usb);
    host.serial("usb-coordinator", "ttyACM9");
    host.usb();
    let changed = list(&host, "/data");
    assert_eq!(changed.serial[0].host_path, initial.serial[0].host_path);
    assert_eq!(changed.serial[0].node, "ttyACM9");
    assert!(changed.usb);
    assert!(!changed.udev);
    host.udev();
    assert!(list(&host, "/data").udev);
}

#[test]
fn malformed_discovery_directories_and_sysfs_reads_are_errors() {
    let host = FakeHost::empty();
    fs::create_dir_all(host.dev().join("disk")).unwrap();
    fs::write(host.dev().join("disk/by-id"), "not a directory").unwrap();
    assert!(
        HostDevices::new(&host.paths(), "/data".into())
            .list(&langs())
            .is_err()
    );
    fs::remove_file(host.dev().join("disk/by-id")).unwrap();
    host.disk("sda", "ata-data", "wwn-data", "Data", "D", "8:0");
    let model = host.sys().join("block/sda/device/model");
    fs::remove_file(&model).unwrap();
    fs::create_dir(model).unwrap();
    assert!(
        HostDevices::new(&host.paths(), "/data".into())
            .list(&langs())
            .is_err()
    );
}

#[test]
fn mountinfo_read_failure_is_not_reported_as_an_ordinary_disk() {
    let host = FakeHost::empty();
    host.disk("sda", "ata-data", "wwn-data", "Data", "D", "8:0");
    fs::create_dir_all(host.dir.path().join("proc/self/mountinfo")).unwrap();
    assert!(
        HostDevices::new(&host.paths(), "/data".into())
            .list(&langs())
            .is_err()
    );
}

#[test]
fn unreadable_serial_aliases_do_not_silently_disappear() {
    let host = FakeHost::empty();
    host.serial("usb-coordinator", "ttyUSB0");
    fs::remove_file(host.dev().join("tty/ttyUSB0")).unwrap();
    assert!(
        HostDevices::new(&host.paths(), "/data".into())
            .list(&langs())
            .is_err()
    );
    fs::remove_file(host.dev().join("serial/by-id/usb-coordinator")).unwrap();
    fs::write(
        host.dev().join("serial/by-id/usb-coordinator"),
        "not a symlink",
    )
    .unwrap();
    assert!(
        HostDevices::new(&host.paths(), "/data".into())
            .list(&langs())
            .is_err()
    );
}
