//! 排队的签发：设置变了跳过旧快照，last_error 变化不算设置变。

use super::*;

/// 排队中的旧签发任务带的是排队时的设置快照：拿到锁后重读现状，变了就跳过——
/// 不然旧快照会重新 prepare，把新服务商的准备状态和证书都写成旧的，续期从此对不上。
#[tokio::test]
async fn queued_issue_skips_when_settings_changed_before_the_lock() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let clients = domain(dir.path());
    // 当前设置是 A（httpreq + A 凭据）。
    clients
        .apply(|store| {
            *store = super::Store {
                suffix: Some("abcdefgh.example.test".into()),
                provider: Some("httpreq".into()),
                credentials: [("HTTPREQ_TOKEN".into(), "a-token".into())].into(),
                env_file: None,
                terms_accepted: true,
                last_error: None,
            };
            Ok(())
        })
        .await
        .unwrap();
    // 模拟“排队时抓了 A 快照、还没拿到签发锁”：另一轮 save 已把设置写成 B。
    let stale = clients.effective().unwrap();
    clients
        .apply(|store| {
            store.provider = Some("otherdns".into());
            store.credentials = [("OTHERDNS_TOKEN".into(), "b-token".into())].into();
            Ok(())
        })
        .await
        .unwrap();
    let quiet = clients.engine.recorded_creations().len();
    // 旧任务此刻拿到锁：设置已变，必须跳过，不能按 A 起 lego。
    clients.pending.fetch_add(1, Ordering::SeqCst);
    clients.clone().run_issue(stale, false).await;
    assert_eq!(
        clients.engine.recorded_creations().len(),
        quiet,
        "过期快照的任务不该真的起 lego 容器"
    );
    // 对照：按现状排的任务照常签（测试引擎在起容器那步失败，但“试图起”发生）。
    let fresh = clients.effective().unwrap();
    clients.pending.fetch_add(1, Ordering::SeqCst);
    clients.clone().run_issue(fresh, false).await;
    assert_eq!(
        clients.engine.recorded_creations().len(),
        quiet + 1,
        "按最新设置排的任务应该照常走签发"
    );
}
/// 上一次签发的结果（last_error）写盘不该把排队的下一次签发误判成“设置变了”：
/// last_error 是签发的输出，不参与签发行为，比较时排除——否则修正凭据后要白等
/// 24 小时的续期循环，设置页还显示着旧失败的报错。
#[tokio::test]
async fn queued_issue_runs_even_after_last_error_changed() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    settings(dir.path(), Some("abcdefgh.example.test"));
    let clients = domain(dir.path());
    clients
        .apply(|store| {
            *store = super::Store {
                suffix: Some("abcdefgh.example.test".into()),
                provider: Some("httpreq".into()),
                credentials: [("HTTPREQ_TOKEN".into(), "a-token".into())].into(),
                env_file: None,
                terms_accepted: true,
                last_error: None,
            };
            Ok(())
        })
        .await
        .unwrap();
    // 上一次签发失败写下的 last_error，快照带着它排队。
    clients
        .update(|state| state.last_error = Some("上一次的失败".into()))
        .await
        .unwrap();
    let queued = clients.effective().unwrap();
    // 上一任务收尾改写了 last_error（成功清空或换成新失败都一样）：设置一个字没变。
    clients
        .update(|state| state.last_error = None)
        .await
        .unwrap();
    let quiet = clients.engine.recorded_creations().len();
    clients.pending.fetch_add(1, Ordering::SeqCst);
    clients.clone().run_issue(queued, false).await;
    assert_eq!(
        clients.engine.recorded_creations().len(),
        quiet + 1,
        "last_error 变化不该误杀排队的签发"
    );
}
