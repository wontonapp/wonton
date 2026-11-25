//! `auth: credentials` 的状态机（ADR-0021「专用密码」）：假 exec 按队列给结果，不起 Docker。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;

use super::super::fake_docker::FakeDocker;
use super::super::{PluginId, PluginManager};
use super::manager::manager_with;
use super::{langs, package};
use crate::clients::credentials::{
    COMMAND_TIMEOUT, CredentialError, CredentialExec, ExecError, ExecOutput, Verify,
};
use crate::clients::passwords::{PasswordError, PasswordStore, State};
use crate::clients::pending::{Issue, Settle};

#[derive(Debug, Clone)]
enum Step {
    Ok(i64, &'static str, &'static str),
    Out(String),
    Timeout,
    Engine,
}

#[derive(Debug, Clone)]
struct Call {
    argv: Vec<String>,
    stdin: Option<String>,
}

#[derive(Default)]
struct Script {
    queue: Mutex<VecDeque<Step>>,
    calls: Mutex<Vec<Call>>,
}

impl Script {
    fn push(&self, steps: impl IntoIterator<Item = Step>) {
        self.queue.lock().unwrap().extend(steps);
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn ops(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .map(|call| call.argv[1].clone())
            .collect()
    }

    fn stdins(&self) -> Vec<Option<String>> {
        self.calls().into_iter().map(|call| call.stdin).collect()
    }
}

impl CredentialExec for Script {
    fn exec<'a>(
        &'a self,
        _container: &'a str,
        argv: &'a [String],
        stdin: Option<&'a [u8]>,
        _timeout: Duration,
    ) -> BoxFuture<'a, Result<ExecOutput, ExecError>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(Call {
                argv: argv.to_vec(),
                stdin: stdin.map(|bytes| String::from_utf8_lossy(bytes).into_owned()),
            });
            match self.queue.lock().unwrap().pop_front() {
                Some(Step::Out(stdout)) => Ok(ExecOutput {
                    exit_code: 0,
                    stdout,
                    stderr: String::new(),
                }),
                Some(Step::Ok(code, stdout, stderr)) => Ok(ExecOutput {
                    exit_code: code,
                    stdout: stdout.to_owned(),
                    stderr: stderr.to_owned(),
                }),
                Some(Step::Timeout) => Err(ExecError::Timeout),
                Some(Step::Engine) => Err(ExecError::Engine("容器没了".into())),
                // 没排的就当成功：安装时的首次共享签发也会走这里。
                None => Ok(ExecOutput {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                }),
            }
        })
    }
}

fn manifest(auth: &str) -> String {
    package(
        "  web:\n    image: nginx\n",
        &format!(
            "  permissions: [network.clients]\n  clients:\n    http:\n      service: web\n      port: 3000\n      auth:\n        credentials:\n{auth}"
        ),
    )
    .replace("spec: '0.5'", "spec: '0.13'")
}

const PER_CLIENT: &str = "          issue: [pw, issue, '{name}']\n          verify: [pw, verify, '{name}']\n          revoke: [pw, revoke, '{name}']\n          ends_sessions: false\n";

struct Home {
    manager: Arc<PluginManager>,
    script: Arc<Script>,
    id: PluginId,
    dir: tempfile::TempDir,
}

async fn home(auth: &str) -> Option<Home> {
    let (docker, engine) = FakeDocker::start().await;
    docker.on_exec_output(|_, _, _| (0, String::new(), String::new()));
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    let script = Arc::new(Script::default());
    manager.set_credential_exec(script.clone());
    let id = PluginId::parse("app.test").unwrap();
    if let Err(error) = manager
        .install(
            &manifest(auth),
            &[wonton_protocol::manifest::Permission::NetworkClients],
            langs(),
        )
        .await
    {
        panic!("install 失败: {error:#?}");
    }
    Some(Home {
        manager,
        script,
        id,
        dir,
    })
}

fn store(home: &Home) -> Arc<PasswordStore> {
    home.manager.client_passwords.clone().unwrap()
}

#[tokio::test]
async fn issue_overwrites_and_is_idempotent() {
    let home = home(PER_CLIENT).await.unwrap();
    home.script.push([Step::Ok(0, "", "")]);
    let Issue::Done(issued) = home
        .manager
        .issue_new(&home.id, "手机", None)
        .await
        .unwrap()
    else {
        panic!("插件在跑就该当场签发");
    };
    let stored = store(&home).snapshot("app.test", &issued.id).unwrap();
    assert_eq!(stored.state, State::Ready);
    // 再发一次同一个值：应用看到的就是同一个值，落地值不变。
    home.script.push([Step::Ok(0, "", "")]);
    home.manager
        .issue_password(&home.id, &stored, false)
        .await
        .unwrap();
    let again = store(&home).snapshot("app.test", &issued.id).unwrap();
    assert_eq!(again.value, issued.value);
    assert_eq!(
        home.script.stdins(),
        [
            Some(format!("{}\n", issued.value)),
            Some(format!("{}\n", issued.value))
        ]
    );
}

#[tokio::test]
async fn generates_reads_first_line_only() {
    let mut auth = PER_CLIENT.to_owned();
    auth.push_str("          generates: true\n");
    let home = home(&auth).await.unwrap();
    home.script.push([Step::Ok(0, "  abc  \n第二批\n", "")]);
    let Issue::Done(issued) = home
        .manager
        .issue_new(&home.id, "手机", None)
        .await
        .unwrap()
    else {
        panic!("一样要当场签发");
    };
    assert_eq!(issued.value, "abc");
    assert_eq!(home.script.calls()[0].stdin, None);

    // 4 KiB 截断，且不切坏字符。
    let long = "x".repeat(5000);
    home.script.push([Step::Out(long)]);
    let snapshot = store(&home).snapshot("app.test", &issued.id).unwrap();
    let generated = home
        .manager
        .issue_password(&home.id, &snapshot, true)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(generated.len(), 4096);
}

#[tokio::test]
async fn verify_then_issue_on_invalid() {
    let home = home(PER_CLIENT).await.unwrap();
    home.script.push([Step::Ok(0, "", "")]);
    let Issue::Done(issued) = home
        .manager
        .issue_new(&home.id, "手机", None)
        .await
        .unwrap()
    else {
        panic!("先发出去");
    };
    let before = home.script.calls().len();
    home.script
        .push([Step::Ok(1, "", "旧值不认了"), Step::Ok(0, "", "")]);
    home.manager.reconcile_credentials(&home.id).await;
    let calls = home.script.calls()[before..].to_vec();
    assert_eq!(calls[0].argv[1], "verify");
    assert_eq!(calls[1].argv[1], "issue");
    // 无效就重发同一个值，不换。
    assert_eq!(calls[1].stdin, Some(format!("{}\n", issued.value)));
    assert_eq!(
        store(&home).snapshot("app.test", &issued.id).unwrap().value,
        issued.value
    );
}

#[tokio::test]
async fn rotate_pending_survives_restart() {
    let home = home(PER_CLIENT).await.unwrap();
    home.script.push([Step::Ok(0, "", "")]);
    let Issue::Done(issued) = home
        .manager
        .issue_new(&home.id, "手机", None)
        .await
        .unwrap()
    else {
        panic!("先发出去");
    };
    let store = store(&home);
    store
        .begin_rotate("app.test", &issued.id, Some("next-value".into()))
        .unwrap();
    // 重启：重新打开同一个文件，待换状态还在，旧值照旧能用。
    let reopened = PasswordStore::open(home.dir.path()).unwrap();
    assert_eq!(
        reopened.snapshot("app.test", &issued.id).unwrap().state,
        State::PendingRotate {
            next: Some("next-value".into())
        }
    );
    assert!(
        reopened
            .authenticate("app.test", &issued.value)
            .unwrap()
            .is_some()
    );
    drop(reopened);
    home.script.push([Step::Ok(0, "", "")]);
    home.manager.reconcile_credentials(&home.id).await;
    let after = store.snapshot("app.test", &issued.id).unwrap();
    assert_eq!(after.state, State::Ready);
    assert_eq!(after.value, "next-value");
    assert!(
        store
            .authenticate("app.test", &issued.value)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn revoke_timeout_is_unknown_and_retried() {
    let home = home(PER_CLIENT).await.unwrap();
    home.script.push([Step::Ok(0, "", "")]);
    let Issue::Done(issued) = home
        .manager
        .issue_new(&home.id, "手机", None)
        .await
        .unwrap()
    else {
        panic!("先发出去");
    };
    let store = store(&home);
    home.script.push([Step::Timeout]);
    let first = home
        .manager
        .settle_password(&home.id, &issued.id)
        .await
        .unwrap();
    assert!(matches!(first, Settle::Failed(CredentialError::Timeout)));
    assert_eq!(
        store.snapshot("app.test", &issued.id).unwrap().state,
        State::PendingRevoke
    );
    assert!(
        store
            .authenticate("app.test", &issued.value)
            .unwrap()
            .is_some()
    );

    let before = home.script.calls().len();
    home.script.push([Step::Ok(0, "", "")]);
    home.manager.reconcile_credentials(&home.id).await;
    assert_eq!(home.script.calls()[before].argv[1], "revoke");
    assert!(store.snapshot("app.test", &issued.id).is_err());
}

#[tokio::test]
async fn reconcile_never_reissues_superseded_value() {
    let home = home(PER_CLIENT).await.unwrap();
    home.script.push([Step::Ok(0, "", "")]);
    let Issue::Done(issued) = home
        .manager
        .issue_new(&home.id, "手机", None)
        .await
        .unwrap()
    else {
        panic!("先发出去");
    };
    store(&home)
        .begin_rotate("app.test", &issued.id, Some("fresh".into()))
        .unwrap();
    let before = home.script.calls().len();
    home.script.push([Step::Ok(0, "", ""), Step::Ok(0, "", "")]);
    home.manager.reconcile_credentials(&home.id).await;
    let calls = home.script.calls()[before..].to_vec();
    // 清单没有 rotate：重试用 issue 覆盖，发的是待换新值；落地后按 verify 对账。
    assert_eq!(home.script.ops()[before..], ["issue", "verify"]);
    assert_eq!(calls[0].stdin, Some("fresh\n".into()));
    assert_eq!(
        store(&home).snapshot("app.test", &issued.id).unwrap().value,
        "fresh"
    );
}

#[tokio::test]
async fn pending_revoke_only_retries_revoke() {
    let home = home(PER_CLIENT).await.unwrap();
    home.script.push([Step::Ok(0, "", "")]);
    let Issue::Done(issued) = home
        .manager
        .issue_new(&home.id, "手机", None)
        .await
        .unwrap()
    else {
        panic!("先发出去");
    };
    let store = store(&home);
    store.begin_revoke("app.test", &issued.id).unwrap();
    let before = home.script.calls().len();
    home.script.push([Step::Ok(0, "", "")]);
    home.manager.reconcile_credentials(&home.id).await;
    let calls = home.script.calls()[before..].to_vec();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].argv[1], "revoke");
    assert!(store.snapshot("app.test", &issued.id).is_err());
}

async fn shared_home() -> Home {
    // 共享清单：必须有 rotate，不能有 revoke。
    let auth = "          per_client: false\n          rotate: [pw, rotate, '{name}']\n          issue: [pw, issue, '{name}']\n          ends_sessions: false\n";
    home(auth).await.unwrap()
}

async fn shared_generated_home() -> Home {
    let auth = "          per_client: false\n          generates: true\n          verify: [pw, verify, '{name}']\n          rotate: [pw, rotate, '{name}']\n          issue: [pw, issue, '{name}']\n          ends_sessions: false\n";
    home(auth).await.unwrap()
}

#[tokio::test]
async fn shared_password_rotated_on_device_revoke_unless_kept() {
    let home = shared_home().await;
    let store = store(&home);
    let issued = store
        .snapshot("app.test", "shared")
        .expect("安装时就发了共享密码");
    store.read("app.test", "shared", Some("device-a")).unwrap();
    let before = home.script.calls().len();
    let report = home
        .manager
        .revoke_passwords_for_device("device-a", false)
        .await
        .unwrap();
    assert!(report.pending_revokes.is_empty());
    assert_eq!(home.script.ops()[before..], ["rotate"]);
    let rotated = store.snapshot("app.test", "shared").unwrap();
    assert_eq!(rotated.state, State::Ready);
    assert_ne!(rotated.value, issued.value);

    // keep_shared：记录和值都留着、不跑命令，别的设备继续用老值。
    store.read("app.test", "shared", Some("device-a")).unwrap();
    let kept_value = store.snapshot("app.test", "shared").unwrap().value;
    // 老值上挂着一条已升级的连接，keep_shared 吊销不该断它。
    let receiver = store
        .authenticate("app.test", &kept_value)
        .unwrap()
        .unwrap();
    let before = home.script.calls().len();
    let kept_report = home
        .manager
        .revoke_passwords_for_device("device-a", true)
        .await
        .unwrap();
    assert_eq!(home.script.calls().len(), before);
    assert!(!*receiver.borrow());
    let kept = store.snapshot("app.test", "shared").unwrap();
    assert_eq!(kept.value, kept_value);
    assert_eq!(kept.state, State::Ready);
    // 重启后对账重发同一个值，不再换。
    home.manager.reconcile_credentials(&home.id).await;
    assert_eq!(
        store.snapshot("app.test", "shared").unwrap().value,
        kept_value
    );
    // 登录没结束的应用仍要列出来。
    assert_eq!(kept_report.ends_sessions_false, ["app.test"]);
    assert!(kept_report.pending_revokes.is_empty());
}

#[tokio::test]
async fn ends_sessions_false_listed_in_response() {
    // shared_home 的清单就是 `ends_sessions: false`。
    let home = shared_home().await;
    store(&home)
        .read("app.test", "shared", Some("device-a"))
        .unwrap();
    let report = home
        .manager
        .revoke_passwords_for_device("device-a", true)
        .await
        .unwrap();
    assert_eq!(report.ends_sessions_false, ["app.test"]);
}

#[tokio::test]
async fn switch_blocked_until_first_issue() {
    let home = home(PER_CLIENT).await.unwrap();
    assert!(!home.manager.credentials_ready(&home.id));
    home.script.push([Step::Ok(0, "", "")]);
    home.manager
        .issue_new(&home.id, "手机", None)
        .await
        .unwrap();
    assert!(home.manager.credentials_ready(&home.id));
}

#[test]
fn command_timeout_30s() {
    assert_eq!(COMMAND_TIMEOUT, Duration::from_secs(30));
}

#[tokio::test]
async fn verify_unknown_keeps_value() {
    let home = home(PER_CLIENT).await.unwrap();
    home.script.push([Step::Ok(0, "", "")]);
    let Issue::Done(issued) = home
        .manager
        .issue_new(&home.id, "手机", None)
        .await
        .unwrap()
    else {
        panic!("先发出去");
    };
    let snapshot = store(&home).snapshot("app.test", &issued.id).unwrap();
    home.script.push([Step::Engine]);
    assert_eq!(
        home.manager.verify_password(&home.id, &snapshot).await,
        Verify::Unknown
    );
    assert_eq!(
        store(&home).snapshot("app.test", &issued.id).unwrap().value,
        issued.value
    );
}

#[tokio::test]
async fn unavailable_issue_leaves_pending_value() {
    let (docker, engine) = FakeDocker::start().await;
    docker.on_exec_output(|_, _, _| (0, String::new(), String::new()));
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    manager.set_credential_exec(Arc::new(Script::default()));
    let id = PluginId::parse("app.test").unwrap();
    manager
        .install(
            &manifest(PER_CLIENT),
            &[wonton_protocol::manifest::Permission::NetworkClients],
            langs(),
        )
        .await
        .unwrap();
    let store = manager.client_passwords.clone().unwrap();
    // 停掉插件的服务：命令跑不成，值已经落盘、照旧能用。
    {
        let mut plugins = manager.plugins.write().unwrap();
        plugins.get_mut(&id).unwrap().serving.clear();
    }
    let Issue::Deferred(issued) = manager.issue_new(&id, "手机", None).await.unwrap() else {
        panic!("没在跑就该 202 路径");
    };
    assert_eq!(
        store.snapshot("app.test", &issued.id).unwrap().state,
        State::PendingRotate {
            next: Some(issued.value.clone())
        }
    );
    assert!(
        store
            .authenticate("app.test", &issued.value)
            .unwrap()
            .is_some()
    );
    assert!(matches!(
        store.snapshot("app.test", "nope"),
        Err(PasswordError::NotFound)
    ));
}

#[tokio::test]
async fn deferred_issue_without_rotate_is_retried_with_issue() {
    // per-client 的 rotate 可选：新签发落在待换，没 rotate 时启动重试必须走 issue。
    let auth = "          issue: [pw, issue, '{name}']\n          revoke: [pw, revoke, '{name}']\n          ends_sessions: false\n";
    let home = home(auth).await.unwrap();
    {
        let mut plugins = home.manager.plugins.write().unwrap();
        plugins.get_mut(&home.id).unwrap().serving.clear();
    }
    let Issue::Deferred(issued) = home
        .manager
        .issue_new(&home.id, "手机", None)
        .await
        .unwrap()
    else {
        panic!("插件没在跑，应该 202");
    };
    // 恢复服务后启动对账：补 issue 落地，之后 Ready 记录无 verify 会被再发同一个值。
    {
        let mut plugins = home.manager.plugins.write().unwrap();
        plugins
            .get_mut(&home.id)
            .unwrap()
            .serving
            .insert("web".into());
    }
    home.script.push([Step::Ok(0, "", ""), Step::Ok(0, "", "")]);
    home.manager.reconcile_credentials(&home.id).await;
    let calls = home.script.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].argv[1], "issue");
    assert_eq!(calls[0].stdin, Some(format!("{}\n", issued.value)));
    assert_eq!(calls[1].argv[1], "issue");
    assert_eq!(calls[1].stdin, Some(format!("{}\n", issued.value)));
    let stored = store(&home).snapshot("app.test", &issued.id).unwrap();
    assert_eq!(stored.state, State::Ready);
    assert_eq!(stored.value, issued.value);
}

#[tokio::test]
async fn generates_with_empty_stdout_stays_pending() {
    let mut auth = PER_CLIENT.to_owned();
    auth.push_str("          generates: true\n");
    let home = home(&auth).await.unwrap();
    home.script.push([Step::Ok(0, "", "")]);
    let Issue::Failed { issued, error } = home
        .manager
        .issue_new(&home.id, "手机", None)
        .await
        .unwrap()
    else {
        panic!("没有输出应该失败");
    };
    assert!(matches!(error, CredentialError::Failed { code: 0, .. }));
    let stored = store(&home).snapshot("app.test", &issued.id).unwrap();
    assert_eq!(stored.state, State::PendingRotate { next: None });
    assert!(!home.manager.credentials_ready(&home.id));
}

#[tokio::test]
async fn shared_generated_empty_output_retries_without_stdin() {
    let home = shared_generated_home().await;
    let store = store(&home);
    assert_eq!(
        store.snapshot("app.test", "shared").unwrap().state,
        State::PendingRotate { next: None }
    );

    // 应用没输出：仍是待换，下次还要跑 issue、且不给标准输入。
    home.manager.reconcile_credentials(&home.id).await;
    assert_eq!(
        store.snapshot("app.test", "shared").unwrap().state,
        State::PendingRotate { next: None }
    );
    let before = home.script.calls().len();
    home.script.push([Step::Out("from-app\n".into())]);
    home.manager.reconcile_credentials(&home.id).await;
    let calls = home.script.calls()[before..].to_vec();
    // 共享清单带 rotate：重试跑 rotate、不给标准输入，新值从标准输出读。
    assert_eq!(calls[0].argv[1], "rotate");
    assert_eq!(calls[0].stdin, None);
    let stored = store.snapshot("app.test", "shared").unwrap();
    assert_eq!(stored.state, State::Ready);
    assert_eq!(stored.value, "from-app");
}

#[tokio::test]
async fn concurrent_reconcile_runs_one_rotate() {
    let home = home(PER_CLIENT).await.unwrap();
    home.script.push([Step::Ok(0, "", "")]);
    let Issue::Done(issued) = home
        .manager
        .issue_new(&home.id, "手机", None)
        .await
        .unwrap()
    else {
        panic!("先发出去");
    };
    store(&home)
        .begin_rotate("app.test", &issued.id, Some("only-once".into()))
        .unwrap();
    let before = home.script.calls().len();
    tokio::join!(
        home.manager.reconcile_credentials(&home.id),
        home.manager.reconcile_credentials(&home.id),
    );
    let calls = home.script.calls()[before..].to_vec();
    // 没有 rotate 命令：重试是 issue；第二个对账只补一次 verify，不再重发值。
    assert_eq!(home.script.ops()[before..], ["issue", "verify", "verify"]);
    assert_eq!(calls[0].stdin, Some("only-once\n".into()));
    assert_eq!(
        store(&home).snapshot("app.test", &issued.id).unwrap().value,
        "only-once"
    );
}

#[tokio::test]
async fn deleted_shared_is_recreated_with_fixed_id() {
    let home = shared_home().await;
    let store = store(&home);
    store.revoke("app.test", "shared").unwrap();
    let Issue::Done(issued) = home
        .manager
        .issue_new(&home.id, "共享", None)
        .await
        .unwrap()
    else {
        panic!("共享密码要当场签发");
    };
    assert_eq!(issued.id, "shared");
    let before = home.script.calls().len();
    home.manager.reconcile_credentials(&home.id).await;
    // 已经有了：对账不再补第二条，只按「无 verify 重发同值」再发一遍。
    assert_eq!(store.snapshots("app.test").len(), 1);
    assert_eq!(home.script.ops()[before..], ["issue"]);
    assert_eq!(
        home.script.calls()[before].stdin,
        Some(format!("{}\n", issued.value))
    );
}

#[tokio::test]
async fn concurrent_first_issue_only_one_wins() {
    let home = shared_home().await;
    let store = store(&home);
    store.revoke("app.test", "shared").unwrap();
    let (first, second) = tokio::join!(
        home.manager.issue_new(&home.id, "甲", None),
        home.manager.issue_new(&home.id, "乙", None),
    );
    let outcomes = [first, second];
    assert_eq!(
        outcomes
            .iter()
            .filter(|issue| matches!(issue, Ok(Issue::Done(_))))
            .count(),
        1
    );
    assert!(outcomes.iter().any(|issue| matches!(
        issue,
        Err(crate::clients::pending::IssueError::SharedExists)
    )));
    assert_eq!(store.snapshots("app.test").len(), 1);
}

#[tokio::test]
async fn concurrent_reconcile_and_settle_do_not_overlap() {
    let home = home(PER_CLIENT).await.unwrap();
    home.script.push([Step::Ok(0, "", "")]);
    let Issue::Done(issued) = home
        .manager
        .issue_new(&home.id, "手机", None)
        .await
        .unwrap()
    else {
        panic!("先发出去");
    };
    store(&home)
        .begin_rotate("app.test", &issued.id, Some("next".into()))
        .unwrap();
    let before = home.script.calls().len();
    let (_, settled) = tokio::join!(
        home.manager.reconcile_credentials(&home.id),
        home.manager.settle_password(&home.id, &issued.id),
    );
    settled.unwrap();
    // 换值先拿锁，吊销等它落盘后才跑：命令一条接一条，不叠着。
    // 没有 rotate 命令：重试是 issue；对账里 verify 过了才轮到吊销。
    assert_eq!(home.script.ops()[before..], ["issue", "verify", "revoke"]);
    assert!(store(&home).snapshot("app.test", &issued.id).is_err());
}

#[tokio::test]
async fn generated_reconcile_without_rotate_keeps_pending_on_timeout() {
    // per-client、generates、没有 rotate：超时后应用可能已经换了值，记录要留在待换。
    let auth = "          generates: true\n          issue: [pw, issue, '{name}']\n          verify: [pw, verify, '{name}']\n          revoke: [pw, revoke, '{name}']\n          ends_sessions: false\n";
    let home = home(auth).await.unwrap();
    home.script.push([Step::Ok(0, "first", "")]);
    let Issue::Done(issued) = home
        .manager
        .issue_new(&home.id, "手机", None)
        .await
        .unwrap()
    else {
        panic!("先发出去");
    };
    home.script.push([Step::Ok(1, "", ""), Step::Timeout]);
    home.manager.reconcile_credentials(&home.id).await;
    let stored = store(&home).snapshot("app.test", &issued.id).unwrap();
    assert_eq!(stored.state, State::PendingRotate { next: None });
    assert!(
        store(&home)
            .authenticate("app.test", &issued.value)
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn shared_rotate_with_ends_sessions_drops_connections() {
    let auth = "          per_client: false\n          ends_sessions: true\n          rotate: [pw, rotate, '{name}']\n          issue: [pw, issue, '{name}']\n";
    let home = home(auth).await.unwrap();
    let store = store(&home);
    let old = store.snapshot("app.test", "shared").unwrap().value;
    let mut receiver = store.authenticate("app.test", &old).unwrap().unwrap();
    assert!(!*receiver.borrow());
    home.script.push([Step::Ok(0, "", "")]);
    let settled = home
        .manager
        .settle_password(&home.id, "shared")
        .await
        .unwrap();
    assert!(matches!(settled, Settle::Done));
    assert!(*receiver.borrow_and_update());
    // 新连接用新值建一个新守卫，不会被上一次的置位带走。
    let new = store.snapshot("app.test", "shared").unwrap().value;
    let fresh = store.authenticate("app.test", &new).unwrap().unwrap();
    assert!(!*fresh.borrow());
}

#[tokio::test]
async fn pending_retry_runs_without_a_start() {
    let home = home(PER_CLIENT).await.unwrap();
    home.script.push([Step::Ok(0, "", "")]);
    let Issue::Done(issued) = home
        .manager
        .issue_new(&home.id, "手机", None)
        .await
        .unwrap()
    else {
        panic!("先发出去");
    };
    let store = store(&home);
    store
        .begin_rotate("app.test", &issued.id, Some("next".into()))
        .unwrap();
    let before = home.script.calls().len();
    home.manager.retry_pending_credentials(&home.id).await;
    assert_eq!(home.script.ops()[before..], ["issue"]);
    assert_eq!(
        store.snapshot("app.test", &issued.id).unwrap().value,
        "next"
    );
}
