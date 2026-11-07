//! 共享文件来源的测试：哪个插件给的引用决定来源，来源授权换插件后跟着换。

use super::BotRef;
use super::record::{Accessor, FileRecord};
use super::share_fs::ShareRef;
use super::test_support::{Fixture, share};
use crate::files::test_support::any_langs;
use crate::plugin::PluginId;
use wonton_protocol::{FileFailure, FileSource};

fn hermes() -> BotRef {
    BotRef {
        name: "hermes".into(),
        plugin: PluginId::parse("app.files").unwrap(),
    }
}

fn from_plugin(plugin: &str) -> BotRef {
    BotRef {
        name: "hermes".into(),
        plugin: PluginId::parse(plugin).unwrap(),
    }
}

fn source_plugin(record: &FileRecord) -> String {
    match &record.source {
        FileSource::Bot { plugin, .. } => plugin.clone(),
        other => panic!("{other:?}"),
    }
}

/// 卡片上的来源是主人判断内容可不可信的依据，分享之后不能被另一个插件改掉。
#[tokio::test]
async fn a_shared_files_source_stays_when_another_plugin_gives_the_same_reference() {
    let f = Fixture::new();
    f.put("photos/a.jpg", b"a");
    let give = |plugin: &'static str| {
        let files = f.files.clone();
        async move {
            let bot = BotRef {
                name: "hermes".to_owned(),
                plugin: PluginId::parse(plugin).unwrap(),
            };
            let reference = ShareRef::parse("wonton-share:photos/a.jpg").unwrap();
            files
                .register_share(&bot, reference, None, None, &any_langs())
                .await
                .unwrap()
        }
    };
    let plugin_of = |record: &FileRecord| match &record.source {
        FileSource::Bot { plugin, .. } => plugin.clone(),
        other => panic!("{other:?}"),
    };
    let first = give("app.first").await;
    assert_eq!(
        plugin_of(&give("app.second").await),
        "app.second",
        "没分享的看最近给 Bot 引用的插件"
    );
    assert_eq!(plugin_of(&give("app.first").await), "app.first");
    f.files
        .share_with_owner(&first.id, "hermes", &any_langs())
        .await
        .unwrap();
    let again = give("app.second").await;
    assert_ne!(again.id, first.id, "换了插件发新编号");
    assert_eq!(plugin_of(&again), "app.second");
    let old = f
        .files
        .info(&first.id, &Accessor::Owner, &any_langs())
        .await
        .unwrap();
    assert!(matches!(old.source, FileSource::Bot { plugin, .. } if plugin == "app.first"));
}

/// 分享过的文件，来源插件没了授权，换一个有授权的插件给同一个引用：Bot 用新编号，不会一直卡在旧插件上。
#[tokio::test]
async fn a_shared_file_given_again_by_another_plugin_gets_its_own_id() {
    let f = Fixture::new();
    f.put("photos/a.txt", b"a");
    let (a, b) = (from_plugin("app.a"), from_plugin("app.b"));
    let langs = any_langs();
    let register = |bot| {
        f.files
            .register_share(bot, share("photos/a.txt"), None, None, &langs)
    };
    let first = register(&a).await.unwrap();
    f.files
        .share_with_owner(&first.id, "hermes", &any_langs())
        .await
        .unwrap();
    assert_eq!(register(&a).await.unwrap().id, first.id, "同一个插件再给");

    f.host
        .revoked_for
        .lock()
        .unwrap()
        .push(("app.a".into(), "photos".into()));
    let again = register(&b).await.unwrap();
    assert_ne!(again.id, first.id);
    assert_eq!(source_plugin(&again), "app.b");
    assert_eq!(
        f.files
            .for_bot(&first.id, "hermes", &any_langs())
            .await
            .unwrap_err()
            .kind,
        FileFailure::PermissionDenied
    );
    f.files
        .for_bot(&again.id, "hermes", &any_langs())
        .await
        .unwrap();
    assert_eq!(register(&b).await.unwrap().id, again.id);
}

/// 有写权限的插件改了文件再给：旧卡片的来源不变，新内容的卡片写它自己。
#[tokio::test]
async fn new_content_cannot_be_shared_under_the_old_source() {
    let f = Fixture::new();
    f.put("photos/a.txt", b"a");
    let langs = any_langs();
    let register = |bot| {
        f.files
            .register_share(bot, share("photos/a.txt"), None, None, &langs)
    };
    let (a, b) = (from_plugin("app.a"), from_plugin("app.b"));
    let first = register(&a).await.unwrap();
    f.files
        .share_with_owner(&first.id, "hermes", &any_langs())
        .await
        .unwrap();

    f.put("photos/a.txt", b"written by b");
    let second = register(&b).await.unwrap();
    let shared = f
        .files
        .share_with_owner(&second.id, "hermes", &any_langs())
        .await
        .unwrap();
    assert_eq!(source_plugin(&shared), "app.b");
    let old = f
        .files
        .info(&first.id, &Accessor::Owner, &any_langs())
        .await
        .unwrap();
    assert!(matches!(old.source, FileSource::Bot { plugin, .. } if plugin == "app.a"));
}

/// 单独收回之后，Bot 经插件重新找到同一个文件：回到范围里，按来源插件的授权和文件改没改过算。
#[tokio::test]
async fn a_bot_revoked_by_the_owner_can_find_the_file_again_through_its_plugin() {
    let f = Fixture::new();
    f.put("photos/a.txt", b"a");
    let bot = hermes();
    let langs = any_langs();
    let register = || {
        f.files
            .register_share(&bot, share("photos/a.txt"), None, None, &langs)
    };
    let id = register().await.unwrap().id;
    f.files
        .share_with_owner(&id, "hermes", &any_langs())
        .await
        .unwrap();
    f.host
        .rooms
        .lock()
        .unwrap()
        .insert("!bot:home".into(), vec!["hermes".into()]);
    f.files
        .grant_room(&id, "!bot:home", &any_langs())
        .await
        .unwrap();
    f.files
        .revoke_bot(&id, "hermes", &any_langs())
        .await
        .unwrap();
    assert_eq!(
        f.files
            .for_bot(&id, "hermes", &any_langs())
            .await
            .unwrap_err()
            .kind,
        FileFailure::FileUnknown
    );

    assert_eq!(register().await.unwrap().id, id);
    f.files.for_bot(&id, "hermes", &any_langs()).await.unwrap();
    f.put("photos/a.txt", b"edited");
    assert_eq!(
        f.files
            .for_bot(&id, "hermes", &any_langs())
            .await
            .unwrap_err()
            .kind,
        FileFailure::FileChanged,
        "主人亲手的授权没有恢复"
    );
}

/// 同一个错误按 ja 和 fr-FR 拿到不同的话：文件接口的文字按请求的语言挑（期望值由目录格式化出来）。
#[tokio::test]
async fn error_texts_follow_the_requested_language() {
    use crate::l10n::CATALOG;
    use crate::plugin::languages::Languages;

    let f = Fixture::new();
    f.put("photos/a.jpg", b"photo");
    let record = f
        .files
        .register_share(&hermes(), share("photos/a.jpg"), None, None, &any_langs())
        .await
        .unwrap();
    let not_shared = |langs: Languages| {
        f.files
            .authorize(&record.id, &Accessor::Bot("other".into()), &langs)
            .unwrap_err()
            .message
    };
    let ja = not_shared(Languages::from_header(Some("ja")));
    let french = not_shared(Languages::from_header(Some("fr-FR")));
    // Bot 看不见别人的分享，一律按编号不认识回（防探测）：files-not-found。
    let expected = |tag: &str| CATALOG.format(CATALOG.negotiate(&[tag]), "files-not-found", &[]);
    assert_eq!(ja, expected("ja"));
    // fr-FR 目录里没有，按英语说。
    assert_eq!(french, expected("en"));
    assert_ne!(ja, french);
}
