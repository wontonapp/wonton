//! 回收槽和私有名字上的东西。

use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::files::test_support::any_langs;

/// 回收槽里留着东西，就像改名之后崩溃了。
fn left_halfway(transfers: &std::path::Path, slot: usize, name: &str, content: &str) {
    prepare(transfers);
    fs::write(transfers.join(format!("{TRASH}/{slot}/{name}")), content).unwrap();
}

fn prepare(transfers: &std::path::Path) {
    let dir = File::open(transfers).unwrap();
    removal::prepare_trash(dir.as_fd(), &any_langs()).unwrap();
}

#[test]
fn files_left_halfway_through_a_delete_are_put_back() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    left_halfway(&transfers, 0, "a.txt", "photo");
    // 原来的名字已经被占了：谁都不盖。
    fs::write(transfers.join("a.txt"), "newer").unwrap();
    f.shares.recover_deleting().unwrap();
    assert_eq!(visible_in(&transfers), ["a (1).txt", "a.txt"]);
    assert_eq!(
        fs::read_to_string(transfers.join("a (1).txt")).unwrap(),
        "photo"
    );
    assert_eq!(trash_contents(&transfers), Vec::<String>::new());
}

/// 启动时建不了回收槽要报错。名字上放的不是目录就清掉，否则一个插件就能让核心起不来。
#[test]
fn slots_are_made_ahead_of_time_and_taken_back_from_whatever_sits_on_the_name() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    fs::write(transfers.join(TRASH), "planted").unwrap();
    f.shares.prepare_removal().unwrap();
    assert!(transfers.join(TRASH).join("0").is_dir());
    assert_eq!(
        fs::metadata(transfers.join(TRASH)).unwrap().mode() & 0o777,
        0o700
    );

    fs::remove_dir_all(f.root.join(TRANSFERS)).unwrap();
    let missing = f.shares.prepare_removal();
    assert!(missing.is_err(), "transfers 不在时要报错");
}

/// 槽里一直有一个名字最长的占位文件，目录才一直有块，磁盘写满时长名字也改得进去（ADR-0020 的“验证结果”）。它不算槽里有东西，也不会被
/// 当成删除到一半的文件放回去。
#[test]
fn a_slot_keeps_its_placeholder_and_still_counts_as_empty() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    assert_eq!(removal::keep_name().len(), 255);
    f.shares.prepare_removal().unwrap();
    f.shares.prepare_removal().unwrap();
    let keep = transfers.join(TRASH).join("7").join(removal::keep_name());
    assert!(keep.is_file());

    f.shares.recover_deleting().unwrap();
    assert!(keep.is_file());
    assert_eq!(visible_in(&transfers), Vec::<String>::new());

    fs::write(transfers.join("a.txt"), "photo").unwrap();
    let dir = File::open(&transfers).unwrap();
    let opened = open_regular(dir.as_fd(), "a.txt").unwrap();
    assert_eq!(
        removal::remove_if_same(
            dir.as_fd(),
            dir.as_fd(),
            "a.txt",
            &opened.snapshot,
            &any_langs()
        )
        .unwrap(),
        Deleted::Removed { durable: true }
    );
    assert!(
        transfers
            .join(TRASH)
            .join("0")
            .join(removal::keep_name())
            .is_file()
    );
}

/// 插件也可能往槽里放东西：不能挡住别的文件，不能逃出 `transfers`，也不能占核心的私有名字。
#[test]
fn one_bad_entry_does_not_hold_up_the_others() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    left_halfway(&transfers, 0, ".wonton-trash", "x");
    left_halfway(&transfers, 0, "good.txt", "photo");
    fs::create_dir(transfers.join(TRASH).join("1/album")).unwrap();
    f.shares.recover_deleting().unwrap();
    assert_eq!(
        visible_in(&transfers),
        ["album", "good.txt", "wonton-trash"]
    );
    assert!(!f.root.join("escaped").exists());
    assert_eq!(trash_contents(&transfers), Vec::<String>::new());
}

/// 放回目录时不能盖掉同名的空目录。
#[test]
fn a_directory_put_back_does_not_replace_an_empty_one() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    prepare(&transfers);
    fs::create_dir(transfers.join("album")).unwrap();
    fs::create_dir(transfers.join(TRASH).join("0/album")).unwrap();
    fs::write(transfers.join(TRASH).join("0/album/a.jpg"), "photo").unwrap();
    f.shares.recover_deleting().unwrap();
    assert_eq!(visible_in(&transfers), ["album", "album (1)"]);
    assert!(transfers.join("album (1)/a.jpg").exists());
}

/// 正常删除不留下任何东西，槽也空出来了。
#[test]
fn a_delete_leaves_nothing_behind() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    f.shares.prepare_removal().unwrap();
    fs::write(transfers.join("a.txt"), "photo").unwrap();
    let target = reference("transfers/a.txt");
    let snapshot = f.shares.open(&target).unwrap().snapshot;
    assert_eq!(
        f.shares
            .delete_if_unchanged(&target, &snapshot, &any_langs())
            .unwrap(),
        Deleted::Removed { durable: true }
    );
    assert_eq!(visible_in(&transfers), Vec::<String>::new());
    assert_eq!(trash_contents(&transfers), Vec::<String>::new());
}

/// 一个槽放不回去（比如文件系统出错）不挡别的删除。
#[test]
fn a_stuck_slot_does_not_stop_deletes() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    left_halfway(&transfers, 0, "stuck.txt", "stuck");
    fs::write(transfers.join("a.txt"), "photo").unwrap();
    let target = reference("transfers/a.txt");
    let snapshot = f.shares.open(&target).unwrap().snapshot;
    assert_eq!(
        f.shares
            .delete_if_unchanged(&target, &snapshot, &any_langs())
            .unwrap(),
        Deleted::Removed { durable: true }
    );
    assert_eq!(trash_contents(&transfers), ["0/stuck.txt"]);

    // 槽全占着：如实报错，文件不动。
    for slot in 1..8 {
        left_halfway(&transfers, slot, "stuck.txt", "stuck");
    }
    fs::write(transfers.join("b.txt"), "photo").unwrap();
    let target = reference("transfers/b.txt");
    let snapshot = f.shares.open(&target).unwrap().snapshot;
    let refused = f
        .shares
        .delete_if_unchanged(&target, &snapshot, &any_langs());
    assert!(matches!(refused, Err(ShareError::Io(_))), "{refused:?}");
    assert!(transfers.join("b.txt").exists());
}

/// 老内核和 NFS、CIFS 没有 `RENAME_NOREPLACE`：退回先查再改，不覆盖，也不能把目录永远藏起来。
#[test]
fn without_noreplace_a_rename_still_keeps_what_is_there() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    let dir = f.shares.open_dir(TRANSFERS, &[]).unwrap();
    fs::create_dir(transfers.join("from")).unwrap();
    fs::create_dir(transfers.join("taken")).unwrap();
    super::NO_NOREPLACE.set(true);
    let put = removal::put_back(dir.as_fd(), "from", dir.as_fd(), "taken", &any_langs());
    super::NO_NOREPLACE.set(false);
    assert_eq!(put.unwrap(), "taken (1)");
    assert_eq!(visible_in(&transfers), ["taken", "taken (1)"]);
}

/// 配额满了和磁盘满了一样：NAS 上常见。
#[test]
fn a_full_quota_reads_as_no_space() {
    assert!(matches!(
        classify(Errno::DQUOT, "a.txt"),
        ShareError::NoSpace(_)
    ));
}

type Plant<'a> = &'a dyn Fn(&std::path::Path);

/// 私有的临时名字上不是普通文件：清掉重放，否则这个上传永远放不进去。不是文件和空目录的不递归地删：插件可能把别人的目录改名过来，
/// 而核心可能以 root 运行。
#[test]
fn whatever_sits_on_a_temporary_name_is_cleared() {
    use std::os::unix::net::UnixListener;
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    let temp = |id: &str| transfers.join(format!("{UPLOAD_TEMP_PREFIX}{id}"));
    let plant: [(&str, Plant); 3] = [
        ("empty", &|path| fs::create_dir(path).unwrap()),
        ("link", &|path| {
            std::os::unix::fs::symlink("/etc/passwd", path).unwrap();
        }),
        ("socket", &|path| {
            UnixListener::bind(path).unwrap();
        }),
    ];
    for (id, planted) in plant {
        planted(&temp(id));
        let size = put_upload(&f, id, "hello");
        let (name, _) = place(&f, id, size, &format!("{id}.txt")).unwrap();
        assert_eq!(
            fs::read_to_string(transfers.join(name)).unwrap(),
            "hello",
            "{id}"
        );
        f.shares.remove_upload_temp(id).unwrap();
        assert!(!temp(id).exists(), "{id}");

        // 清理走的也是同一条路。
        planted(&temp(id));
        f.shares.remove_upload_temp(id).unwrap();
        assert!(fs::symlink_metadata(temp(id)).is_err(), "{id}");
    }
    assert!(fs::metadata("/etc/passwd").is_ok());
}

/// 别人的目录被改名到临时名字上：一个文件都不能删，改个看得见的名字交还给主人，上传照常放。
#[test]
fn a_directory_moved_onto_a_temporary_name_is_handed_back_whole() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    let temp = transfers.join(format!("{UPLOAD_TEMP_PREFIX}{ID}"));
    let visible = format!("wonton-upload-{ID}");
    let plant = || {
        fs::create_dir_all(temp.join("deep")).unwrap();
        fs::write(temp.join("deep/album.jpg"), "the owner's").unwrap();
    };
    plant();
    let size = put_upload(&f, ID, "hello");
    place(&f, ID, size, "a.txt").unwrap();
    assert_eq!(
        fs::read_to_string(transfers.join(&visible).join("deep/album.jpg")).unwrap(),
        "the owner's"
    );
    f.shares.remove_upload_temp(ID).unwrap();

    // 清理走的也是同一条路，名字被占就加序号。
    plant();
    f.shares.remove_upload_temp(ID).unwrap();
    assert!(!temp.exists());
    assert_eq!(
        fs::read_to_string(
            transfers
                .join(format!("{visible} (1)"))
                .join("deep/album.jpg")
        )
        .unwrap(),
        "the owner's"
    );
}

/// 核对之后、删之前名字被换掉了：不删换上来的文件。
#[test]
fn deleting_what_was_swapped_in_at_the_last_moment_puts_it_back() {
    let f = fixture();
    f.shares.prepare_removal().unwrap();
    let dir_path = f.root.join(TRANSFERS);
    fs::write(dir_path.join("a.jpg"), "photo").unwrap();
    let target = reference("transfers/a.jpg");
    let original = f.shares.open(&target).unwrap().snapshot;
    fs::remove_file(dir_path.join("a.jpg")).unwrap();
    fs::write(dir_path.join("a.jpg"), "the owner's new file").unwrap();
    let dir = f.shares.open_dir(TRANSFERS, &[]).unwrap();
    let dir = dir.as_fd();
    assert_eq!(
        removal::remove_if_same(dir, dir, "a.jpg", &original, &any_langs()).unwrap(),
        Deleted::Changed
    );
    assert_eq!(visible_in(&dir_path), ["a.jpg"]);
    assert_eq!(
        fs::read_to_string(dir_path.join("a.jpg")).unwrap(),
        "the owner's new file"
    );

    // 换成了目录也放回去。
    fs::remove_file(dir_path.join("a.jpg")).unwrap();
    fs::create_dir(dir_path.join("a.jpg")).unwrap();
    assert_eq!(
        removal::remove_if_same(dir, dir, "a.jpg", &original, &any_langs()).unwrap(),
        Deleted::Changed
    );
    assert!(dir_path.join("a.jpg").is_dir());

    // 放回去时原来的名字又有了文件：谁都不盖。
    fs::remove_dir(dir_path.join("a.jpg")).unwrap();
    fs::write(dir_path.join("moved"), "moved aside").unwrap();
    fs::write(dir_path.join("a.jpg"), "newer").unwrap();
    assert_eq!(
        removal::put_back(dir, "moved", dir, "a.jpg", &any_langs()).unwrap(),
        "a (1).jpg"
    );
    assert_eq!(visible_in(&dir_path), ["a (1).jpg", "a.jpg"]);
    assert_eq!(
        fs::read_to_string(dir_path.join("a (1).jpg")).unwrap(),
        "moved aside"
    );

    assert_eq!(
        removal::remove_if_same(dir, dir, "gone.jpg", &original, &any_langs()).unwrap(),
        Deleted::AlreadyGone
    );
}

/// 一个放好的文件，以及删它要用的引用和样子。
fn deletable(f: &Fixture) -> (ShareRef, Snapshot) {
    f.shares.prepare_removal().unwrap();
    let transfers = f.root.join(TRANSFERS);
    fs::write(transfers.join("a.txt"), "photo").unwrap();
    let reference = reference("transfers/a.txt");
    let snapshot = f.shares.open(&reference).unwrap().snapshot;
    (reference, snapshot)
}

fn mode(path: &std::path::Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// 有写权限的插件把 `.wonton-trash` 改名，再建一套自己的：核心不往里放东西，删除报明确的原因，重启后拿回来，假的那套交还给主人。
#[test]
fn a_trash_swapped_for_another_is_not_used_and_is_replaced_at_the_next_start() {
    let f = fixture();
    let (reference, snapshot) = deletable(&f);
    let transfers = f.root.join(TRANSFERS);
    fs::rename(transfers.join(TRASH), transfers.join("moved")).unwrap();
    for slot in 0..8 {
        fs::create_dir_all(transfers.join(TRASH).join(slot.to_string())).unwrap();
    }
    fs::write(transfers.join(TRASH).join("0/planted"), "x").unwrap();
    let refused = f
        .shares
        .delete_if_unchanged(&reference, &snapshot, &any_langs());
    assert!(matches!(refused, Err(ShareError::Io(_))), "{refused:?}");
    assert_eq!(
        fs::read_to_string(transfers.join("a.txt")).unwrap(),
        "photo"
    );

    f.shares.prepare_removal().unwrap();
    assert_eq!(
        visible_in(&transfers),
        ["a.txt", "moved", "planted"],
        "假的那套里的东西放回 transfers，空了的目录不留"
    );
    assert_eq!(
        f.shares
            .delete_if_unchanged(&reference, &snapshot, &any_langs())
            .unwrap(),
        Deleted::Removed { durable: true }
    );
}

/// NAS 的“重置权限”把回收槽改得不像核心建的，槽里还有删除到一半的文件：先放回原名，再重建，否则它就永远回不来了。
#[test]
fn a_half_deleted_file_goes_back_even_when_its_slot_no_longer_looks_ours() {
    let f = fixture();
    deletable(&f);
    let transfers = f.root.join(TRANSFERS);
    let slot = transfers.join(TRASH).join("3");
    fs::rename(transfers.join("a.txt"), slot.join("a.txt")).unwrap();
    mode(&transfers.join(TRASH), 0o777);
    mode(&slot, 0o777);

    f.shares.prepare_removal().unwrap();
    assert_eq!(
        fs::read_to_string(transfers.join("a.txt")).unwrap(),
        "photo"
    );
    assert_eq!(
        visible_in(&transfers),
        ["a.txt"],
        "放回去之后没有东西要交还"
    );
    assert!(slot.is_dir(), "回收槽重建了");
}

/// 打不开的东西不猜，跟着目录交还给主人。
#[test]
fn what_cannot_be_put_back_is_returned_with_the_directory() {
    let f = fixture();
    deletable(&f);
    let transfers = f.root.join(TRANSFERS);
    let slot = transfers.join(TRASH).join("3");
    fs::remove_dir_all(&slot).unwrap();
    fs::write(&slot, "x").unwrap();
    mode(&transfers.join(TRASH), 0o777);

    f.shares.prepare_removal().unwrap();
    assert_eq!(visible_in(&transfers), ["a.txt", "wonton-trash"]);
    assert_eq!(
        fs::read_to_string(transfers.join("wonton-trash/3")).unwrap(),
        "x"
    );
}

/// 里面只有一个槽不对也一样：换过的槽可能在任何位置。
#[test]
fn one_slot_that_is_not_private_stops_deletes() {
    let f = fixture();
    let (reference, snapshot) = deletable(&f);
    let transfers = f.root.join(TRANSFERS);
    mode(&transfers.join(TRASH).join("0"), 0o755);
    let refused = f
        .shares
        .delete_if_unchanged(&reference, &snapshot, &any_langs());
    assert!(matches!(refused, Err(ShareError::Io(_))), "{refused:?}");
    assert!(transfers.join("a.txt").exists());
    f.shares.prepare_removal().unwrap();
    assert_eq!(
        fs::metadata(transfers.join(TRASH).join("0"))
            .unwrap()
            .mode()
            & 0o777,
        0o700
    );
}

/// 属主不对也算：测试不以 root 运行，建不出别人的目录。
#[test]
fn a_directory_owned_by_someone_else_is_not_ours() {
    let f = fixture();
    f.shares.prepare_removal().unwrap();
    let dir = File::open(f.root.join(TRANSFERS).join(TRASH)).unwrap();
    let stat = rustix::fs::fstat(&dir).unwrap();
    let euid = rustix::process::geteuid().as_raw();
    assert!(removal::is_ours(&stat, euid));
    assert!(!removal::is_ours(&stat, euid + 1));
}

/// 运行中不悄悄重建：懒建出来的槽没有占位文件，磁盘写满时又改不进长名字。
#[test]
fn a_missing_trash_is_reported_and_not_rebuilt_while_running() {
    let f = fixture();
    let (reference, snapshot) = deletable(&f);
    let transfers = f.root.join(TRANSFERS);
    fs::remove_dir_all(transfers.join(TRASH)).unwrap();
    let refused = f
        .shares
        .delete_if_unchanged(&reference, &snapshot, &any_langs());
    assert!(matches!(refused, Err(ShareError::Io(_))), "{refused:?}");
    assert!(transfers.join("a.txt").exists());
    assert!(!transfers.join(TRASH).exists());
}

#[test]
fn a_slot_without_its_placeholder_is_reported() {
    let f = fixture();
    let (reference, snapshot) = deletable(&f);
    let transfers = f.root.join(TRANSFERS);
    fs::remove_file(transfers.join(TRASH).join("0").join(removal::keep_name())).unwrap();
    let refused = f
        .shares
        .delete_if_unchanged(&reference, &snapshot, &any_langs());
    assert!(matches!(refused, Err(ShareError::Io(_))), "{refused:?}");
    assert!(transfers.join("a.txt").exists());
    f.shares.prepare_removal().unwrap();
    assert!(
        transfers
            .join(TRASH)
            .join("0")
            .join(removal::keep_name())
            .exists()
    );
}

/// 文件已经删了，目录没能确认落盘：不能报“没删”，否则审计和记录都错。
#[test]
fn a_delete_that_cannot_be_confirmed_on_disk_still_counts_as_removed() {
    let f = fixture();
    let (reference, snapshot) = deletable(&f);
    super::DIR_SYNC_FAILS.set(Some("a.txt".to_owned()));
    let deleted = f
        .shares
        .delete_if_unchanged(&reference, &snapshot, &any_langs());
    super::DIR_SYNC_FAILS.set(None);
    assert_eq!(deleted.unwrap(), Deleted::Removed { durable: false });
    assert!(!f.root.join(TRANSFERS).join("a.txt").exists());
}

/// 回收槽和没放好的上传在 0700 或者私有名字后面，核心可能以 root 打开：外面给的路径到不了。
#[test]
fn private_names_cannot_be_reached_from_outside() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    left_halfway(&transfers, 3, "合同.pdf", "secret");
    fs::write(transfers.join(".wonton-upload-abc"), "half").unwrap();
    fs::write(transfers.join(".WONTON-other"), "x").unwrap();
    for path in [
        "transfers/.wonton-trash/3/合同.pdf",
        "transfers/.WONTON-TRASH/3/合同.pdf",
        "transfers/.wonton-upload-abc",
        "transfers/.WONTON-other",
        ".wonton-trash/a.txt",
    ] {
        let refused = f.shares.open(&reference(path));
        assert!(matches!(refused, Err(ShareError::Refused(_))), "{path}");
    }
    let refused = f.shares.open_dir(TRANSFERS, &[".wonton-trash".to_owned()]);
    assert!(matches!(refused, Err(ShareError::Refused(_))));
}
