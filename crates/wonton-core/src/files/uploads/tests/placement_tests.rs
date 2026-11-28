//! 放文件：名字和样子先落盘再链接；跨卷复制失败要忘掉临时文件的 inode。

use super::*;

/// 名字和样子要在链接之前记下：链接之后崩溃，记录里已经有了。重名时记的是真正要放的那个名字。
#[tokio::test]
async fn the_name_and_look_are_written_before_the_link() {
    let f = Fixture::new();
    let id = f
        .files
        .create_upload(new_upload(5), &any_langs())
        .await
        .unwrap();
    fs::write(f.uploads_dir().join(id.as_str()), "hello").unwrap();
    let transfers = f.home_dir().join(TRANSFERS);
    fs::create_dir_all(&transfers).unwrap();
    fs::write(transfers.join("a.txt"), "the owner's").unwrap();

    let mut seen = Vec::new();
    let (name, placed) = f
        .files
        .home_shares("owner", false)
        .unwrap()
        .place_upload(
            &f.files.uploads.dir,
            id.as_str(),
            5,
            Names {
                preferred: "a.txt",
                owner: f.files.uploads.owner,
                placing_inode: None,
            },
            &any_langs(),
            &mut |name, snapshot| {
                assert!(!transfers.join(name).exists(), "链接之前");
                seen.push((name.to_owned(), *snapshot));
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(seen, [(name.clone(), placed)]);
    assert_eq!(name, "a (1).txt");

    // 经 `place_marked` 走的话，记在待完成记录里。
    let id = f
        .files
        .create_upload(new_upload(5), &any_langs())
        .await
        .unwrap();
    fs::write(f.uploads_dir().join(id.as_str()), "hello").unwrap();
    let pending = f.files.uploads.load_pending(&id).unwrap().unwrap();
    let (name, placed) = f.files.place_marked(&pending, &any_langs()).unwrap();
    let pending = f.files.uploads.load_pending(&id).unwrap().unwrap();
    assert_eq!(
        pending.placing,
        Some(Placing {
            name,
            snapshot: placed
        })
    );
}

/// 名字和样子没落盘就链接的话，断电后分不清名字上的文件是不是这个上传放的。
#[tokio::test]
async fn nothing_is_linked_until_the_name_and_look_are_durable() {
    let f = Fixture::new();
    let id = f
        .files
        .create_upload(new_upload(5), &any_langs())
        .await
        .unwrap();
    fs::write(f.uploads_dir().join(id.as_str()), "hello").unwrap();
    fail_dir_sync(f.dir.path().join("files/pending"), 1);
    assert!(f.files.finish_upload(&id, &any_langs()).await.is_err());
    assert!(!f.home_dir().join(TRANSFERS).join("a.txt").exists());
    assert!(f.files.finish_upload(&id, &any_langs()).await.is_ok());
    assert!(f.home_dir().join(TRANSFERS).join("a.txt").exists());
}

#[test]
fn a_file_system_out_of_inodes_has_no_room() {
    assert!(out_of_inodes(100, 0));
    assert!(!out_of_inodes(100, 1));
    assert!(!out_of_inodes(0, 0), "btrfs 不报 inode 数");
}

/// 这个上传的数据“在另一个卷上”：链接临时文件回 `XDEV`，走复制。按编号收窄，别的测试照常走硬链接。
struct CrossVolume(String);

impl CrossVolume {
    fn on(id: &FileId) -> Self {
        *crate::files::share_fs::tests::CROSS_VOLUME.lock().unwrap() = Some(id.to_string());
        Self(id.to_string())
    }
}

impl Drop for CrossVolume {
    fn drop(&mut self) {
        let mut current = crate::files::share_fs::tests::CROSS_VOLUME.lock().unwrap();
        if current.as_deref() == Some(self.0.as_str()) {
            *current = None;
        }
    }
}

/// 链接在名字写进待完成记录之后才失败：临时文件删掉，记下的 inode 不能留（跨卷时会被别的文件复用）。
#[tokio::test]
async fn a_failed_placement_forgets_the_temporary_inode() {
    let f = Fixture::new();
    let id = f
        .files
        .create_upload(new_upload(5), &any_langs())
        .await
        .unwrap();
    let _volume = CrossVolume::on(&id);
    fs::write(f.uploads_dir().join(id.as_str()), "hello").unwrap();
    *crate::files::share_fs::tests::LINK_FAILS.lock().unwrap() = Some(id.to_string());
    assert!(
        f.files.place_for_test(&id, &any_langs()).await.is_err(),
        "链接失败"
    );
    let pending = f.files.uploads.load_pending(&id).unwrap().unwrap();
    assert!(
        pending.placing.is_none(),
        "临时文件已经删了，不能留着它的 inode"
    );

    let record = f.files.finish_upload(&id, &any_langs()).await.unwrap();
    assert_eq!(record.name, "a.txt");
    assert_eq!(
        fs::read_to_string(f.home_dir().join(TRANSFERS).join("a.txt")).unwrap(),
        "hello"
    );
    assert!(f.files.uploads.load_pending(&id).unwrap().is_none());
    assert!(
        !f.home_dir()
            .join(TRANSFERS)
            .join(format!("{UPLOAD_TEMP_PREFIX}{id}"))
            .exists()
    );
}
