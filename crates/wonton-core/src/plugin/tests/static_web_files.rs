//! 静态网页的作者契约（ADR-0080）：`webui`、`tabs` 哪些字段能写，网关给的 CSP，以及 `root` 目录的规则
//! （首页、隐藏文件、软链接、`_wonton/`、`fallback`、项数），插件包归档读回来摘要不变。

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use serde_json::{Value, json};

use super::super::manifest_static::{StaticWebSpec, content_security_policy};
use super::super::package::static_files::MAX_ENTRIES;
use super::super::package::{PackageError, PluginPackage};
use super::super::package_files::PackageFiles;

fn doc(webui: Value) -> Value {
    json!({"x-wonton": {
        "spec": "0.12", "id": "io.github.alice.snake", "version": "1.0.0", "kind": "app",
        "runtime": "static_web", "title": {"en": "Snake"}, "description": {"en": "Snake"},
        "permissions": [], "webui": webui
    }})
}

fn manifest_error(doc: &Value) -> String {
    match PluginPackage::parse(&doc.to_string(), &super::langs()) {
        Err(PackageError::Manifest(error)) => error.to_string(),
        other => panic!("应当是清单错误：{other:?}"),
    }
}

#[test]
fn a_static_page_takes_webui_display_fields_and_tabs() {
    let mut page =
        doc(json!({"path": "/play", "display": "fullscreen", "cross_origin_isolated": true}));
    page["x-wonton"]["tabs"] = json!([{
        "id": "scores", "title": {"zh": "成绩", "en": "Scores"}, "icon": "tabs/scores.svg", "path": "/scores"
    }]);
    let files: PackageFiles = [("tabs/scores.svg".to_owned(), b"<svg/>".to_vec())].into();
    let package = PluginPackage::with_files(&page.to_string(), files, &super::langs()).unwrap();
    let webui = package.manifest.webui.unwrap();
    assert_eq!(webui.path, "/play");
    assert!(webui.cross_origin_isolated);
    assert_eq!(package.manifest.tabs.map(|t| t.len()), Some(1));
}

// 页面本身就是网页：没写 webui 也有打开的入口，tabs 也能单独写。
#[test]
fn a_static_page_without_webui_still_opens_at_the_root() {
    let mut page = doc(json!(null));
    page["x-wonton"].as_object_mut().unwrap().remove("webui");
    let package = PluginPackage::parse(&page.to_string(), &super::langs()).unwrap();
    let webui = package.manifest.webui.unwrap();
    assert_eq!((webui.path.as_str(), webui.service.as_str()), ("/", ""));

    page["x-wonton"]["tabs"] =
        json!([{"id": "a", "title": {"zh": "甲", "en": "A"}, "icon": "a.svg", "path": "/a"}]);
    let files: PackageFiles = [("a.svg".to_owned(), b"<svg/>".to_vec())].into();
    PluginPackage::with_files(&page.to_string(), files, &super::langs()).unwrap();
}

#[test]
fn a_static_page_refuses_webui_fields_that_need_a_server() {
    for (field, webui) in [
        ("webui.service", json!({"service": "web"})),
        ("webui.port", json!({"port": 8080})),
        ("webui.sso", json!({"sso": "proxy"})),
        ("webui.accounts", json!({"accounts": "per_user"})),
    ] {
        let error = manifest_error(&doc(webui));
        assert!(error.contains(field), "{field}: {error}");
    }
}

#[test]
fn a_container_plugin_still_needs_webui_service_and_port() {
    let compose = |webui: &str| {
        format!(
            "services:\n  web:\n    image: nginx\nx-wonton:\n  spec: '0.12'\n  id: app.test\n  \
             version: 1.0.0\n  kind: app\n  title: {{en: T}}\n  description: {{en: T}}\n  \
             permissions: []\n  webui: {webui}\n"
        )
    };
    for webui in ["{port: 8080}", "{service: web}"] {
        let parsed = PluginPackage::parse(&compose(webui), &super::langs());
        assert!(
            matches!(parsed, Err(PackageError::Manifest(_))),
            "{webui}: {parsed:?}"
        );
    }
    PluginPackage::parse(&compose("{service: web, port: 8080}"), &super::langs()).unwrap();
}

// 网页能连哪儿完全由清单的 connect 定；没写就只连自己、不能嵌别的网页。
#[test]
fn the_csp_follows_connect() {
    let none = content_security_policy(&StaticWebSpec::default());
    assert_eq!(
        none,
        "default-src 'self'; script-src 'self' 'unsafe-inline' 'unsafe-eval' blob:; \
         style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; \
         media-src 'self' data: blob:; connect-src 'self'; worker-src 'self' blob:; \
         frame-src 'none'; object-src 'none'; base-uri 'self'; form-action 'self'"
    );
    let spec = StaticWebSpec {
        connect: vec![
            "https://a.example".to_owned(),
            "wss://*.b.example".to_owned(),
        ],
        ..StaticWebSpec::default()
    };
    let some = content_security_policy(&spec);
    for directive in [
        "connect-src 'self' https://a.example wss://*.b.example",
        "img-src 'self' data: blob: https://a.example wss://*.b.example",
        "frame-src https://a.example wss://*.b.example",
    ] {
        assert!(some.contains(directive), "{some}");
    }
}

fn package_dir(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let mut source = doc(json!({"path": "/"}));
    source["x-wonton"]["static_web"] = json!({"root": "dist", "fallback": "index.html"});
    fs::write(dir.path().join("docker-compose.yml"), source.to_string()).unwrap();
    for (path, data) in files {
        let path = dir.path().join("dist").join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }
    dir
}

fn load(dir: &Path) -> Result<PluginPackage, PackageError> {
    PluginPackage::load_dir(dir, &super::langs())
}

#[test]
fn a_package_directory_brings_every_file_under_root() {
    let dir = package_dir(&[("index.html", "<h1>"), ("assets/app.js", "go()")]);
    let package = load(dir.path()).unwrap();
    let files = package.static_files().unwrap();
    assert_eq!(
        files.keys().collect::<Vec<_>>(),
        ["assets/app.js", "index.html"]
    );
    assert_eq!(package.static_hashes().len(), 2);

    // 归档读回来是同一个包：文件一样、摘要一样；改一个文件摘要就变。
    let again = PluginPackage::from_archive(&package.archive(), &super::langs()).unwrap();
    assert_eq!(again.static_files(), package.static_files());
    assert_eq!(again.digest(), package.digest());
    let changed = package_dir(&[("index.html", "<h1>"), ("assets/app.js", "stop()")]);
    assert_ne!(load(changed.path()).unwrap().digest(), package.digest());
}

#[test]
fn a_package_directory_breaking_the_rules_is_refused() {
    let cases: [(&[(&str, &str)], &str); 4] = [
        (&[("app.js", "x")], "index.html"),
        (&[("index.html", "x"), (".env", "secret")], ".env"),
        (&[("index.html", "x"), ("_wonton/data", "x")], "_wonton"),
        (&[("index.html", "x"), ("assets/.cache/a", "x")], ".cache"),
    ];
    for (files, expected) in cases {
        let dir = package_dir(files);
        let error = load(dir.path()).unwrap_err();
        assert!(matches!(error, PackageError::StaticDir(_)), "{error:?}");
        assert!(error.to_string().contains(expected), "{expected}: {error}");
    }
}

#[test]
fn symlinks_under_root_are_refused_not_followed() {
    let dir = package_dir(&[("index.html", "x")]);
    let outside = dir.path().join("docker-compose.yml");
    symlink(&outside, dir.path().join("dist/leak.txt")).unwrap();
    let error = load(dir.path()).unwrap_err();
    assert!(error.to_string().contains("leak.txt"), "{error}");

    let dir = package_dir(&[]);
    fs::remove_dir(dir.path().join("dist")).ok();
    let elsewhere = tempfile::tempdir().unwrap();
    fs::write(elsewhere.path().join("index.html"), "x").unwrap();
    symlink(elsewhere.path(), dir.path().join("dist")).unwrap();
    assert!(matches!(load(dir.path()), Err(PackageError::StaticDir(_))));
}

#[test]
fn too_many_entries_under_root_are_refused() {
    let dir = package_dir(&[("index.html", "x")]);
    for n in 0..MAX_ENTRIES {
        fs::write(dir.path().join(format!("dist/f{n}")), "").unwrap();
    }
    let error = load(dir.path()).unwrap_err();
    assert!(
        error.to_string().contains(&MAX_ENTRIES.to_string()),
        "{error}"
    );
}
