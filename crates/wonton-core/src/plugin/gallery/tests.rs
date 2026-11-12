use super::*;
use std::os::unix::fs::PermissionsExt;

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

    // 换成一个总说“不合规格”的助手：这次真的处理了。
    let helper = dir.path().join("refusing-helper.sh");
    std::fs::write(
        &helper,
        "#!/bin/sh\nprintf '0 1\\n'\nwhile read -r id timeout lang job input; do\n  head -c \"$input\" >/dev/null\n  printf '%s 1 0 4\\nnope' \"$id\"\ndone\n",
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink(&helper, &program).unwrap();
    let refused = gallery.render(&source).await;
    assert!(
        matches!(refused, Err(ImageError::Refused(_))),
        "{refused:?}"
    );

    std::fs::remove_file(&program).unwrap();
    assert_eq!(gallery.render(&source).await, refused);
}
