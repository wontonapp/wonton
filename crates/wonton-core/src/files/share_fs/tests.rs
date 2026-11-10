use std::fs;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::PathBuf;

use super::placing::{link_checked, link_to_temp};
use super::removal::TRASH;
use super::*;
use crate::files::test_support::any_langs;
use crate::plugin::hostdir::Owner;

mod removal_tests;

thread_local! {
    /// 这个线程上的目录 fsync 都失败。
    /// 这个名字引起的目录落盘都失败。
    pub(in crate::files) static DIR_SYNC_FAILS: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// 这个线程上 `RENAME_NOREPLACE` 不被支持，像 NFS、CIFS。
    pub(super) static NO_NOREPLACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// 这个线程上跨目录改名回 `EXDEV`，像 mergerfs 的两个分支。
    pub(super) static EXDEV: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 数据不在 `transfers` 所在的卷上：链接临时文件回 `XDEV`。要跨线程，不是 `thread_local`。
pub(in crate::files) static CROSS_VOLUME: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// 这个上传链接临时文件时失败一次（名字已经写进待完成记录之后）。
pub(in crate::files) static LINK_FAILS: std::sync::Mutex<Option<String>> =
    std::sync::Mutex::new(None);

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    shares: Shares,
    owner: Owner,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("shares");
    fs::create_dir_all(root.join("photos/2025")).unwrap();
    fs::create_dir_all(root.join(TRANSFERS)).unwrap();
    fs::write(root.join("photos/2025/a.jpg"), "photo").unwrap();
    let meta = fs::metadata(&root).unwrap();
    let owner = Owner {
        uid: meta.uid(),
        gid: meta.gid(),
    };
    Fixture {
        shares: Shares::new(root.clone()),
        root,
        owner,
        _dir: dir,
    }
}

fn reference(path: &str) -> ShareRef {
    ShareRef::parse(&format!("wonton-share:{path}")).unwrap()
}

fn read(opened: Opened) -> String {
    let mut text = String::new();
    let mut file = opened.file;
    file.read_to_string(&mut text).unwrap();
    text
}

#[test]
fn references_split_into_share_and_path() {
    let parsed = ShareRef::parse("wonton-share:photos/2025/07/IMG%201234.jpg").unwrap();
    assert_eq!(parsed.share, "photos");
    assert_eq!(parsed.path, ["2025", "07", "IMG 1234.jpg"]);
    assert_eq!(parsed.file_name(), "IMG 1234.jpg");
}

#[test]
fn references_that_could_escape_the_share_are_refused() {
    for uri in [
        "wonton-share:/etc/passwd",
        "wonton-share:photos/../secrets/a",
        "wonton-share:photos/./a",
        "wonton-share:photos//a",
        "wonton-share:photos/a/",
        "wonton-share:photos",
        "wonton-share:..",
        "wonton-share:photos/%2E%2E/a",
        "wonton-share:photos/%2e%2e%2fetc%2fpasswd",
        "wonton-share:photos/a%2Fb",
        "wonton-share:photos/a%00b",
        "wonton-share:photos/%FF",
        "file:///etc/passwd",
        "https://example.com/a",
    ] {
        assert!(ShareRef::parse(uri).is_err(), "{uri}");
    }
}

#[test]
fn opens_regular_files_with_their_snapshot() {
    let f = fixture();
    let opened = f.shares.open(&reference("photos/2025/a.jpg")).unwrap();
    assert_eq!(opened.snapshot.size, 5);
    assert_eq!(read(opened), "photo");
}

#[test]
fn a_symlink_anywhere_on_the_path_is_refused() {
    let f = fixture();
    let outside = f.root.parent().unwrap().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("secret"), "secret").unwrap();
    // 有写权限的插件把子目录换成软链接，或者放一个指向外面的文件软链接。
    symlink(&outside, f.root.join("photos/link")).unwrap();
    symlink(outside.join("secret"), f.root.join("photos/file")).unwrap();
    // 共享文件夹自己也可能是软链接（比如主人上次手工留下的）。
    symlink(&outside, f.root.join("linked")).unwrap();
    for path in ["photos/link/secret", "photos/file", "linked/secret"] {
        let err = f.shares.open(&reference(path)).unwrap_err();
        assert!(matches!(err, ShareError::Refused(_)), "{path}: {err}");
    }
}

#[test]
fn only_regular_files_open() {
    let f = fixture();
    let err = f.shares.open(&reference("photos/2025")).unwrap_err();
    assert!(matches!(err, ShareError::Refused(_)), "{err}");
    let fifo = f.root.join("photos/fifo");
    rustix::fs::mknodat(
        CWD,
        &fifo,
        rustix::fs::FileType::Fifo,
        Mode::from_raw_mode(0o644),
        0,
    )
    .unwrap();
    // 不能卡在等写端上。
    let err = f.shares.open(&reference("photos/fifo")).unwrap_err();
    assert!(matches!(err, ShareError::Refused(_)), "{err}");
}

#[test]
fn missing_files_and_directories_are_gone() {
    let f = fixture();
    for path in ["photos/2025/b.jpg", "photos/2024/a.jpg", "missing/a.jpg"] {
        let err = f.shares.open(&reference(path)).unwrap_err();
        assert!(matches!(err, ShareError::Gone), "{path}: {err}");
    }
    // 上级目录被换成了文件。
    let err = f
        .shares
        .open(&reference("photos/2025/a.jpg/x"))
        .unwrap_err();
    assert!(matches!(err, ShareError::Gone), "{err}");
}

#[test]
fn delete_checks_the_file_is_still_the_same_one() {
    let f = fixture();
    f.shares.prepare_removal().unwrap();
    let path = f.root.join("transfers/a.jpg");
    fs::write(&path, "photo").unwrap();
    let target = reference("transfers/a.jpg");
    let original = f.shares.open(&target).unwrap().snapshot;

    // 同一路径换成了主人别的文件：不动。
    fs::remove_file(&path).unwrap();
    fs::write(&path, "another").unwrap();
    assert_eq!(
        f.shares
            .delete_if_unchanged(&target, &original, &any_langs())
            .unwrap(),
        Deleted::Changed
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "another");

    let current = f.shares.open(&target).unwrap().snapshot;
    assert_eq!(
        f.shares
            .delete_if_unchanged(&target, &current, &any_langs())
            .unwrap(),
        Deleted::Removed { durable: true }
    );
    assert!(!path.exists());
    assert_eq!(
        f.shares
            .delete_if_unchanged(&target, &current, &any_langs())
            .unwrap(),
        Deleted::AlreadyGone
    );
}

#[test]
fn delete_does_not_follow_a_symlink_put_in_its_place() {
    let f = fixture();
    let path = f.root.join("transfers/a.jpg");
    fs::write(&path, "photo").unwrap();
    let target = reference("transfers/a.jpg");
    let original = f.shares.open(&target).unwrap().snapshot;
    let elsewhere = f.root.join("photos/kept");
    fs::rename(&path, &elsewhere).unwrap();
    symlink(&elsewhere, &path).unwrap();
    assert_eq!(
        f.shares
            .delete_if_unchanged(&target, &original, &any_langs())
            .unwrap(),
        Deleted::Changed
    );
    assert!(elsewhere.exists());
    assert!(fs::symlink_metadata(&path).unwrap().is_symlink());
}

/// 核心只删设备上传的文件，别的共享文件夹没有回收槽。
#[test]
fn only_files_in_transfers_can_be_deleted() {
    let f = fixture();
    let target = reference("photos/2025/a.jpg");
    let original = f.shares.open(&target).unwrap().snapshot;
    let refused = f
        .shares
        .delete_if_unchanged(&target, &original, &any_langs());
    assert!(
        matches!(refused, Err(ShareError::Refused(_))),
        "{refused:?}"
    );
    assert!(f.root.join("photos/2025/a.jpg").exists());
}

const ID: &str = "aaaqeayeaudaocajbifqydiob4";
const OTHER_ID: &str = "77777777777777777777777774";

fn uploads(f: &Fixture) -> PathBuf {
    let path = f.root.parent().unwrap().join("uploads");
    fs::create_dir_all(&path).unwrap();
    path
}

fn put_upload(f: &Fixture, id: &str, text: &str) -> u64 {
    fs::write(uploads(f).join(id), text).unwrap();
    text.len() as u64
}

fn place(f: &Fixture, id: &str, size: u64, name: &str) -> Result<(String, Snapshot), ShareError> {
    f.shares.place_upload(
        &uploads(f),
        id,
        size,
        Names {
            preferred: name,
            owner: f.owner,
            placing_inode: None,
        },
        &any_langs(),
        &mut |_, _| Ok(()),
    )
}

fn names_in(path: &std::path::Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// 回收槽里的东西，`槽/名字`，占位文件除外。
fn trash_contents(transfers: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    for slot in fs::read_dir(transfers.join(TRASH)).unwrap() {
        let slot = slot.unwrap();
        for entry in fs::read_dir(slot.path()).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            if name != removal::keep_name() {
                found.push(format!(
                    "{}/{name}",
                    slot.file_name().into_string().unwrap()
                ));
            }
        }
    }
    found.sort();
    found
}

/// 主人看得到的名字：核心的临时文件留到写好记录之后才删。
fn visible_in(path: &std::path::Path) -> Vec<String> {
    let mut names = names_in(path);
    names.retain(|name| !name.starts_with(PRIVATE_PREFIX));
    names
}

#[test]
fn placing_links_the_data_and_numbers_duplicates() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    let size = put_upload(&f, ID, "one");
    let (first, _) = place(&f, ID, size, "a.txt").unwrap();
    let size = put_upload(&f, OTHER_ID, "two");
    let (second, snapshot) = place(&f, OTHER_ID, size, "a.txt").unwrap();
    assert_eq!((first.as_str(), second.as_str()), ("a.txt", "a (1).txt"));
    assert_eq!(snapshot.size, 3);
    assert_eq!(fs::read_to_string(transfers.join("a.txt")).unwrap(), "one");
    assert_eq!(
        fs::read_to_string(transfers.join("a (1).txt")).unwrap(),
        "two"
    );
    assert!(uploads(&f).join(ID).exists(), "写好记录之后才删数据");
}

/// 放好了但记录没写成（磁盘满、崩溃）：再放一次还是那个文件，不多出一份。
#[test]
fn placing_again_finds_what_was_already_placed() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    let size = put_upload(&f, ID, "one");
    let (name, snapshot) = place(&f, ID, size, "a.txt").unwrap();
    assert_eq!(place(&f, ID, size, "a.txt").unwrap(), (name, snapshot));
    assert_eq!(visible_in(&transfers), ["a.txt"]);

    // 这期间主人改了名：认得出来。
    fs::rename(transfers.join("a.txt"), transfers.join("b.txt")).unwrap();
    assert_eq!(place(&f, ID, size, "a.txt").unwrap().0, "b.txt");
    assert_eq!(visible_in(&transfers), ["b.txt"]);

    // 删掉了：算没放过。
    fs::remove_file(transfers.join("b.txt")).unwrap();
    assert_eq!(place(&f, ID, size, "a.txt").unwrap().0, "a.txt");

    f.shares.remove_upload_temp(ID).unwrap();
    assert_eq!(names_in(&transfers), ["a.txt"]);
}

#[test]
fn placing_never_replaces_what_the_owner_has() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    fs::write(transfers.join("a.txt"), "the owner's").unwrap();
    let size = put_upload(&f, ID, "new");
    assert_eq!(place(&f, ID, size, "a.txt").unwrap().0, "a (1).txt");
    assert_eq!(
        fs::read_to_string(transfers.join("a.txt")).unwrap(),
        "the owner's"
    );
}

#[test]
fn placing_does_not_write_through_a_symlink_with_the_same_name() {
    let f = fixture();
    let outside = f.root.parent().unwrap().join("outside");
    fs::write(&outside, "keep").unwrap();
    // 有写权限的插件在 transfers 里放好软链接，等主人上传同名文件。
    symlink(&outside, f.root.join(TRANSFERS).join("a.txt")).unwrap();
    let size = put_upload(&f, ID, "new");
    assert_eq!(place(&f, ID, size, "a.txt").unwrap().0, "a (1).txt");
    assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
}

#[test]
fn placing_refuses_a_transfers_folder_that_is_a_symlink() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    fs::remove_dir(&transfers).unwrap();
    let outside = f.root.parent().unwrap().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, &transfers).unwrap();
    let size = put_upload(&f, ID, "new");
    let err = place(&f, ID, size, "a.txt").unwrap_err();
    assert!(matches!(err, ShareError::Refused(_)), "{err}");
    assert!(
        uploads(&f).join(ID).exists(),
        "放不进去时文件还留着，之后还能再放"
    );
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
}

/// tusd 的目录归 `PUID`：数据被换成软链接时，以 root 运行的核心不能把链接指向的文件放进 `transfers`。
#[test]
fn placing_refuses_data_that_is_not_a_regular_file_of_the_declared_size() {
    let f = fixture();
    let secret = f.root.parent().unwrap().join("matrix-jwt.pem");
    fs::write(&secret, "secret").unwrap();
    symlink(&secret, uploads(&f).join(ID)).unwrap();
    let err = place(&f, ID, 6, "a.txt").unwrap_err();
    assert!(matches!(err, ShareError::Refused(_)), "{err}");

    let size = put_upload(&f, OTHER_ID, "short");
    let err = place(&f, OTHER_ID, size + 1, "a.txt").unwrap_err();
    assert!(matches!(err, ShareError::Io(_)), "{err}");

    let err = place(&f, "77777777777777777777777775", 0, "a.txt").unwrap_err();
    assert!(matches!(err, ShareError::Gone), "{err}");
    assert!(names_in(&f.root.join(TRANSFERS)).is_empty());
}

fn copy_upload(f: &Fixture, text: &str, name: &str) -> (String, Snapshot) {
    copy_again(f, text, name, None)
}

/// 现在的临时文件的 inode。
fn temp_inode(f: &Fixture) -> Option<u64> {
    let temp = format!("{UPLOAD_TEMP_PREFIX}{ID}");
    Some(
        fs::metadata(f.root.join(TRANSFERS).join(temp))
            .unwrap()
            .ino(),
    )
}

/// `placing_inode`：上次走到链接这一步时记下的 inode。
fn copy_again(
    f: &Fixture,
    text: &str,
    name: &str,
    placing_inode: Option<u64>,
) -> (String, Snapshot) {
    let size = put_upload(f, ID, text);
    let source = open_regular(open_absolute(&uploads(f), "上传目录").unwrap().as_fd(), ID).unwrap();
    assert_eq!(source.snapshot.size, size);
    let dir = f.shares.open_dir(TRANSFERS, &[]).unwrap();
    let names = Names {
        preferred: name,
        owner: f.owner,
        placing_inode,
    };
    placing::through_temp(
        &source,
        ID,
        dir.as_fd(),
        names,
        &any_langs(),
        &mut |_, _| Ok(()),
        |_| Ok(None),
    )
    .unwrap()
}

/// 跨卷的路径：单元测试里造不出第二个卷，让链接这一步报不在同一个卷上（ADR-0020 的“验证结果”有两个真卷上的结果）。
#[test]
fn copying_across_volumes_converges_and_leaves_nothing_behind() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    let temp = format!("{UPLOAD_TEMP_PREFIX}{ID}");
    // 上次崩溃留下的半个文件不影响这次。
    fs::write(transfers.join(&temp), "half").unwrap();
    let (name, snapshot) = copy_upload(&f, "content", "big.bin");
    assert_eq!(name, "big.bin");
    assert_eq!(snapshot.size, 7);
    assert_eq!(
        fs::read_to_string(transfers.join("big.bin")).unwrap(),
        "content"
    );

    // 放好了但记录没写成：再来一次认得出临时文件放好的那个，不多复制一份。
    let placing = temp_inode(&f);
    assert_eq!(
        copy_again(&f, "content", "big.bin", placing),
        (name, snapshot)
    );
    assert_eq!(names_in(&transfers), [temp.as_str(), "big.bin"]);

    f.shares.remove_upload_temp(ID).unwrap();
    f.shares.remove_upload_temp(ID).unwrap();
    assert_eq!(names_in(&transfers), ["big.bin"]);
}

/// 放好之后主人改了内容，大小和声明的不一样了：还是放好的那个文件，不再复制成 `big (1).bin`。同一个卷上的路径也是这样。
#[test]
fn a_placed_copy_the_owner_edited_is_not_copied_again() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    copy_upload(&f, "content", "big.bin");
    fs::write(transfers.join("big.bin"), "content, and more").unwrap();
    let placing = temp_inode(&f);
    let (name, snapshot) = copy_again(&f, "content", "big.bin", placing);
    assert_eq!((name.as_str(), snapshot.size), ("big.bin", 17));
    assert_eq!(visible_in(&transfers), ["big.bin"]);
}

/// 没走到链接这一步的临时文件有别的硬链接（插件能链接它）：那不是放好的文件，不能登记成完整的。
#[test]
fn a_short_temporary_file_with_another_link_does_not_count_as_placed() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    let temp = transfers.join(format!("{UPLOAD_TEMP_PREFIX}{ID}"));
    fs::write(&temp, "half").unwrap();
    fs::hard_link(&temp, transfers.join("other")).unwrap();
    let (name, snapshot) = copy_upload(&f, "content", "big.bin");
    assert_eq!((name.as_str(), snapshot.size), ("big.bin", 7));
    assert_eq!(fs::read_to_string(transfers.join("other")).unwrap(), "half");
}

/// 上次的临时文件被删了，插件自己建了一个，再硬链接到公开的名字上：不是记下的那个 inode，不能算放好了。
#[test]
fn a_temporary_file_someone_planted_does_not_count_as_placed() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    let temp = transfers.join(format!("{UPLOAD_TEMP_PREFIX}{ID}"));
    fs::write(&temp, "planted").unwrap();
    fs::hard_link(&temp, transfers.join("invoice.pdf")).unwrap();
    let recorded = temp_inode(&f).map(|inode| inode + 1);
    let (name, snapshot) = copy_again(&f, "content", "big.bin", recorded);
    assert_eq!((name.as_str(), snapshot.size), ("big.bin", 7));
    assert_eq!(
        fs::read_to_string(transfers.join("big.bin")).unwrap(),
        "content"
    );
}

/// 临时文件名由上传编号派生，和文件名无关。
#[test]
fn the_temporary_name_does_not_depend_on_the_file_name() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    let owners = format!("{UPLOAD_TEMP_PREFIX}a.txt");
    fs::write(transfers.join(&owners), "the owner's").unwrap();
    assert_eq!(copy_upload(&f, "content", "a.txt").0, "a.txt");
    f.shares.remove_upload_temp(ID).unwrap();
    assert_eq!(
        fs::read_to_string(transfers.join(&owners)).unwrap(),
        "the owner's"
    );

    let long = sanitize_name(&format!("{}.bin", "长".repeat(200)));
    assert!(long.len() > 230);
    fs::write(transfers.join(&long), "taken").unwrap();
    let (name, _) = copy_upload(&f, "other content", &long);
    assert!(name.contains(" (1)"), "{name}");
    assert!(name.len() <= 255);
}

/// 设备声明的名字不能占另一个上传的临时文件，否则那个上传结束时会把它当临时文件删掉。
#[test]
fn a_name_from_a_device_cannot_take_a_private_name() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    let claimed = sanitize_name(&format!("{UPLOAD_TEMP_PREFIX}{OTHER_ID}"));
    assert!(!claimed.starts_with(PRIVATE_PREFIX), "{claimed}");
    let size = put_upload(&f, ID, "mine");
    assert_eq!(place(&f, ID, size, &claimed).unwrap().0, claimed);
    f.shares.remove_upload_temp(OTHER_ID).unwrap();
    assert_eq!(
        fs::read_to_string(transfers.join(&claimed)).unwrap(),
        "mine"
    );
    assert_eq!(sanitize_name(".WONTON-delete-abc"), "WONTON-delete-abc");
    // 别的点开头的名字不受影响。
    assert_eq!(sanitize_name(".wontonrc"), ".wontonrc");
}

/// 链接之后、核对之前，别人（Samba 另存、插件）把自己的文件改名到了这个名字上：报错，不删它。
#[test]
fn a_file_swapped_onto_the_target_name_is_left_alone() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    let dir = f.shares.open_dir(TRANSFERS, &[]).unwrap();
    fs::write(transfers.join("theirs"), "theirs").unwrap();
    fs::write(transfers.join("ours"), "ours").unwrap();
    let ours = fs::File::open(transfers.join("ours")).unwrap();
    // 把“名字上已经是别的文件”造出来：链接的是别的文件，核对的是 `ours`。
    let err = link_checked(
        dir.as_fd(),
        "theirs",
        "a.txt",
        &ours,
        &any_langs(),
        &mut |_, _| Ok(()),
    )
    .unwrap_err();
    assert!(matches!(err, ShareError::Refused(_)), "{err}");
    assert_eq!(
        fs::read_to_string(transfers.join("a.txt")).unwrap(),
        "theirs"
    );
}

/// 打开之后、链接之前，数据被换了（tusd 的目录归 `PUID`）：换上来的文件进不了 `transfers`，原地也不动。
#[test]
fn data_swapped_after_opening_is_not_placed() {
    let f = fixture();
    let transfers = f.root.join(TRANSFERS);
    put_upload(&f, ID, "real");
    let data_dir = open_absolute(&uploads(&f), "上传目录").unwrap();
    let source = open_regular(data_dir.as_fd(), ID).unwrap();
    fs::remove_file(uploads(&f).join(ID)).unwrap();
    fs::write(uploads(&f).join(ID), "swapped").unwrap();
    let dir = f.shares.open_dir(TRANSFERS, &[]).unwrap();
    let temp = format!("{UPLOAD_TEMP_PREFIX}{ID}");
    let err = link_to_temp(
        data_dir.as_fd(),
        ID,
        dir.as_fd(),
        &temp,
        &source.file,
        &any_langs(),
    )
    .unwrap_err();
    assert!(matches!(err, ShareError::Refused(_)), "{err}");
    assert!(names_in(&transfers).is_empty());
    assert_eq!(fs::read_to_string(uploads(&f).join(ID)).unwrap(), "swapped");
}

/// 不打开就拒绝：打开 socket 报的是 `ENXIO`，类别就错了；设备、管道打开本身有副作用。
#[test]
fn sockets_are_refused_without_opening_them() {
    let f = fixture();
    let socket = f.root.join("photos/socket");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let err = f.shares.open(&reference("photos/socket")).unwrap_err();
    assert!(matches!(err, ShareError::Refused(_)), "{err}");
    let err = f.shares.open(&reference("photos/socket/x")).unwrap_err();
    assert!(matches!(err, ShareError::Gone), "{err}");
}

#[test]
fn names_from_clients_are_cleaned_up() {
    assert_eq!(sanitize_name("../../etc/passwd"), "passwd");
    assert_eq!(sanitize_name("C:\\Users\\me\\a.txt"), "a.txt");
    assert_eq!(sanitize_name("a\u{0}b\nc.txt"), "abc.txt");
    assert_eq!(sanitize_name(""), "file");
    assert_eq!(sanitize_name(".."), "file");
    assert_eq!(sanitize_name("dir/"), "file");
    assert_eq!(sanitize_name("  照片.jpg "), "照片.jpg");
    let long = format!("{}.jpg", "长".repeat(200));
    let cleaned = sanitize_name(&long);
    assert!(cleaned.len() <= MAX_NAME_BYTES, "{}", cleaned.len());
    assert!(cleaned.ends_with("长.jpg"));
    // 显示时能把名字反过来的 bidi 字符、让两个名字看着一样的零宽字符。
    assert_eq!(sanitize_name("exe.\u{202E}gpj"), "exe.gpj");
    assert_eq!(sanitize_name("\u{2066}a\u{2069}.txt"), "a.txt");
    assert_eq!(sanitize_name("a\u{200B}b\u{FEFF}\u{200D}.txt"), "ab.txt");
    assert_eq!(sanitize_name("\u{202E}"), "file");
    // 标签字符、蒙古文分隔符、行间注解符、行分隔符、私有区、韩文填充字符、组合用字素连接符。
    for hidden in [
        '\u{E0041}',
        '\u{180E}',
        '\u{FFF9}',
        '\u{FFFB}',
        '\u{2028}',
        '\u{2029}',
        '\u{E000}',
        '\u{115F}',
        '\u{1160}',
        '\u{3164}',
        '\u{FFA0}',
        '\u{034F}',
    ] {
        assert_eq!(
            sanitize_name(&format!("a{hidden}b.txt")),
            "ab.txt",
            "{hidden:?}"
        );
    }
    assert_eq!(sanitize_name("\u{3164}"), "file");
    // 其余的 Default_Ignorable_Code_Point：看不见，也不属于上面的类别。
    for hidden in [
        '\u{17B4}',
        '\u{180B}',
        '\u{FE00}',
        '\u{FE0E}',
        '\u{FFF0}',
        '\u{1BCA0}',
        '\u{1D173}',
        '\u{E0100}',
    ] {
        assert_eq!(
            sanitize_name(&format!("a{hidden}b.txt")),
            "ab.txt",
            "{hidden:?}"
        );
    }
    // emoji 的样式选择符留着。
    assert_eq!(
        sanitize_name("\u{2764}\u{FE0F}.txt"),
        "\u{2764}\u{FE0F}.txt"
    );
}
