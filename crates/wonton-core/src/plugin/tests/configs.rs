//! 插件包目录、被引用的文件和 compose `configs`。

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use bollard::models::MountType;

use super::super::configs::ConfigFile;
use super::super::package::{PackageError, PluginPackage};
use super::super::package_files::{FileError, MAX_FILE_BYTES, PackageFiles};
use super::super::render::{RenderError, dry_run};
use super::package;

/// 一个服务引用包里的 `conf/app.yaml`，清单引用图标。
fn bundle() -> String {
    format!(
        "configs:\n  app:\n    file: ./conf/app.yaml\n{}",
        package(
            "  web:\n    image: nginx\n    configs: [app]",
            "  icon: icon.png\n  permissions: []",
        )
    )
}

fn write_dir(dir: &Path, compose: &str) {
    fs::create_dir_all(dir.join("conf")).unwrap();
    fs::write(dir.join("docker-compose.yml"), compose).unwrap();
    fs::write(dir.join("conf/app.yaml"), "level: info\n").unwrap();
    fs::write(dir.join("icon.png"), [0x89, b'P', b'N', b'G']).unwrap();
    fs::write(dir.join("notes.txt"), "没有被引用").unwrap();
}

#[test]
fn a_package_directory_keeps_only_the_files_it_references() {
    let tmp = tempfile::tempdir().unwrap();
    write_dir(tmp.path(), &bundle());
    let pkg = PluginPackage::load_dir(tmp.path(), &super::langs()).unwrap();
    let paths: Vec<&str> = pkg.files().keys().map(String::as_str).collect();
    assert_eq!(paths, ["conf/app.yaml", "icon.png"]);
    // 安装记录里保存的就是这些文件，恢复时用它们重新组装插件包。
    let again =
        PluginPackage::with_files(&pkg.source, pkg.files().clone(), &super::langs()).unwrap();
    assert_eq!(again.files(), pkg.files());

    // 只有 compose 文本（如经 API 上传的）时，被引用的文件不在包里。
    let only_icon: PackageFiles = [("icon.png".to_owned(), Vec::new())].into();
    let err = PluginPackage::with_files(&bundle(), only_icon, &super::langs()).unwrap_err();
    assert!(
        matches!(&err, PackageError::File { path, source, .. } if path == "conf/app.yaml" && **source == FileError::Missing),
        "{err}"
    );
}

#[test]
fn referenced_files_cannot_come_through_symlinks_or_leave_the_package() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    write_dir(&dir, &bundle());
    fs::write(tmp.path().join("host-secret"), "宿主上的文件").unwrap();
    fs::remove_file(dir.join("conf/app.yaml")).unwrap();
    symlink(tmp.path().join("host-secret"), dir.join("conf/app.yaml")).unwrap();
    let err = PluginPackage::load_dir(&dir, &super::langs()).unwrap_err();
    assert!(
        matches!(
            &err,
            PackageError::File { source, .. }
                if matches!(&**source, FileError::Symlink(_))
        ),
        "{err}"
    );

    for path in ["../host-secret", "/etc/passwd"] {
        let compose = bundle().replace("./conf/app.yaml", path);
        assert!(
            matches!(
                PluginPackage::parse(&compose, &super::langs()),
                Err(PackageError::Config { .. })
            ),
            "{path}"
        );
    }
    let icon = bundle().replace("icon: icon.png", "icon: http://example.com/a.png");
    assert!(matches!(
        PluginPackage::parse(&icon, &super::langs()),
        Err(PackageError::Reference { .. })
    ));
    let too_big: PackageFiles = [
        (
            "conf/app.yaml".to_owned(),
            vec![0; MAX_FILE_BYTES as usize + 1],
        ),
        ("icon.png".to_owned(), Vec::new()),
    ]
    .into();
    assert!(matches!(
        PluginPackage::with_files(&bundle(), too_big, &super::langs()),
        Err(PackageError::File { source, .. })
            if matches!(&*source, FileError::TooBig)
    ));
}

#[test]
fn configs_become_files_with_owner_and_mode() {
    let langs = super::langs();
    let compose = format!(
        "configs:\n  app:\n    file: app.yaml\n  motd:\n    content: \"id=${{WONTON_APP_ID}}\\n\"\n{}",
        package(
            "  web:\n    image: nginx\n    user: '1000:1001'\n    configs:\n      - motd\n      \
             - source: app\n        target: /etc/app/app.yaml\n        uid: '0'\n        mode: 0440",
            "  permissions: []",
        )
    );
    let files: PackageFiles = [("app.yaml".to_owned(), b"level: info\n".to_vec())].into();
    let plan = dry_run(
        &PluginPackage::with_files(&compose, files, &super::langs()).unwrap(),
        &langs,
    )
    .unwrap();
    let file = |target: &str, uid, gid, mode, data: &[u8]| ConfigFile {
        target: PathBuf::from(target),
        uid,
        gid,
        mode,
        data: data.to_vec(),
    };
    assert_eq!(
        plan.containers[0].files,
        [
            // 短语法放到 /<名字>；默认属主是服务的 user，默认所有人可读；content 按注入的变量插值。
            file("/motd", 1000, 1001, 0o444, b"id=app.test\n"),
            file("/etc/app/app.yaml", 0, 1001, 0o440, b"level: info\n"),
        ]
    );
}

/// 一个服务，挂载给定的卷并引用 `app` 配置；`app` 是内联内容。
fn with_config(service_extra: &str, config_ref: &str) -> String {
    format!(
        "configs:\n  app:\n    content: x\n{}",
        package(
            &format!("  web:\n    image: nginx\n{service_extra}    configs: [{config_ref}]"),
            "  permissions: []",
        )
    )
}

#[test]
fn config_definitions_outside_the_subset_are_refused() {
    for definition in [
        "{external: true}",
        "{environment: HOME}",
        "{file: a, name: b}",
        "{file: a, content: b}",
        "{}",
        "{content: 1}",
    ] {
        let compose = format!(
            "configs:\n  app: {definition}\n{}",
            package("  web:\n    image: nginx", "  permissions: []")
        );
        assert!(
            matches!(
                PluginPackage::parse(&compose, &super::langs()),
                Err(PackageError::Config { .. })
            ),
            "{definition}"
        );
    }
}

#[test]
fn configs_are_written_only_into_the_containers_own_filesystem() {
    let langs = super::langs();
    let cases = [
        ("", "nope"),
        ("", "{source: app, target: etc/app}"),
        ("", "{source: app, target: /etc/../x}"),
        ("", "app, {source: app, target: /app}"),
        ("", "{source: app, mode: 04755}"),
        // 写进挂载点会在应用数据目录、卷或共享文件夹里留下文件；tmpfs 启动时会把它盖住。
        (
            "    volumes: ['${WONTON_APP_DATA}/data:/data']\n",
            "{source: app, target: /data/app}",
        ),
        (
            "    tmpfs: /run:size=1m\n",
            "{source: app, target: /run/app}",
        ),
        ("    volumes: ['/cache']\n", "{source: app, target: /cache}"),
    ];
    for (service_extra, config_ref) in cases {
        let pkg =
            PluginPackage::parse(&with_config(service_extra, config_ref), &super::langs()).unwrap();
        let err = dry_run(&pkg, &langs).unwrap_err();
        assert!(
            matches!(&err, RenderError::Service { service, .. } if service == "web"),
            "{service_extra}{config_ref}: {err}"
        );
    }
    let fine = with_config(
        "    volumes: ['${WONTON_APP_DATA}/data:/data']\n",
        "{source: app, target: /database}",
    );
    dry_run(
        &PluginPackage::parse(&fine, &super::langs()).unwrap(),
        &langs,
    )
    .unwrap();
}

/// `read_only` 的服务：配置文件改为只读单文件绑定挂载，源是核心数据目录里的 store 文件（编号）。
#[test]
fn read_only_services_get_configs_from_the_core_store() {
    let langs = super::langs();
    let compose = with_config(
        "    read_only: true\n",
        "{source: app, target: /etc/app.yaml}",
    );
    let plan = dry_run(
        &PluginPackage::parse(&compose, &super::langs()).unwrap(),
        &langs,
    )
    .unwrap();
    let container = &plan.containers[0];
    assert_eq!(
        container.store_dir.as_deref(),
        Some(Path::new("/wonton-dry-run/core/configs/app-test/web"))
    );
    let host = container.body.host_config.as_ref().unwrap();
    let mount = &host.mounts.as_ref().unwrap()[0];
    assert_eq!(mount.typ, Some(MountType::BIND));
    assert_eq!(
        mount.source.as_deref(),
        Some("/wonton-dry-run/core/configs/app-test/web/0")
    );
    assert_eq!(mount.target.as_deref(), Some("/etc/app.yaml"));
    assert_eq!(mount.read_only, Some(true));
    // 内容在 files 里，创建容器前由一次性容器经归档接口写进 store 目录。
    assert_eq!(container.files.len(), 1);
    assert_eq!(container.files[0].target, Path::new("/etc/app.yaml"));
}

/// 没有 `read_only` 的服务维持原来的交付：启动前把归档写进容器根，不走 store。
#[test]
fn writable_services_keep_the_archive_delivery() {
    let langs = super::langs();
    let compose = with_config("", "app");
    let plan = dry_run(
        &PluginPackage::parse(&compose, &super::langs()).unwrap(),
        &langs,
    )
    .unwrap();
    let container = &plan.containers[0];
    assert_eq!(container.store_dir, None);
    let host = container.body.host_config.as_ref().unwrap();
    assert!(host.mounts.as_ref().is_none_or(|m| m.is_empty()));
    assert_eq!(container.files.len(), 1);
}

#[test]
fn services_without_network_join_no_network() {
    let langs = super::langs();
    let compose = package(
        "  init:\n    image: busybox\n    network_mode: none\n    restart: 'no'\n  \
         web:\n    image: nginx\n    depends_on: {init: {condition: service_completed_successfully}}",
        "  permissions: []\n  webui: {service: web, port: 80}",
    );
    let plan = dry_run(
        &PluginPackage::parse(&compose, &super::langs()).unwrap(),
        &langs,
    )
    .unwrap();
    let init = &plan.containers[0];
    assert_eq!(init.service, "init");
    let host = init.body.host_config.as_ref().unwrap();
    assert_eq!(host.network_mode.as_deref(), Some("none"));
    assert!(init.body.networking_config.is_none(), "不接插件网络");
    let web = &plan.containers[1];
    assert_eq!(
        web.body
            .host_config
            .as_ref()
            .unwrap()
            .network_mode
            .as_deref(),
        Some(plan.network.as_str())
    );

    for mode in ["bridge", "container:x"] {
        let other = compose.replace("network_mode: none", &format!("network_mode: '{mode}'"));
        assert!(
            matches!(
                PluginPackage::parse(&other, &super::langs()),
                Err(PackageError::Service { .. })
            ),
            "{mode}"
        );
    }
    let isolated_webui = compose.replace("{service: web, port: 80}", "{service: init, port: 80}");
    let err = dry_run(
        &PluginPackage::parse(&isolated_webui, &super::langs()).unwrap(),
        &langs,
    )
    .unwrap_err();
    assert!(
        matches!(&err, RenderError::Service { service, .. } if service == "init"),
        "{err}"
    );
}
