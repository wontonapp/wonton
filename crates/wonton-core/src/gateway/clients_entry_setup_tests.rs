//! setup 门：setup_done 没过的请求一直 409，过了就缓存到插件重新部署；
//! 失败只给匿名通用类别，不漏 stderr。

use super::*;

#[tokio::test]
async fn setup_pending_until_setup_done_succeeds() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    let setup_done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = setup_done.clone();
    home.docker.on_exec(move |_container, argv, _stdin| {
        assert_eq!(argv.first().map(String::as_str), Some("sh"));
        if flag.load(std::sync::atomic::Ordering::SeqCst) {
            (0, String::new())
        } else {
            (1, String::new())
        }
    });
    let pending = home.get(&host, "/", &entry_public()).await;
    assert_eq!(pending.status, StatusCode::CONFLICT);
    assert_eq!(pending.error_kind(), Some("setup_pending"));
    assert!(home.upstream_hits().is_empty());
    setup_done.store(true, std::sync::atomic::Ordering::SeqCst);
    let ok = home.get(&host, "/", &entry_public()).await;
    assert_eq!(ok.status, StatusCode::OK);
    // 查到完成就缓存到插件重新部署：后续请求不再执行命令。
    let cached = home.get(&host, "/", &entry_public()).await;
    assert_eq!(cached.status, StatusCode::OK);
    assert_eq!(home.docker.execs().len(), 2);
    assert_eq!(home.upstream_hits().len(), 2);
}

#[tokio::test]
async fn setup_done_error_hides_stderr_from_anonymous() {
    let home = Home::start().await;
    home.register(ENTRY_APP, "entryapp", false, true);
    let host = home.host("entryapp");
    home.docker
        .on_exec(|_container, _argv, _stdin| (7, "database is migrating".to_owned()));
    let reply = home.get(&host, "/", &entry_public()).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), Some("plugin_unavailable"));
    // 匿名响应只带通用类别，不带 stderr 或 Docker 错误（review P0-4）；完整错误在服务器日志里。
    assert!(!reply.text().contains("database is migrating"));
    assert!(home.upstream_hits().is_empty());
}
