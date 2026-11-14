//! 按来源安装（插件规范的“来源”）：对着假的 GitHub（git 协议的 `ls-refs` 和 raw 文件）验证解析版本、取包、核对摘要
//! 和报错。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use wonton_protocol::{InstallRequest, PluginTrust};

use super::super::fake_docker::FakeDocker;
use super::super::github::GitHub;
use super::super::languages::Languages;
use super::super::{InstallError, PluginId, PluginManager};
use super::egress::serve;
use super::manager::manager_with;

/// （仓库，提交，路径）→ 文件内容。
type Files = BTreeMap<(String, String, String), Vec<u8>>;

/// 假的 GitHub：仓库 → tag（名字 → 提交），提交里的文件，和搜得到的仓库（仓库 → 星数）。
#[derive(Clone, Default)]
pub(super) struct FakeGitHub {
    pub(super) tags: Arc<Mutex<BTreeMap<String, BTreeMap<String, String>>>>,
    files: Arc<Mutex<Files>>,
    commits: Arc<Mutex<u32>>,
    pub(super) topic: Arc<Mutex<Vec<(String, u32)>>>,
    /// 读过 tag 的仓库，按先后。
    pub(super) tag_reads: Arc<Mutex<Vec<String>>>,
    /// 不答：什么请求都挂着，直到客户端超时。
    pub(super) hanging: Arc<AtomicBool>,
    /// 出错：什么请求都回 503。
    pub(super) broken: Arc<AtomicBool>,
    /// 只有 raw 出错，API 正常。
    pub(super) raw_broken: Arc<AtomicBool>,
    /// API 限速：`/repos/…` 回 403。
    pub(super) api_limited: Arc<AtomicBool>,
}

impl FakeGitHub {
    pub(super) async fn start() -> (Self, GitHub) {
        let fake = Self::default();
        let app = Router::new().fallback(handle).with_state(fake.clone());
        let addr = serve(app).await;
        (fake, GitHub::for_test(&format!("http://{addr}")))
    }

    /// 官方索引所在的位置放上这份内容。
    pub(super) fn publish_index(&self, index: Vec<u8>) {
        let key = (
            "wontonapp/wonton-store".to_owned(),
            "main".to_owned(),
            "index.json".to_owned(),
        );
        self.files.lock().unwrap().insert(key, index);
    }

    /// 默认分支上放这些文件（raw 的 `HEAD`）。
    pub(super) fn publish_at_head(&self, repo: &str, files: &[(&str, &str)]) {
        let mut stored = self.files.lock().unwrap();
        for (path, data) in files {
            let key = (repo.to_owned(), "HEAD".to_owned(), (*path).to_owned());
            stored.insert(key, data.as_bytes().to_vec());
        }
    }

    /// 提交这些文件，打上 tag（已有的 tag 就是移动它），返回提交号。
    pub(super) fn publish(&self, repo: &str, tag: &str, files: &[(&str, &str)]) -> String {
        let commit = {
            let mut n = self.commits.lock().unwrap();
            *n += 1;
            format!("{:040x}", *n)
        };
        let mut stored = self.files.lock().unwrap();
        for (path, data) in files {
            let key = (repo.to_owned(), commit.clone(), (*path).to_owned());
            stored.insert(key, data.as_bytes().to_vec());
        }
        let mut tags = self.tags.lock().unwrap();
        let repo_tags = tags.entry(repo.to_owned()).or_default();
        repo_tags.insert(tag.to_owned(), commit.clone());
        commit
    }
}

fn pkt_line(payload: &str) -> String {
    format!("{:04x}{payload}", payload.len() + 4)
}

async fn handle(State(fake): State<FakeGitHub>, request: Request) -> Response {
    if fake.hanging.load(Ordering::Relaxed) {
        std::future::pending::<()>().await;
    }
    if fake.broken.load(Ordering::Relaxed) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let path = request.uri().path().trim_start_matches('/').to_owned();
    let parts: Vec<&str> = path.splitn(4, '/').collect();
    match (request.method(), parts.as_slice()) {
        (&Method::POST, [owner, git, "git-upload-pack"]) => {
            let repo = format!("{owner}/{}", git.trim_end_matches(".git"));
            fake.tag_reads.lock().unwrap().push(repo.clone());
            let tags = fake.tags.lock().unwrap().get(&repo).cloned();
            let Some(tags) = tags else {
                return StatusCode::UNAUTHORIZED.into_response();
            };
            let mut body: String = tags
                .iter()
                .map(|(name, commit)| pkt_line(&format!("{commit} refs/tags/{name}\n")))
                .collect();
            body.push_str("0000");
            Bytes::from(body).into_response()
        }
        (&Method::GET, ["search", "repositories"]) => {
            let items: Vec<_> = fake
                .topic
                .lock()
                .unwrap()
                .iter()
                .map(|(repo, stars)| {
                    let (owner, name) = repo.split_once('/').unwrap();
                    serde_json::json!({
                        "name": name,
                        "owner": {"login": owner},
                        "stargazers_count": stars,
                        "pushed_at": "2026-11-01T00:00:00Z",
                    })
                })
                .collect();
            axum::Json(serde_json::json!({"total_count": items.len(), "items": items}))
                .into_response()
        }
        (&Method::GET, ["repos", owner, repo, rest]) => {
            if fake.api_limited.load(Ordering::Relaxed) {
                return StatusCode::FORBIDDEN.into_response();
            }
            let reference = request.uri().query().and_then(|q| q.strip_prefix("ref="));
            let key = (
                format!("{owner}/{repo}"),
                reference.unwrap_or_default().to_owned(),
                rest.trim_start_matches("contents/").to_owned(),
            );
            match fake.files.lock().unwrap().get(&key) {
                Some(data) => Bytes::from(data.clone()).into_response(),
                None => StatusCode::NOT_FOUND.into_response(),
            }
        }
        (&Method::GET, _) if fake.raw_broken.load(Ordering::Relaxed) => {
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
        (&Method::GET, [owner, repo, commit, file]) => {
            let key = (
                format!("{owner}/{repo}"),
                (*commit).to_owned(),
                (*file).to_owned(),
            );
            match fake.files.lock().unwrap().get(&key) {
                Some(data) => Bytes::from(data.clone()).into_response(),
                None => StatusCode::NOT_FOUND.into_response(),
            }
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// 版本是 `version` 的插件，带一个图标文件。
pub(super) fn compose(version: &str, image: &str) -> String {
    format!(
        "services:\n  web:\n    image: {image}\nx-wonton:\n  spec: '0.4'\n  id: app.hello\n  version: {version}\n  kind: app\n  \
         title: {{zh: 你好, en: Hello}}\n  description: {{zh: 打个招呼, en: Say hi}}\n  icon: icon.svg\n  \
         permissions: []\n"
    )
}

pub(super) const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" fill="#22aa66"/></svg>"##;

pub(super) fn release(fake: &FakeGitHub, version: &str, image: &str) -> String {
    let compose = compose(version, image);
    fake.publish(
        "acme/hello",
        &format!("v{version}"),
        &[("docker-compose.yml", &compose), ("icon.svg", ICON)],
    )
}

pub(super) fn zh() -> Languages {
    Languages::from_header(Some("zh"))
}

#[tokio::test]
async fn a_source_without_a_version_installs_the_latest_release_and_remembers_it() {
    let (github, client) = FakeGitHub::start().await;
    release(&github, "1.0.0", "nginx");
    let commit = release(&github, "1.1.0", "nginx");
    github.publish(
        "acme/hello",
        "v2.0.0-beta.1",
        &[("docker-compose.yml", &compose("2.0.0-beta.1", "nginx"))],
    );
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine.clone(), &dir).with_github(client.clone()));

    let preview = m.preview_source("github:acme/hello", &zh()).await.unwrap();
    assert_eq!(preview.source.as_deref(), Some("github:acme/hello@v1.1.0"));
    assert_eq!(preview.commit.as_deref(), Some(commit.as_str()));
    assert_eq!(preview.version, "1.1.0");
    assert_eq!(preview.trust, PluginTrust::Community);
    assert!(preview.notices[0].emphasis, "{:?}", preview.notices);
    assert!(preview.icon.is_some());

    let request = InstallRequest {
        scope: None,
        audience: None,
        source: preview.source.clone().unwrap(),
        digest: preview.digest.clone(),
        plan: preview.plan.clone(),
        grant: Vec::new(),
        enabled_permissions: Vec::new(),
        secrets: Default::default(),
        settings: Default::default(),
        devices: Vec::new(),
        lan_webui: false,
        install_together: Vec::new(),
        connect_plugins: Vec::new(),
        remote_oauth: None,
    };
    m.install_source(&request).await.unwrap();
    let id = PluginId::parse("app.hello").unwrap();
    let installed = m.installed_plugin(&id, &zh()).unwrap();
    assert_eq!(installed.trust, PluginTrust::Community);
    assert_eq!(
        installed.source.as_deref(),
        Some("github:acme/hello@v1.1.0")
    );
    let detail = m.installed_detail(&id, &zh()).unwrap();
    assert_eq!(detail.source.as_deref(), Some("github:acme/hello"));
    assert_eq!(
        detail.repository.as_deref(),
        Some("https://github.com/acme/hello")
    );
    assert!(!docker.containers().is_empty());

    // 核心重启后按记录恢复，来源还在。
    let restarted = manager_with(engine, &dir).with_github(client);
    restarted.restore().await.unwrap();
    let installed = restarted.installed_plugin(&id, &zh()).unwrap();
    assert_eq!(
        installed.source.as_deref(),
        Some("github:acme/hello@v1.1.0")
    );
}

#[tokio::test]
async fn a_tag_moved_after_the_preview_is_not_installed() {
    let (github, client) = FakeGitHub::start().await;
    release(&github, "1.0.0", "nginx");
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir).with_github(client));
    let preview = m
        .preview_source("github:acme/hello@v1.0.0", &zh())
        .await
        .unwrap();

    // 作者把 tag 移到了换了镜像的提交上：主人确认的不是这个。
    release(&github, "1.0.0", "evil");
    let request = InstallRequest {
        scope: None,
        audience: None,
        source: preview.source.unwrap(),
        digest: preview.digest,
        plan: preview.plan,
        grant: Vec::new(),
        enabled_permissions: Vec::new(),
        secrets: Default::default(),
        settings: Default::default(),
        devices: Vec::new(),
        lan_webui: false,
        install_together: Vec::new(),
        connect_plugins: Vec::new(),
        remote_oauth: None,
    };
    let err = m.install_source(&request).await.unwrap_err();
    assert_eq!(err.kind(), "changed", "{err}");
    assert!(m.list().is_empty());
    assert!(docker.containers().is_empty());
}

async fn preview_err(m: &PluginManager, source: &str) -> InstallError {
    m.preview_source(source, &zh()).await.unwrap_err()
}

#[tokio::test]
async fn sources_that_cannot_be_installed_say_why() {
    let (github, client) = FakeGitHub::start().await;
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine.clone(), &dir).with_github(client);

    assert_eq!(
        preview_err(&m, "github:acme/nope").await.kind(),
        "not_found"
    );
    github.publish("acme/hello", "nightly", &[]);
    assert_eq!(
        preview_err(&m, "github:acme/hello").await.kind(),
        "not_found"
    );
    assert_eq!(
        preview_err(&m, "github:acme/hello@v9.9.9").await.kind(),
        "not_found"
    );
    // 清单的版本和 tag 不一致。
    let compose = compose("1.0.1", "nginx");
    github.publish(
        "acme/hello",
        "v1.0.0",
        &[("docker-compose.yml", &compose), ("icon.svg", ICON)],
    );
    assert_eq!(
        preview_err(&m, "github:acme/hello").await.kind(),
        "invalid_input"
    );
    // 引用的图标没提交。
    github.publish("acme/hello", "v1.0.2", &[("docker-compose.yml", &compose)]);
    let commit = github.tags.lock().unwrap()["acme/hello"]["v1.0.2"].clone();
    let err = preview_err(&m, &format!("github:acme/hello@{commit}")).await;
    assert_eq!(err.kind(), "not_found", "{err}");
    assert_eq!(
        preview_err(&m, "gitlab:acme/hello").await.kind(),
        "invalid_input"
    );

    let offline = manager_with(engine, &dir).with_github(GitHub::for_test("http://127.0.0.1:9"));
    assert_eq!(
        preview_err(&offline, "github:acme/hello").await.kind(),
        "source_unavailable"
    );
}
