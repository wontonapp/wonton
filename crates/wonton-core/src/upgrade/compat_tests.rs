use std::path::Path;
use std::time::Duration;

use wonton_protocol::{CoreUpgradeCompat, CoreUpgradeFailure, PluginLimits};

use super::*;
use crate::plugin::package_files::PackageFiles;
use crate::plugin::store::{InstalledRecord, PluginId, Store};
use crate::upgrade::fake_host::{FakeHost, output};
use crate::upgrade::host::OneShotOutput;
use crate::upgrade::state::Layout;

fn langs() -> Languages {
    Languages::from_header(None)
}

/// 能被当前核心读懂的插件包 compose。
fn package(id: &str, kind: &str) -> String {
    format!(
        "services:\n  web:\n    image: nginx\nx-wonton:\n  spec: '0.12'\n  id: {id}\n  \
         version: 1.0.0\n  kind: {kind}\n  title: {{en: {id}}}\n  \
         description: {{en: {id}}}\n  permissions: []\n"
    )
}

fn record(id: &str, source: &str) -> InstalledRecord {
    InstalledRecord {
        lan_webui: false,
        relays: Vec::new(),
        firewalled: false,
        proxied: false,
        id: PluginId::parse(id).unwrap(),
        version: "1.0.0".into(),
        source: source.to_owned(),
        files: Default::default(),
        limits: PluginLimits::default(),
        granted_permissions: Vec::new(),
        enabled_permissions: Vec::new(),
        oidc_client_secret: None,
        installed_at: 1,
        entry_label: None,
        origin: None,
        written_by: None,
        ask_every_update: false,
        family: false,
        static_files: Default::default(),
        hosted: Vec::new(),
        gpu_vendor: None,
    }
}

struct Scene {
    root: tempfile::TempDir,
    host: std::sync::Arc<FakeHost>,
}

impl Scene {
    /// 记录写进 `<data>/core/plugins/`：compat 从那里读已装插件。
    fn install(&self, id: &str, kind: &str, source: &str) {
        let store =
            Store::open(self.data_dir().join("core").join("plugins")).expect("打开记录目录");
        store.save(&record(id, source)).expect("写记录");
    }

    fn data_dir(&self) -> std::path::PathBuf {
        self.root.path().join("data")
    }

    fn compat_dir(&self) -> std::path::PathBuf {
        self.data_dir().join("core").join("upgrade").join("compat")
    }

    async fn check(&self, digest: &str) -> CoreUpgradeCompat {
        check(
            self.host.as_ref(),
            "reg/core@sha256:new",
            digest,
            &self.data_dir(),
            &langs(),
        )
        .await
    }
}

fn scene() -> Scene {
    let root = tempfile::tempdir().unwrap();
    let layout = Layout {
        install_dir: root.path().join("opt"),
        data_dir: root.path().join("data"),
    };
    let host = std::sync::Arc::new(FakeHost::installed(&layout));
    Scene { root, host }
}

fn reason(category: &str, raw: &str) -> String {
    format!(r#"{{"category":"{category}","raw":"{raw}"}}"#)
}

fn breaking(id: &str, reasons: &[String]) -> String {
    format!(
        r#"{{"id":"{id}","verdict":"breaking","reasons":[{}]}}"#,
        reasons.join(",")
    )
}

fn ok(id: &str) -> String {
    format!(r#"{{"id":"{id}","verdict":"ok"}}"#)
}

fn canned(entries: &[String]) -> OneShotOutput {
    output(
        0,
        &format!(
            r#"{{"spec_supported":"0.12","capabilities":["models"],"plugins":[{}]}}"#,
            entries.join(",")
        ),
        "",
    )
}

/// 只报新失效：目标读不懂、当前核心也读不懂的不列；系统插件不列；现在读得懂、目标读不懂的才列。
#[tokio::test]
async fn only_new_breaking_plugins_are_reported() {
    let s = scene();
    s.install("app.good", "app", &package("app.good", "app"));
    // 现在就异常的：当前核心都读不懂，插件页已经显示，升级不改变它的处境。
    s.install("app.already", "app", "services: [broken");
    // 系统插件跟着核心镜像走，新核心放行后自己会升级。
    s.install("app.system", "system", &package("app.system", "system"));
    *s.host.world().compat = Some(canned(&[
        ok("app.good"),
        breaking("app.already", &[reason("other", "")]),
        breaking("app.system", &[reason("other", "")]),
        breaking(
            "app.good",
            &[reason("spec_unsupported", "0.13"), reason("other", "")],
        ),
    ]));

    let compat = s.check("sha256:new").await;

    let CoreUpgradeCompat::Breaking { plugins } = compat else {
        panic!("{compat:?}");
    };
    assert_eq!(plugins.len(), 1, "{plugins:?}");
    assert_eq!(plugins[0].id, "app.good");
    assert_eq!(plugins[0].reasons.len(), 2, "{plugins:?}");
    // 物化目录读完就删。
    assert!(!s.compat_dir().exists(), "{:?}", s.compat_dir());
}

#[tokio::test]
async fn nothing_installed_runs_no_container() {
    let s = scene();
    assert_eq!(s.check("sha256:new").await, CoreUpgradeCompat::Ok);
    assert!(
        !s.host
            .world()
            .log
            .iter()
            .any(|op| op.contains("wonton-compat")),
        "{:?}",
        s.host.world().log
    );
}

/// 目标镜像太老没有 compat 子命令（退出码 2）：Unavailable，不是"没有会坏的"。
#[tokio::test]
async fn an_old_target_image_is_unavailable() {
    let s = scene();
    s.install("app.good", "app", &package("app.good", "app"));
    assert_eq!(s.check("sha256:new").await, CoreUpgradeCompat::Unavailable);
}

#[tokio::test]
async fn unreadable_output_is_failed() {
    let s = scene();
    s.install("app.good", "app", &package("app.good", "app"));
    *s.host.world().compat = Some(output(0, "不是 JSON", ""));
    let compat = s.check("sha256:new").await;
    let CoreUpgradeCompat::Failed { problem } = compat else {
        panic!("{compat:?}");
    };
    assert_eq!(
        problem.reason,
        wonton_protocol::CoreUpgradeFailure::Upstream
    );
}

#[tokio::test]
async fn a_failed_run_is_failed() {
    let s = scene();
    s.install("app.good", "app", &package("app.good", "app"));
    s.host.world().compat_error = Some("起不来".to_owned());
    assert!(matches!(
        s.check("sha256:new").await,
        CoreUpgradeCompat::Failed { .. }
    ));
    assert!(!s.compat_dir().exists());
}

/// 目标只对这次送查的包裁决：报告里多出来的 id 不是已装插件，忽略。
#[tokio::test]
async fn breaking_for_an_unsent_id_is_ignored() {
    let s = scene();
    s.install("app.good", "app", &package("app.good", "app"));
    *s.host.world().compat = Some(canned(&[breaking("app.other", &[reason("other", "")])]));
    assert_eq!(s.check("sha256:new").await, CoreUpgradeCompat::Ok);
}

/// 类别逐字对照：认得的三个照转，`other` 和不认识的类别都落 Other。
#[tokio::test]
async fn categories_map_by_name_and_fall_to_other() {
    let s = scene();
    s.install("app.good", "app", &package("app.good", "app"));
    *s.host.world().compat = Some(canned(&[breaking(
        "app.good",
        &[
            reason("spec_unsupported", "0.13"),
            reason("unknown_feature", "files.home"),
            reason("capability_missing", "wonton_cloud"),
            reason("other", "WONTON_GONE_VAR"),
            reason("revoked_publisher", "app.good"),
        ],
    )]));

    let CoreUpgradeCompat::Breaking { plugins } = s.check("sha256:new").await else {
        panic!("breaking");
    };
    let categories: Vec<_> = plugins[0]
        .reasons
        .iter()
        .map(|reason| reason.category)
        .collect();
    use wonton_protocol::CoreUpgradeCompatCategory as Category;
    assert_eq!(
        categories,
        [
            Category::SpecUnsupported,
            Category::UnknownFeature,
            Category::CapabilityMissing,
            Category::Other,
            Category::Other,
        ],
        "{categories:?}"
    );
}

/// 材料化：compose 原文加引用的文件按原路径写出；越界路径拒绝。
#[test]
fn materialization_writes_the_package_and_refuses_escape_paths() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("compat").join("sha256-x");
    let mut files = PackageFiles::new();
    files.insert("settings/form.yaml".to_owned(), b"form".to_vec());
    let mut record = record("app.good", &package("app.good", "app"));
    record.files = files;
    let dirs = materialize(&root, &[("app.good".to_owned(), record)]).unwrap();
    assert_eq!(dirs, vec![root.join("app.good").display().to_string()]);
    assert_eq!(
        std::fs::read_to_string(root.join("app.good").join(COMPOSE_FILE)).unwrap(),
        package("app.good", "app")
    );
    assert_eq!(
        std::fs::read(root.join("app.good").join("settings").join("form.yaml")).unwrap(),
        b"form"
    );

    let mut escaping = record("app.bad", "services: []");
    escaping.files.insert("../escape".to_owned(), Vec::new());
    assert!(materialize(&root, &[("app.bad".to_owned(), escaping)]).is_err());
    assert!(!dir.path().join("escape").exists());
}

/// 一次 compat 的形态：限时 60 秒、只读挂载物化目录、命令是 `plugin compat <各包目录>`。
#[tokio::test]
async fn the_compat_job_is_limited_read_only_and_names_every_package() {
    let s = scene();
    s.install("app.one", "app", &package("app.one", "app"));
    s.install("app.two", "app", &package("app.two", "app"));
    *s.host.world().compat = Some(canned(&[ok("app.one"), ok("app.two")]));
    s.check("sha256:new").await;

    let job = s.host.world().compat_spec.clone().expect("跑过 compat");
    assert_eq!(job.name, "wonton-compat");
    assert_eq!(job.timeout, TIMEOUT);
    assert_eq!(job.timeout, Duration::from_secs(60));
    assert_eq!(job.cmd[..2], ["plugin".to_owned(), "compat".to_owned()]);
    assert_eq!(job.cmd.len(), 4, "{:?}", job.cmd);
    let Mount::Bind {
        target, read_only, ..
    } = &job.mounts[0]
    else {
        panic!("{:?}", job.mounts);
    };
    assert!(read_only);
    assert!(target.ends_with("compat/sha256-new"), "{target:?}");
    // 每个包一个目录，路径在挂载的物化目录下。
    for dir in &job.cmd[2..] {
        let dir = Path::new(dir);
        assert!(dir.starts_with(target), "{dir:?}");
    }
}
