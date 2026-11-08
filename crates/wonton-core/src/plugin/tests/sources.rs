//! 四种来源走同一条路（ADR-0031）：上传的包按暂存的来源安装，来源写在确认页上，主人可以主动换版本。

use std::collections::BTreeMap;
use std::sync::Arc;

use wonton_protocol::{InstallPreview, InstallRequest, PluginTrust, SourceKind, UpgradeRequest};

use super::super::fake_docker::FakeDocker;
use super::super::github::GitHub;
use super::super::package_files::PackageFiles;
use super::super::storefront::index::IndexEntry;
use super::super::{PluginId, PluginManager, PluginPackage, UpgradeTo};
use super::github::{FakeGitHub, zh};
use super::manager::manager_with;

const REPO: &str = "acme/hello";

fn hello(version: &str, manifest: &str) -> String {
    format!(
        "services:\n  web:\n    image: ghcr.io/acme/web:{version}\n    volumes: ['data:/data']\nvolumes:\n  data: {{}}\n\
         x-wonton:\n  spec: \"0.2\"\n  id: app.hello\n  version: {version}\n  kind: app\n  title: {{zh: 你好, en: Hello}}\n  \
         description: {{zh: 打个招呼, en: Say hi}}\n  permissions: []\n{manifest}"
    )
}

fn package(compose: &str) -> PluginPackage {
    PluginPackage::parse(compose, &super::langs()).unwrap()
}

fn secret(value: &str) -> wonton_protocol::SecretString {
    wonton_protocol::SecretString::new(value)
}

fn id() -> PluginId {
    PluginId::parse("app.hello").unwrap()
}

async fn manager() -> (
    FakeGitHub,
    FakeDocker,
    Arc<PluginManager>,
    tempfile::TempDir,
) {
    let (github, client): (FakeGitHub, GitHub) = FakeGitHub::start().await;
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir).with_github(client);
    (github, docker, Arc::new(m), dir)
}

fn texts(preview: &InstallPreview) -> Vec<&str> {
    preview.notices.iter().map(|n| n.text.as_str()).collect()
}

fn install_request(preview: &InstallPreview) -> InstallRequest {
    InstallRequest {
        devices: Vec::new(),
        source: preview.source.clone().unwrap(),
        digest: preview.digest.clone(),
        grant: preview
            .permissions
            .iter()
            .filter(|p| !p.optional)
            .map(|p| p.permission.clone())
            .collect(),
        enabled_permissions: Vec::new(),
        secrets: Default::default(),
        lan_webui: false,
        install_together: Vec::new(),
        connect_plugins: Vec::new(),
    }
}

fn upgrade_request(preview: &InstallPreview) -> UpgradeRequest {
    UpgradeRequest {
        devices: Vec::new(),
        source: preview.source.clone(),
        digest: preview.digest.clone(),
        grant: preview
            .permissions
            .iter()
            .filter(|p| !p.optional)
            .map(|p| p.permission.clone())
            .collect(),
        enabled_permissions: Vec::new(),
        secrets: Default::default(),
        lan_webui: false,
    }
}

#[tokio::test]
async fn an_uploaded_package_is_confirmed_and_installed_like_any_other_source() {
    let (_github, _docker, m, _dir) = manager().await;
    let compose = hello("1.0.0", "  author: {name: 张三}\n");
    let preview = m.preview_upload(package(&compose), &zh()).await.unwrap();

    let source = preview.source.clone().unwrap();
    assert_eq!(source, format!("upload:{}", preview.digest));
    assert_eq!(preview.trust, PluginTrust::Local);
    assert_eq!(preview.provider, None);
    let notices = texts(&preview);
    assert!(
        notices[0].contains("你上传的文件") && notices[0].contains("张三"),
        "{notices:?}"
    );
    assert!(preview.notices[0].emphasis);
    assert!(notices[1].contains("ghcr.io/acme/web"), "{notices:?}");
    assert!(!notices[1].contains("1.0.0"), "只写仓库：{notices:?}");

    m.install_source(&install_request(&preview)).await.unwrap();
    let installed = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(installed.trust, PluginTrust::Local);
    assert_eq!(installed.provider, None);

    let gone = m.fetch_source(&source).await.unwrap_err();
    assert_eq!(gone.kind(), "not_found", "装上之后不再暂存：{gone}");
}

#[tokio::test]
async fn a_staged_upload_cannot_be_swapped_after_the_preview() {
    let (_github, _docker, m, _dir) = manager().await;
    let preview = m
        .preview_upload(package(&hello("1.0.0", "")), &zh())
        .await
        .unwrap();
    let mut request = install_request(&preview);
    request.digest = format!("sha256:{}", "0".repeat(64));
    let err = m.install_source(&request).await.unwrap_err();
    assert_eq!(err.kind(), "changed", "{err}");
}

#[tokio::test]
async fn an_unreviewed_preview_names_the_repository_that_provides_it() {
    let (github, _docker, m, _dir) = manager().await;
    let compose = hello("1.0.0", "");
    github.publish(REPO, "v1.0.0", &[("docker-compose.yml", &compose)]);
    let preview = m.preview_source("github:acme/hello", &zh()).await.unwrap();
    assert_eq!(preview.trust, PluginTrust::Community);
    assert_eq!(preview.provider.as_deref(), Some("github.com/acme/hello"));
    assert!(
        texts(&preview)[0].contains("github.com/acme/hello"),
        "{:?}",
        texts(&preview)
    );
}

#[tokio::test]
async fn leftover_data_under_the_same_id_is_called_out_before_installing() {
    let (_github, _docker, m, _dir) = manager().await;
    let first = m
        .preview_upload(package(&hello("1.0.0", "")), &zh())
        .await
        .unwrap();
    assert!(!texts(&first).iter().any(|t| t.contains("留着")));
    m.install_source(&install_request(&first)).await.unwrap();
    m.uninstall_detached(id(), false, super::langs())
        .await
        .unwrap();

    let again = m
        .preview_upload(package(&hello("1.0.0", "  author: {name: 别人}\n")), &zh())
        .await
        .unwrap();
    let notice = again
        .notices
        .iter()
        .find(|n| n.text.contains("留着"))
        .expect("要提示留着的数据");
    assert!(notice.emphasis);
    assert!(again.blocked.is_none(), "只提示，不拦");
}

#[tokio::test]
async fn the_owner_can_go_back_to_an_older_version_after_a_warning() {
    let (_github, _docker, m, _dir) = manager().await;
    let v2 = m
        .preview_upload(package(&hello("2.0.0", "")), &zh())
        .await
        .unwrap();
    m.install_source(&install_request(&v2)).await.unwrap();

    let older = package(&hello("1.0.0", ""));
    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Upload(Box::new(older)), &zh())
        .await
        .unwrap();
    assert_eq!(preview.upgrade_from.as_deref(), Some("2.0.0"));
    assert!(preview.notices[0].emphasis);
    assert!(
        preview.notices[0].text.contains("旧"),
        "{:?}",
        texts(&preview)
    );

    m.upgrade_source(&id(), &upgrade_request(&preview), &super::langs())
        .await
        .unwrap();
    assert_eq!(m.installed_plugin(&id(), &zh()).unwrap().version, "1.0.0");
}

#[tokio::test]
async fn the_same_package_is_refused_and_a_changed_one_with_the_same_version_is_not() {
    let (_github, _docker, m, _dir) = manager().await;
    let compose = hello("1.0.0", "");
    let first = m.preview_upload(package(&compose), &zh()).await.unwrap();
    m.install_source(&install_request(&first)).await.unwrap();

    let same = m
        .preview_upgrade(&id(), UpgradeTo::Upload(Box::new(package(&compose))), &zh())
        .await
        .unwrap_err();
    assert_eq!(same.kind(), "invalid_input", "{same}");

    let rebuilt = package(&hello("1.0.0", "  author: {name: 开发中}\n"));
    m.preview_upgrade(&id(), UpgradeTo::Upload(Box::new(rebuilt)), &zh())
        .await
        .unwrap();

    let other = hello("1.1.0", "").replace("id: app.hello", "id: app.other");
    let err = m
        .preview_upgrade(&id(), UpgradeTo::Upload(Box::new(package(&other))), &zh())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "invalid_input", "{err}");
}

#[tokio::test]
async fn switching_an_upload_to_a_repository_starts_following_that_repository() {
    let (github, _docker, m, _dir) = manager().await;
    let first = m
        .preview_upload(package(&hello("1.0.0", "")), &zh())
        .await
        .unwrap();
    m.install_source(&install_request(&first)).await.unwrap();

    let v2 = hello("2.0.0", "");
    github.publish(REPO, "v2.0.0", &[("docker-compose.yml", &v2)]);
    let preview = m
        .preview_upgrade(
            &id(),
            UpgradeTo::Source("https://github.com/acme/hello".to_owned()),
            &zh(),
        )
        .await
        .unwrap();
    assert_eq!(preview.source.as_deref(), Some("github:acme/hello@v2.0.0"));
    let switched = preview
        .notices
        .iter()
        .find(|n| n.text.contains("来源从"))
        .expect("来源变了要提示");
    assert!(switched.emphasis);
    assert!(switched.text.contains("你上传的文件"), "{}", switched.text);
    m.upgrade_source(&id(), &upgrade_request(&preview), &super::langs())
        .await
        .unwrap();
    let installed = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(installed.trust, PluginTrust::Community);
    assert_eq!(installed.provider.as_deref(), Some("github.com/acme/hello"));

    let v3 = hello("3.0.0", "");
    github.publish(REPO, "v3.0.0", &[("docker-compose.yml", &v3)]);
    m.check_upgrades_now().await;
    let offer = m.installed_plugin(&id(), &zh()).unwrap().upgrade.unwrap();
    assert_eq!(offer.version, "3.0.0");
}

#[tokio::test]
async fn the_same_content_can_only_be_switched_to_from_another_source() {
    let (github, _docker, m, _dir) = manager().await;
    let compose = hello("1.0.0", "");
    let first = m.preview_upload(package(&compose), &zh()).await.unwrap();
    m.install_source(&install_request(&first)).await.unwrap();

    let err = m
        .preview_upgrade(&id(), UpgradeTo::Upload(Box::new(package(&compose))), &zh())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "invalid_input", "{err}");

    github.publish(REPO, "v1.0.0", &[("docker-compose.yml", &compose)]);
    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Source("acme/hello".to_owned()), &zh())
        .await
        .unwrap();
    assert!(texts(&preview).iter().any(|t| t.contains("来源从")));
    m.upgrade_source(&id(), &upgrade_request(&preview), &super::langs())
        .await
        .unwrap();
    let installed = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(installed.provider.as_deref(), Some("github.com/acme/hello"));
}

/// 仓库 `repo` 里的第三方索引只列 `plugin_repo` 的这一个版本，返回插件的来源。
fn third_party_index(github: &FakeGitHub, repo: &str, plugin_repo: &str, version: &str) -> String {
    let compose = hello(version, "");
    let tag = format!("v{version}");
    let commit = github.publish(plugin_repo, &tag, &[("docker-compose.yml", &compose)]);
    let entry = IndexEntry::of(
        &package(&compose),
        &format!("github:{plugin_repo}"),
        &commit,
    )
    .unwrap();
    let index =
        serde_json::json!({"format": 1, "name": "朋友的插件", "plugins": [entry]}).to_string();
    github.publish_at_head(repo, &[("index.json", &index)]);
    format!("index+https://raw.githubusercontent.com/{repo}/HEAD/index.json#app.hello")
}

#[tokio::test]
async fn an_added_repository_is_listed_once_under_added_sources() {
    let (github, _docker, m, _dir) = manager().await;
    let compose = hello("1.0.0", "");
    github.publish(REPO, "v1.0.0", &[("docker-compose.yml", &compose)]);
    github.publish_at_head(REPO, &[("docker-compose.yml", &compose)]);
    *github.topic.lock().unwrap() = vec![(REPO.to_owned(), 3)];

    let added = m
        .add_store_source("https://github.com/acme/hello.git", &zh())
        .await
        .unwrap();
    assert_eq!(added.address, "github:acme/hello");
    assert_eq!(added.kind, SourceKind::Repo);
    assert_eq!(added.provider, "github.com/acme/hello");
    assert_eq!(added.section.plugins[0].id, "app.hello");
    assert_eq!(added.section.plugins[0].trust, PluginTrust::Community);

    let store = m.store_listing(&zh(), false).await;
    assert_eq!(store.added.len(), 1);
    assert!(store.community.plugins.is_empty(), "不在社区里再列一次");

    let again = m.add_store_source("acme/hello", &zh()).await.unwrap_err();
    assert_eq!(again.kind(), "conflict", "{again}");

    m.remove_store_source("github:acme/hello", &zh()).unwrap();
    assert!(m.store_listing(&zh(), false).await.added.is_empty());
}

#[tokio::test]
async fn added_sources_survive_a_restart() {
    let (github, client): (FakeGitHub, GitHub) = FakeGitHub::start().await;
    let (_first_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir).with_github(client.clone());
    let compose = hello("1.0.0", "");
    github.publish(REPO, "v1.0.0", &[("docker-compose.yml", &compose)]);
    github.publish_at_head(REPO, &[("docker-compose.yml", &compose)]);
    m.add_store_source("acme/hello", &zh()).await.unwrap();

    let (_docker, engine) = FakeDocker::start().await;
    let restarted = manager_with(engine, &dir).with_github(client);
    let store = restarted.store_listing(&zh(), false).await;
    assert_eq!(store.added[0].address, "github:acme/hello");
    assert_eq!(store.added[0].section.plugins[0].id, "app.hello");
}

#[tokio::test]
async fn addresses_that_are_not_a_plugin_or_an_index_are_refused_with_the_reason() {
    let (github, _docker, m, _dir) = manager().await;
    github.publish_at_head("acme/empty", &[("README.md", "你好")]);
    let empty = m.add_store_source("acme/empty", &zh()).await.unwrap_err();
    assert_eq!(empty.kind(), "invalid_input", "{empty}");

    let plain = m
        .add_store_source("http://example.com/index.json", &zh())
        .await
        .unwrap_err();
    assert_eq!(plain.kind(), "invalid_input", "{plain}");

    let compose = hello("1.0.0", "");
    github.publish_at_head(REPO, &[("docker-compose.yml", &compose)]);
    github
        .tags
        .lock()
        .unwrap()
        .insert(REPO.to_owned(), BTreeMap::new());
    let untagged = m.add_store_source(REPO, &zh()).await.unwrap_err();
    assert!(untagged.to_string().contains("没有正式版本"), "{untagged}");

    assert!(
        m.store_listing(&zh(), false).await.added.is_empty(),
        "失败的不写下来"
    );
}

#[tokio::test]
async fn a_plugin_from_a_third_party_index_is_unreviewed_and_only_follows_that_index() {
    let (github, _docker, m, _dir) = manager().await;
    let source = third_party_index(&github, "friend/plugins", REPO, "1.0.0");

    let added = m.add_store_source("friend/plugins", &zh()).await.unwrap();
    assert_eq!(added.kind, SourceKind::Index);
    assert_eq!(added.name.as_deref(), Some("朋友的插件"));
    assert_eq!(added.provider, "github.com/friend/plugins");
    let item = &added.section.plugins[0];
    assert_eq!(item.source, source);
    assert_eq!(item.via_index.as_deref(), Some("github.com/friend/plugins"));

    let detail = m.store_plugin(&source, &zh()).await.unwrap();
    assert_eq!(detail.source.as_deref(), Some(source.as_str()));

    let preview = m.preview_source(&source, &zh()).await.unwrap();
    assert_eq!(preview.trust, PluginTrust::Community);
    assert_eq!(preview.provider.as_deref(), Some("github.com/acme/hello"));
    assert_eq!(
        preview.via_index.as_deref(),
        Some("github.com/friend/plugins")
    );
    m.install_source(&install_request(&preview)).await.unwrap();

    // 仓库打了新 tag，索引没收：不提示。
    let v2 = hello("2.0.0", "");
    github.publish(REPO, "v2.0.0", &[("docker-compose.yml", &v2)]);
    m.check_upgrades_now().await;
    assert!(m.installed_plugin(&id(), &zh()).unwrap().upgrade.is_none());

    third_party_index(&github, "friend/plugins", REPO, "3.0.0");
    m.check_upgrades_now().await;
    let offer = m.installed_plugin(&id(), &zh()).unwrap().upgrade.unwrap();
    assert_eq!(offer.version, "3.0.0");
    let upgrade = m
        .preview_upgrade(&id(), UpgradeTo::Offered, &zh())
        .await
        .unwrap();
    assert_eq!(upgrade.source.as_deref(), Some(source.as_str()));
    m.upgrade_source(&id(), &upgrade_request(&upgrade), &super::langs())
        .await
        .unwrap();
    let installed = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(installed.version, "3.0.0");
    assert_eq!(
        installed.via_index.as_deref(),
        Some("github.com/friend/plugins")
    );

    // 索引删了：插件照常，不再提示，并且说明原因。
    third_party_index(&github, "friend/plugins", REPO, "4.0.0");
    m.remove_store_source(&added.address, &zh()).unwrap();
    m.check_upgrades_now().await;
    let installed = m.installed_plugin(&id(), &zh()).unwrap();
    assert!(installed.upgrade.is_none());
    assert!(
        installed.notices.iter().any(|n| {
            n.text.contains(&super::message(
                "plugin-settings-upgrade-index-removed",
                &[("provider", "github.com/friend/plugins")],
            ))
        }),
        "{:?}",
        installed.notices
    );
}

#[tokio::test]
async fn a_changed_file_in_a_third_party_index_is_refused() {
    let (github, _docker, m, _dir) = manager().await;
    let config = "a: 1\n";
    let compose = hello("1.0.0", "").replace(
        "x-wonton:",
        "configs:\n  c:\n    file: ./config.yaml\nx-wonton:",
    );
    let commit = github.publish(
        REPO,
        "v1.0.0",
        &[("docker-compose.yml", &compose), ("config.yaml", config)],
    );
    let mut files = PackageFiles::new();
    files.insert("config.yaml".to_owned(), config.as_bytes().to_vec());
    let good = PluginPackage::with_files(&compose, files, &super::langs()).unwrap();
    let entry = IndexEntry::of(&good, "github:acme/hello", &commit).unwrap();
    // 索引旁边的 blob 被换了，也不会回退到别的内容：对不上 sha256 就不装。
    let blob = format!("blobs/{}", entry.files["config.yaml"]);
    let index = serde_json::json!({"format": 1, "plugins": [entry]}).to_string();
    github.publish_at_head(
        "friend/plugins",
        &[("index.json", &index), (blob.as_str(), "a: 2\n")],
    );
    let added = m.add_store_source("friend/plugins", &zh()).await.unwrap();
    let source = &added.section.plugins[0].source;
    let err = m.preview_source(source, &zh()).await.unwrap_err();
    assert_eq!(err.kind(), "invalid_input", "{err}");
}

/// 带一项主人填写的密钥的清单，服务引用它的变量。
fn hello_with_input(version: &str, required: bool) -> String {
    let secrets = format!(
        "  secrets:\n    token:\n      input:\n        title: {{zh: 令牌, en: Token}}\n        url: https://example.com/keys\n        required: {required}\n"
    );
    format!(
        "services:\n  web:\n    image: ghcr.io/acme/web:{version}\n    environment:\n      TOKEN: ${{WONTON_SECRET_TOKEN}}\n\
         x-wonton:\n  spec: \"0.8\"\n  id: app.hello\n  version: {version}\n  kind: app\n  title: {{zh: 你好, en: Hello}}\n  \
         description: {{zh: 打个招呼, en: Say hi}}\n  permissions: []\n{secrets}"
    )
}

async fn preview_of(
    m: &Arc<PluginManager>,
    compose: &str,
) -> (wonton_protocol::InstallPreview, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let preview = m
        .preview_upload(
            PluginPackage::parse(compose, &super::langs()).unwrap(),
            &zh(),
        )
        .await
        .unwrap();
    (preview, dir)
}

#[tokio::test]
async fn installing_without_a_required_input_names_it() {
    let (_github, _docker, m, _dir) = manager().await;
    let (preview, _stage) = preview_of(&m, &hello_with_input("1.0.0", true)).await;
    assert_eq!(preview.secret_inputs.len(), 1);
    assert!(preview.secret_inputs[0].required);
    let err = m
        .install_source(&install_request(&preview))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "invalid_input", "{err}");
    assert!(err.to_string().contains("token"), "{err}");
    assert!(
        m.installed_plugin(&id(), &zh()).is_none(),
        "缺必填项时没装上"
    );
}

#[tokio::test]
async fn an_unknown_key_is_rejected() {
    let (_github, _docker, m, _dir) = manager().await;
    let (preview, _stage) = preview_of(&m, &hello_with_input("1.0.0", true)).await;
    let mut request = install_request(&preview);
    request.secrets = [
        ("token".to_owned(), secret("x")),
        ("nope".to_owned(), secret("y")),
    ]
    .into();
    let err = m.install_source(&request).await.unwrap_err();
    assert_eq!(err.kind(), "invalid_input", "{err}");
    assert!(err.to_string().contains("nope"), "{err}");
}

#[tokio::test]
async fn the_value_reaches_the_container_but_not_the_detail() {
    let (_github, docker, m, _dir) = manager().await;
    let (preview, _stage) = preview_of(&m, &hello_with_input("1.0.0", true)).await;
    let mut request = install_request(&preview);
    request.secrets = [("token".to_owned(), secret("s3cret"))].into();
    m.install_source(&request).await.unwrap();
    let detail = m.judged_detail(&id(), &zh()).await.unwrap();
    let line = detail
        .secret_inputs
        .iter()
        .find(|l| l.name == "token")
        .unwrap();
    assert!(line.filled);
    assert!(line.changed_at_ms.is_some());
    assert!(!serde_json::to_string(&detail).unwrap().contains("s3cret"));
    let web = super::super::names::container_name("app-hello", "web");
    assert!(
        docker.env(&web).iter().any(|e| e == "TOKEN=s3cret"),
        "{:?}",
        docker.env(&web)
    );
}

#[tokio::test]
async fn a_reinstall_reuses_the_filled_value() {
    let (_github, _docker, m, _dir) = manager().await;
    let compose = hello_with_input("1.0.0", true);
    let (preview, _stage) = preview_of(&m, &compose).await;
    let mut request = install_request(&preview);
    request.secrets = [("token".to_owned(), secret("kept"))].into();
    m.install_source(&request).await.unwrap();
    m.uninstall(&id(), false, &zh()).await.unwrap();
    let (again, _stage) = preview_of(&m, &compose).await;
    m.install_source(&install_request(&again)).await.unwrap();
    let detail = m.judged_detail(&id(), &zh()).await.unwrap();
    assert!(detail.secret_inputs[0].filled, "填过的沿用");
}

#[tokio::test]
async fn an_upgrade_only_asks_for_new_and_unfilled_inputs() {
    let (github, _docker, m, _dir) = manager().await;
    let v1 = hello_with_input("1.0.0", true).replace(
        "  secrets:",
        "  secrets:\n    other:\n      input:\n        title: {en: Other}\n        required: false\n",
    );
    github.publish(REPO, "v1.0.0", &[("docker-compose.yml", &v1)]);
    let (preview, _stage) = preview_of(&m, &v1).await;
    let mut first = install_request(&preview);
    first.secrets = [("token".to_owned(), secret("kept"))].into();
    m.install_source(&first).await.unwrap();
    let v2 = v1.replace("app.hello\n  version: 1.0.0", "app.hello\n  version: 1.1.0");
    let _ = github;
    let asked = m
        .preview_upgrade(
            &id(),
            super::super::UpgradeTo::Upload(Box::new(package(&v2))),
            &zh(),
        )
        .await
        .unwrap();
    let names: Vec<_> = asked
        .secret_inputs
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    assert!(
        !names.contains(&"token"),
        "还没填的 token 已经填过：{names:?}"
    );
    assert!(names.len() <= 1, "{names:?}");
}
