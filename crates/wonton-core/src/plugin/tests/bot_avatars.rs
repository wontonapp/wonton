use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;
use wonton_protocol::StateAvatars;

use super::super::fake_docker::FakeDocker;
use super::super::package_files::{self, PackageFiles};
use super::super::store::Store;
use super::super::storefront::index::{Files, Index, IndexEntry};
use super::super::{InstallError, Permission, PluginId, PluginManager, PluginPackage};
use super::github::FakeGitHub;
use super::manager::manager_in_container;
use super::package;
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::fake::{Fake, start};
use crate::plugin::languages::Languages;

const PNG: &[u8] = include_bytes!("../../../tests/fixtures/avatars/square.png");
const JPEG: &[u8] = include_bytes!("../../../tests/fixtures/avatars/square.jpg");
const GIF: &[u8] = include_bytes!("../../../tests/fixtures/avatars/animated.gif");

fn source(avatars: &str) -> String {
    package(
        "  bot:\n    image: echo\n    environment:\n      TOKEN: ${WONTON_MATRIX_ACCESS_TOKEN}",
        &format!("  permissions: [matrix.bot]\n  bot:\n    avatars: {avatars}"),
    )
}

fn bundle(avatars: &str, files: &[(&str, &[u8])]) -> PluginPackage {
    PluginPackage::with_files(
        &source(avatars),
        files
            .iter()
            .map(|(p, b)| (p.to_string(), b.to_vec()))
            .collect(),
        &super::langs(),
    )
    .unwrap()
}

fn id() -> PluginId {
    PluginId::parse("app.test").unwrap()
}
fn grant() -> Vec<Permission> {
    vec![Permission::MatrixBot]
}

struct Bed {
    fake: Fake,
    matrix: Arc<Matrix>,
    manager: Arc<PluginManager>,
    _docker: FakeDocker,
    dir: tempfile::TempDir,
}

impl Bed {
    async fn start() -> Self {
        let (docker, engine) = FakeDocker::start().await;
        let (fake, cfg) = start().await;
        let dir = tempfile::tempdir().unwrap();
        let matrix = Arc::new(
            Matrix::connect(&cfg, &dir.path().join("matrix"), HttpClient::new())
                .await
                .unwrap(),
        );
        let manager = manager_in_container(engine, &dir).with_matrix(Some(matrix.clone()));
        Self {
            fake,
            matrix,
            manager: Arc::new(manager),
            _docker: docker,
            dir,
        }
    }

    async fn avatars(&self) -> Option<StateAvatars> {
        self.matrix
            .as_bot("@test:test", async |session| session.avatars().await)
            .await
            .unwrap()
    }
}

#[test]
fn avatar_paths_are_normalized_and_optional_posters_survive_every_package_form() {
    let source = source("{idle: ./avatars//idle.mp4}");
    assert_eq!(
        PluginPackage::referenced_files(&source, &super::langs()).unwrap(),
        ["avatars/idle.mp4", "avatars/idle.jpg"]
    );
    assert_eq!(
        PluginPackage::optional_files(&source, &super::langs()).unwrap(),
        ["avatars/idle.jpg"]
    );
    for poster in [false, true] {
        let mut files = PackageFiles::from([("avatars/idle.mp4".into(), PNG.to_vec())]);
        if poster {
            files.insert("avatars/idle.jpg".into(), JPEG.to_vec());
        }
        let package = PluginPackage::with_files(&source, files.clone(), &super::langs()).unwrap();
        assert_eq!(package.files(), &files);
        assert_eq!(
            PluginPackage::from_archive(&package.archive(), &super::langs())
                .unwrap()
                .files(),
            &files
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("avatars")).unwrap();
        std::fs::write(dir.path().join("docker-compose.yml"), &source).unwrap();
        for (path, data) in &files {
            std::fs::write(dir.path().join(path), data).unwrap();
        }
        assert_eq!(
            PluginPackage::load_dir(dir.path(), &super::langs())
                .unwrap()
                .files(),
            &files
        );
    }
}

#[test]
fn invalid_required_paths_and_unsafe_or_oversized_optional_posters_are_rejected() {
    for path in ["../idle.mp4", "/idle.mp4", "${IDLE}", ""] {
        assert!(
            PluginPackage::with_files(
                &source(&format!("{{idle: '{path}'}}")),
                BTreeMap::new(),
                &super::langs()
            )
            .is_err()
        );
    }
    assert!(PluginPackage::parse(&source("{idle: idle.mp4}"), &super::langs()).is_err());
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("docker-compose.yml"),
        source("{idle: idle.mp4}"),
    )
    .unwrap();
    std::fs::write(dir.path().join("idle.mp4"), PNG).unwrap();
    std::os::unix::fs::symlink("idle.mp4", dir.path().join("idle.jpg")).unwrap();
    assert!(PluginPackage::load_dir(dir.path(), &super::langs()).is_err());
    let files = PackageFiles::from([
        ("idle.mp4".into(), PNG.to_vec()),
        (
            "idle.jpg".into(),
            vec![0; usize::try_from(package_files::MAX_FILE_BYTES + 1).unwrap()],
        ),
    ]);
    assert!(
        PluginPackage::with_files(&source("{idle: idle.mp4}"), files, &super::langs()).is_err()
    );
}

#[tokio::test]
async fn index_recognizes_defaults_and_loads_only_declared_optional_posters() {
    for poster in [false, true] {
        let mut files = vec![("idle.mp4", PNG)];
        if poster {
            files.push(("idle.jpg", JPEG));
        }
        let package = bundle("{idle: idle.mp4}", &files);
        let entry = IndexEntry::of(&package, "github:example/bot", &"a".repeat(40)).unwrap();
        let index = Index::parse(
            &serde_json::to_vec(&json!({"format": 1, "plugins": [entry.clone()]})).unwrap(),
            &Languages::default(),
        )
        .unwrap();
        assert!(index.skipped.is_empty());
        let dir = tempfile::tempdir().unwrap();
        entry.write_to(dir.path(), &package).unwrap();
        let loaded = Files::Dir(dir.path().to_owned())
            .package(&index.plugins[0], &Languages::default())
            .await
            .unwrap();
        assert_eq!(loaded.files(), package.files());
        if poster {
            std::fs::remove_file(dir.path().join("blobs").join(&entry.files["idle.jpg"])).unwrap();
            assert!(
                Files::Dir(dir.path().to_owned())
                    .package(&index.plugins[0], &Languages::default())
                    .await
                    .is_err()
            );
        }
    }
}

#[tokio::test]
async fn github_loads_optional_posters_but_does_not_swallow_present_file_errors() {
    use super::super::github_source::{SourceRef, fetch, resolve};
    for poster in [
        None,
        Some("poster".to_owned()),
        Some("x".repeat(usize::try_from(package_files::MAX_FILE_BYTES + 1).unwrap())),
    ] {
        let (fake, client) = FakeGitHub::start().await;
        let source = source("{idle: idle.mp4}");
        let mut files = vec![
            ("docker-compose.yml", source.as_str()),
            ("idle.mp4", "media"),
        ];
        if let Some(poster) = &poster {
            files.push(("idle.jpg", poster));
        }
        fake.publish("example/bot", "v1.0.0", &files);
        let origin = resolve(
            &client,
            &SourceRef::parse("github:example/bot", &Languages::default()).unwrap(),
            &Languages::default(),
        )
        .await
        .unwrap();
        let result = fetch(&client, &origin, &Languages::default()).await;
        if poster
            .as_ref()
            .is_some_and(|p| p.len() as u64 > package_files::MAX_FILE_BYTES)
        {
            assert!(matches!(result, Err(InstallError::Invalid(_))));
        } else {
            assert_eq!(
                result.unwrap().files().contains_key("idle.jpg"),
                poster.is_some()
            );
        }
    }
}

#[tokio::test]
async fn defaults_upload_as_bot_and_preserve_any_existing_field_including_empty() {
    let bed = Bed::start().await;
    let package = bundle(
        "{idle: idle.mp4, working: working.png}",
        &[("idle.mp4", PNG), ("idle.jpg", JPEG), ("working.png", PNG)],
    );
    bed.manager
        .install_package(package.clone(), &grant(), super::langs())
        .await
        .unwrap();
    let initial = bed.avatars().await.unwrap();
    assert_eq!(initial.states.len(), 2);
    assert_eq!(initial.states["idle"].mimetype, "image/png");
    assert!(initial.states["idle"].poster.is_some());
    assert_eq!(bed.fake.state().media.len(), 3);
    assert_eq!(bed.fake.state().devices("@test:test").len(), 1);
    assert_eq!(bed.manager.profile_revisions().get("app.test"), 1);
    let invalid = bundle("{idle: idle.mp4}", &[("idle.mp4", b"invalid")]);
    bed.manager
        .fill_default_avatars(&invalid, "@test:test")
        .await
        .unwrap();
    assert_eq!(bed.avatars().await, Some(initial));
    bed.matrix
        .as_bot("@test:test", async |session| {
            session.set_avatars(&StateAvatars::default()).await
        })
        .await
        .unwrap();
    bed.manager
        .uninstall(&id(), false, &super::langs())
        .await
        .unwrap();
    bed.manager
        .install_package(invalid, &grant(), super::langs())
        .await
        .unwrap();
    assert_eq!(bed.avatars().await, Some(StateAvatars::default()));
    assert_eq!(bed.fake.state().media.len(), 3);
    assert_eq!(bed.manager.profile_revisions().get("app.test"), 1);
    bed.fake
        .state()
        .users
        .get_mut("@test:test")
        .unwrap()
        .avatars = Some(json!({"future_shape":true}));
    bed.manager
        .fill_default_avatars(&package, "@test:test")
        .await
        .unwrap();
    assert_eq!(
        bed.fake.state().user("@test:test").avatars,
        Some(json!({"future_shape":true}))
    );
    assert_eq!(bed.fake.state().media.len(), 3);
}

#[tokio::test]
async fn invalid_default_or_poster_fails_before_any_upload_and_retry_reuses_account() {
    for (state, poster) in [
        (b"invalid".as_slice(), JPEG),
        (PNG, b"invalid".as_slice()),
        (PNG, GIF),
    ] {
        let bed = Bed::start().await;
        let package = bundle(
            "{idle: good.png, working: bad.mp4}",
            &[("good.png", PNG), ("bad.mp4", state), ("bad.jpg", poster)],
        );
        let error = bed
            .manager
            .install_package(package, &grant(), super::langs())
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::Invalid(_)));
        assert!(error.to_string().contains("bad."));
        assert!(bed.fake.state().media.is_empty());
        assert!(bed.fake.state().devices("@test:test").is_empty());
        assert!(!bed.fake.state().user("@test:test").deactivated);
        let store = Store::open(bed.dir.path().join("plugins")).unwrap();
        assert!(
            store
                .bots()
                .load(&id())
                .unwrap()
                .unwrap()
                .device_id
                .is_none()
        );
        assert_eq!(bed.manager.profile_revisions().get("app.test"), 0);
        bed.manager
            .install_package(
                bundle("{idle: good.png}", &[("good.png", PNG)]),
                &grant(),
                super::langs(),
            )
            .await
            .unwrap();
        assert!(bed.avatars().await.unwrap().states.contains_key("idle"));
        assert_eq!(bed.fake.state().devices("@test:test").len(), 1);
    }
}

#[tokio::test]
async fn upgrade_fills_missing_defaults_without_replacing_the_running_bot_device() {
    let bed = Bed::start().await;
    let old = super::package(
        "  bot:\n    image: echo\n    environment:\n      TOKEN: ${WONTON_MATRIX_ACCESS_TOKEN}",
        "  permissions: [matrix.bot]",
    );
    bed.manager
        .install(&old, &grant(), super::langs())
        .await
        .unwrap();
    let before = bed.fake.state().devices("@test:test");
    assert_eq!(bed.avatars().await, None);
    let invalid = bundle("{idle: idle.mp4}", &[("idle.mp4", b"invalid")]);
    let invalid = PluginPackage::with_files(
        &invalid.source.replace("1.0.0", "1.1.0"),
        invalid.files().clone(),
        &super::langs(),
    )
    .unwrap();
    assert!(matches!(
        bed.manager
            .upgrade_upload(&id(), invalid, grant(), false, &super::langs())
            .await,
        Err(InstallError::Invalid(_))
    ));
    assert_eq!(bed.fake.state().devices("@test:test"), before);
    assert!(bed.fake.state().media.is_empty());
    assert_eq!(bed.avatars().await, None);
    let next = bundle("{idle: idle.mp4}", &[("idle.mp4", PNG)]);
    let next = PluginPackage::with_files(
        &next.source.replace("1.0.0", "1.1.0"),
        next.files().clone(),
        &super::langs(),
    )
    .unwrap();
    bed.manager
        .upgrade_upload(&id(), next, grant(), false, &super::langs())
        .await
        .unwrap();
    assert!(bed.avatars().await.unwrap().states.contains_key("idle"));
    assert_eq!(bed.fake.state().devices("@test:test"), before);
    assert_eq!(bed.manager.profile_revisions().get("app.test"), 1);
    bed.matrix
        .as_bot("@test:test", async |session| {
            session.set_avatars(&StateAvatars::default()).await
        })
        .await
        .unwrap();
    let next = bundle("{idle: idle.mp4}", &[("idle.mp4", b"invalid")]);
    let next = PluginPackage::with_files(
        &next.source.replace("1.0.0", "1.2.0"),
        next.files().clone(),
        &super::langs(),
    )
    .unwrap();
    bed.manager
        .upgrade_upload(&id(), next, grant(), false, &super::langs())
        .await
        .unwrap();
    assert_eq!(bed.avatars().await, Some(StateAvatars::default()));
    assert_eq!(bed.fake.state().devices("@test:test"), before);
    assert_eq!(bed.fake.state().media.len(), 1);
    assert_eq!(bed.manager.profile_revisions().get("app.test"), 1);
}

#[tokio::test]
async fn unavailable_matrix_does_not_turn_into_invalid_media_or_touch_existing_device() {
    let bed = Bed::start().await;
    bed.manager
        .install_package(bundle("{}", &[]), &grant(), super::langs())
        .await
        .unwrap();
    let devices = bed.fake.state().devices("@test:test");
    bed.fake.state().refuse_profile = true;
    let error = bed
        .manager
        .fill_default_avatars(
            &bundle("{idle: idle.mp4}", &[("idle.mp4", PNG)]),
            "@test:test",
        )
        .await
        .unwrap_err();
    assert!(matches!(error, InstallError::MatrixUnavailable(_)));
    assert_eq!(bed.fake.state().devices("@test:test"), devices);
    assert!(bed.fake.state().media.is_empty());
    assert_eq!(bed.manager.profile_revisions().get("app.test"), 1);
}

#[tokio::test]
async fn an_optional_poster_transport_failure_still_fails_package_loading() {
    use super::super::github::GitHub;
    use super::super::github_source::{SourceRef, fetch, resolve};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    let compose = source("{idle: idle.mp4}");
    let app = axum::Router::new().fallback(move |request: axum::extract::Request| {
        let compose = compose.clone();
        async move {
            if request.uri().path().ends_with("docker-compose.yml") {
                compose.into_response()
            } else if request.uri().path().ends_with("idle.mp4") {
                "media".into_response()
            } else {
                StatusCode::SERVICE_UNAVAILABLE.into_response()
            }
        }
    });
    let addr = super::egress::serve(app).await;
    let client = GitHub::for_test(&format!("http://{addr}"));
    let source = SourceRef::parse(
        &format!("github:example/bot@{}", "a".repeat(40)),
        &Languages::default(),
    )
    .unwrap();
    let origin = resolve(&client, &source, &Languages::default())
        .await
        .unwrap();
    assert!(matches!(
        fetch(&client, &origin, &Languages::default()).await,
        Err(InstallError::SourceUnavailable(_))
    ));
}
