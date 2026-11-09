use super::*;

/// 起不了处理图片的进程不是图标的问题：下次照样处理。图标本身处理不了的结果记住，不再起进程。
#[tokio::test]
async fn only_failures_of_the_icon_itself_are_remembered() {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("wonton-core");
    let gallery = Gallery::new(ImageWorker::new(&program));
    let source = IconSource {
        path: "icon.svg".to_owned(),
        sha256: "0".repeat(64),
        format: IconFormat::Svg,
        data: Arc::new(b"<svg/>".to_vec()),
    };
    let unavailable = gallery.render(&source).await;
    assert!(
        matches!(unavailable, Err(ImageError::Unavailable(_))),
        "{unavailable:?}"
    );

    // 换成一个总说“不合规格”的程序（退出码 1）：这次真的处理了。
    std::os::unix::fs::symlink("/usr/bin/false", &program).unwrap();
    let refused = gallery.render(&source).await;
    assert!(
        matches!(refused, Err(ImageError::Refused(_))),
        "{refused:?}"
    );

    std::fs::remove_file(&program).unwrap();
    assert_eq!(gallery.render(&source).await, refused);
}
