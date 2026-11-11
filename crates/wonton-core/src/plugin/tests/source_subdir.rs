//! 子目录来源：取包带子目录前缀、按前缀选 tag、tag 与清单版本核对、引用文件跳不出子目录、子目录里的第三方索引。

use wonton_protocol::SourceKind;

use super::super::storefront::index::IndexEntry;
use super::github::{ICON, zh};
use super::sources::{REPO, hello, id, install_request, manager, package};

/// 带图标的清单：装子目录的包时引用的文件按子目录前缀去取（完成标准 3）。
fn hello_with_icon(version: &str) -> String {
    hello(version, "").replace("  permissions: []", "  icon: icon.svg\n  permissions: []")
}

#[tokio::test]
async fn a_package_in_a_subdirectory_installs_with_the_same_digest_as_at_the_root() {
    let (github, _docker, m, _dir) = manager().await;
    let subdir = "agents/codex";
    let compose = hello_with_icon("1.0.0");
    github.publish(
        REPO,
        "v1.0.0",
        &[("docker-compose.yml", &compose), ("icon.svg", ICON)],
    );
    github.publish(
        REPO,
        "codex/v1.0.0",
        &[
            (&format!("{subdir}/docker-compose.yml"), &compose),
            (&format!("{subdir}/icon.svg"), ICON),
        ],
    );

    let sub = m
        .fetch_source("github:acme/hello/agents/codex")
        .await
        .unwrap();
    assert_eq!(sub.files()["icon.svg"], ICON.as_bytes());
    assert_eq!(
        sub.files()
            .keys()
            .filter(|path| path.starts_with("agents/"))
            .count(),
        0,
        "files 里的路径相对插件包，不带子目录前缀"
    );
    let root = m.fetch_source("github:acme/hello").await.unwrap();
    assert_eq!(
        sub.digest(),
        root.digest(),
        "同一个包放根目录和子目录，摘要一样"
    );

    let origin = sub.origin.as_ref().unwrap();
    assert_eq!(origin.repo, "github:acme/hello/agents/codex");
    assert_eq!(origin.tag.as_deref(), Some("codex/v1.0.0"));
    assert_eq!(
        origin.repository_url(),
        format!(
            "https://github.com/acme/hello/tree/{}/agents/codex",
            origin.commit
        )
    );
    assert_eq!(origin.provider(), "github.com/acme/hello");

    let preview = m
        .preview_source("github:acme/hello/agents/codex", &zh())
        .await
        .unwrap();
    m.install_source(&install_request(&preview)).await.unwrap();
    let installed = m.installed_plugin(&id(), &zh()).unwrap();
    assert_eq!(installed.provider.as_deref(), Some("github.com/acme/hello"));
}

/// tag 有根目录的和两个子目录的：各自按自己的前缀选（完成标准 4）。
#[tokio::test]
async fn a_subdirectory_selects_its_own_prefixed_tags_and_the_root_is_unchanged() {
    let (github, _docker, m, _dir) = manager().await;
    let subdir = "agents/codex";
    github.publish(
        REPO,
        "v9.0.0",
        &[("docker-compose.yml", &hello("9.0.0", ""))],
    );
    github.publish(
        REPO,
        "codex/v0.2.0",
        &[(&format!("{subdir}/docker-compose.yml"), &hello("0.2.0", ""))],
    );
    github.publish(
        REPO,
        "codex/v0.3.0-beta.1",
        &[(
            &format!("{subdir}/docker-compose.yml"),
            &hello("0.3.0-beta.1", ""),
        )],
    );
    github.publish(
        REPO,
        "kimi-code/v1.0.0",
        &[("kimi-code/docker-compose.yml", &hello("1.0.0", ""))],
    );

    let root = m.fetch_source("github:acme/hello").await.unwrap();
    assert_eq!(root.manifest.version, "9.0.0");
    let sub = m
        .fetch_source("github:acme/hello/agents/codex")
        .await
        .unwrap();
    assert_eq!(
        sub.manifest.version, "0.2.0",
        "预发布的不选，别的子目录的 tag 不算"
    );
}

/// 明确写的 tag 照用，但清单版本要对上（完成标准 5）。
#[tokio::test]
async fn an_explicit_subdirectory_tag_with_a_mismatched_manifest_is_refused() {
    let (github, _docker, m, _dir) = manager().await;
    let subdir = "agents/codex";
    github.publish(
        REPO,
        "codex/v0.2.0",
        &[(&format!("{subdir}/docker-compose.yml"), &hello("0.2.1", ""))],
    );
    let error = m
        .preview_source("github:acme/hello/agents/codex@codex/v0.2.0", &zh())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "invalid_input", "{error}");
    assert!(error.to_string().contains("codex/v0.2.0"), "{error}");
}

/// 子目录插件包里引用 `..` 或绝对路径照样被拒：引用文件跳不出子目录（PR #235 核对意见 ①）。
#[tokio::test]
async fn references_in_a_subdirectory_cannot_escape_it() {
    let (github, _docker, m, _dir) = manager().await;
    let subdir = "agents/codex";
    for reference in ["../secret", "/etc/wonton.conf"] {
        let compose = hello("1.0.0", "").replace(
            "x-wonton:",
            &format!("configs:\n  c:\n    file: {reference}\nx-wonton:"),
        );
        github.publish(
            REPO,
            "codex/v1.0.0",
            &[(&format!("{subdir}/docker-compose.yml"), &compose)],
        );
        let error = m
            .preview_source("github:acme/hello/agents/codex", &zh())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "invalid_input", "{reference}: {error}");
        assert!(error.to_string().contains(reference), "{error}");
    }
}

/// 第三方索引放在仓库子目录里也能加、能装（PR #235 核对意见 ③：补测试）。
#[tokio::test]
async fn an_index_in_a_subdirectory_can_be_added_and_installed_from() {
    let (github, _docker, m, _dir) = manager().await;
    let compose = hello("1.0.0", "");
    let commit = github.publish(REPO, "v1.0.0", &[("docker-compose.yml", &compose)]);
    let entry = IndexEntry::of(&package(&compose), "github:acme/hello", &commit).unwrap();
    let index =
        serde_json::json!({"format": 1, "name": "子目录索引", "plugins": [entry]}).to_string();
    github.publish_at_head("friend/plugins", &[("sub/index.json", &index)]);

    let added = m
        .add_store_source("friend/plugins/sub", &zh())
        .await
        .unwrap();
    assert_eq!(added.kind, SourceKind::Index);
    assert_eq!(added.name.as_deref(), Some("子目录索引"));
    assert_eq!(added.provider, "github.com/friend/plugins");
    let source = added.section.plugins[0].source.clone();
    assert_eq!(
        source,
        "index+https://raw.githubusercontent.com/friend/plugins/HEAD/sub/index.json#app.hello"
    );

    let preview = m.preview_source(&source, &zh()).await.unwrap();
    assert_eq!(preview.provider.as_deref(), Some("github.com/acme/hello"));
    assert_eq!(
        preview.via_index.as_deref(),
        Some("github.com/friend/plugins")
    );
    m.install_source(&install_request(&preview)).await.unwrap();
    assert!(m.installed_plugin(&id(), &zh()).is_some());
}
