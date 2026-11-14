//! `wonton-core plugin check` 和 `pack`：插件作者发布前看到的检查结果，开发时上传的插件包。

use std::fs;
use std::io::Cursor;
use std::path::Path;

use image::{ImageFormat, Rgba, RgbaImage};

use super::super::check::{Report, check};
use super::super::image_worker::ImageWorker;
use super::super::package::PluginPackage;
use super::package;
use super::views::{ICON, TAB_ICON};

const UNPINNED: &str = "  web:\n    image: nginx:1.27";

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    RgbaImage::from_pixel(width, height, Rgba([20, 120, 220, 255]))
        .write_to(&mut out, ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

fn tab_png() -> Vec<u8> {
    let mut image = RgbaImage::from_pixel(256, 256, Rgba([0, 0, 0, 0]));
    for x in 64..192 {
        for y in 64..192 {
            image.put_pixel(x, y, Rgba([0, 0, 0, 255]));
        }
    }
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

fn write(dir: &Path, path: &str, data: &[u8]) {
    let path = dir.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

fn compose(services: &str, extra: &str) -> String {
    package(
        services,
        &format!("  icon: icon.svg\n  screenshots: [shots/1.png]\n  permissions: []\n{extra}"),
    )
}

/// 单元测试里在本进程处理图片；起子进程的检查在 `tests/image_worker.rs`。
async fn run(dir: &Path, official: bool) -> Result<Report, String> {
    check(dir, official, &ImageWorker::in_process()).await
}

/// 插件仓库：插件包（compose 和图标），加上不属于插件包的 README 和截图。
fn repo(compose: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "docker-compose.yml", compose.as_bytes());
    write(dir.path(), "icon.svg", ICON.as_bytes());
    write(dir.path(), "shots/1.png", &png(1600, 1000));
    write(dir.path(), "README.md", b"# Test\n");
    write(dir.path(), "NOTICE", b"source and license\n");
    dir
}

#[tokio::test]
async fn lists_the_package_and_warns_about_what_the_store_would_flag() {
    let dir = repo(&compose(UNPINNED, ""));
    let report = run(dir.path(), false).await.unwrap();
    assert_eq!(
        (report.id.as_str(), report.version.as_str()),
        ("app.test", "1.0.0")
    );
    let files: Vec<&str> = report.files.iter().map(|(path, _)| path.as_str()).collect();
    assert_eq!(files, ["docker-compose.yml", "icon.svg"]);
    assert_eq!(
        report.digest,
        PluginPackage::load_dir(dir.path(), &super::langs())
            .unwrap()
            .digest()
    );
    assert!(report.problems.is_empty(), "{report:?}");
    let [unpinned, unhealthy] = &report.warnings[..] else {
        panic!("{report:?}")
    };
    assert!(
        unpinned.contains("nginx:1.27 isn't pinned by digest"),
        "{unpinned}"
    );
    assert!(unhealthy.contains("has no healthcheck"), "{unhealthy}");
}

#[tokio::test]
async fn the_official_index_turns_warnings_into_problems_and_wants_store_info() {
    let report = run(repo(&compose(UNPINNED, "")).path(), true)
        .await
        .unwrap();
    for expected in [
        "isn't pinned by digest",
        "has no healthcheck",
        "the official index requires tags",
        "the official index requires license",
        "the official index requires author",
        "the official index requires notices",
    ] {
        assert!(
            report.problems.iter().any(|p| p.contains(expected)),
            "{expected}：{report:?}"
        );
    }
    let pinned = format!(
        "  web:\n    image: nginx:1.27@sha256:{}\n    healthcheck:\n      test: [CMD, nginx, -t]",
        "a".repeat(64)
    );
    let complete = "  tags: {zh: [工具], en: [Tools]}\n  license: MIT\n  author: {name: Example}\n  notices: [NOTICE]";
    let report = run(repo(&compose(&pinned, complete)).path(), true)
        .await
        .unwrap();
    assert!(
        report.problems.is_empty() && report.warnings.is_empty(),
        "{report:?}"
    );
}

#[tokio::test]
async fn webui_without_healthcheck_requires_0_15_and_only_exempts_the_web_service() {
    let pinned = format!("  web:\n    image: nginx:1.27@sha256:{}", "a".repeat(64));
    let complete = "  tags: {zh: [工具], en: [Tools]}\n  license: MIT\n  author: {name: Example}\n  notices: [NOTICE]";
    let webui = "  webui: {service: web, port: 8080, path: /ui}";
    let yaml = compose(&pinned, &format!("{webui}\n{complete}"));
    for (spec, problems) in [("0.14", 1), ("0.15", 0)] {
        let source = yaml.replace("spec: '0.5'", &format!("spec: '{spec}'"));
        let report = run(repo(&source).path(), true).await.unwrap();
        assert_eq!(report.problems.len(), problems, "{report:?}");
        assert!(report.warnings.is_empty(), "{report:?}");
    }
    let sidecar = format!(
        "{pinned}\n  sidecar:\n    image: nginx:1.27@sha256:{}",
        "b".repeat(64)
    );
    let yaml =
        compose(&sidecar, &format!("{webui}\n{complete}")).replace("spec: '0.5'", "spec: '0.15'");
    for official in [false, true] {
        let report = run(repo(&yaml).path(), official).await.unwrap();
        assert_eq!(report.problems.len(), usize::from(official), "{report:?}");
        assert_eq!(report.warnings.len(), usize::from(!official), "{report:?}");
    }
}

#[tokio::test]
async fn explicit_webui_health_still_passes_under_spec_0_13() {
    let pinned = format!("  web:\n    image: nginx:1.27@sha256:{}", "a".repeat(64));
    let extra = "  webui: {service: web, port: 8080, health: /health}\n  tags: {zh: [工具], en: [Tools]}\n  license: MIT\n  author: {name: Example}\n  notices: [NOTICE]";
    let yaml = compose(&pinned, extra).replace("spec: '0.5'", "spec: '0.13'");
    let report = run(repo(&yaml).path(), true).await.unwrap();
    assert!(report.problems.is_empty(), "{report:?}");
    assert!(report.warnings.is_empty(), "{report:?}");
}

// `WONTON_BOT_AI_LEVEL` 是 0.15 才注入的变量：`spec` 写低时 `plugin check` 在发布前就报要升 spec，
// 不要等装到旧核心上才在插值时报“变量没有注入”。
#[tokio::test]
async fn check_names_the_spec_floor_for_the_bot_ai_level_variable() {
    let services = format!(
        "  web:\n    image: nginx:1.27@sha256:{}\n    environment:\n      LEVEL: ${{WONTON_BOT_AI_LEVEL}}",
        "a".repeat(64)
    );
    let report = run(repo(&compose(&services, "")).path(), false)
        .await
        .unwrap();
    assert!(
        report.problems.iter().any(|problem| {
            problem.contains("references WONTON_BOT_AI_LEVEL") && problem.contains("spec \"0.15\"")
        }),
        "{report:?}"
    );
}

#[tokio::test]
async fn the_official_index_refuses_hardcoded_proxy_addresses() {
    let services = |environment: &str| {
        format!(
            "  web:\n    image: nginx:1.27@sha256:{}\n    healthcheck:\n      test: [CMD, nginx, -t]\n    \
             environment:\n{environment}",
            "a".repeat(64)
        )
    };
    let complete = "  tags: {zh: [工具], en: [Tools]}\n  license: MIT\n  author: {name: Example}\n  notices: [NOTICE]";
    let hardcoded = services("      http_proxy: http://172.17.0.1:7899");
    let report = run(repo(&compose(&hardcoded, complete)).path(), true)
        .await
        .unwrap();
    assert!(
        report.problems.iter().any(|p| p.contains("http_proxy")),
        "{report:?}"
    );
    // 写成空是这个服务不走代理；引用 WONTON_PROXY_URL 是核心给的地址。两种都收。
    let allowed = services("      HTTP_PROXY: \"\"\n      APP_PROXY: ${WONTON_PROXY_URL}")
        .replace("permissions: []", "");
    let permitted =
        compose(&allowed, complete).replace("permissions: []", "permissions: [network.egress]");
    let report = run(repo(&permitted).path(), true).await.unwrap();
    assert!(report.problems.is_empty(), "{report:?}");
}

#[tokio::test]
async fn icons_and_screenshots_are_processed_like_the_store_does() {
    let shots = compose(UNPINNED, "").replace(
        "[shots/1.png]",
        "[shots/1.png, shots/gif.png, shots/tall.png]",
    );
    let dir = repo(&shots);
    write(dir.path(), "shots/gif.png", b"GIF89a");
    write(dir.path(), "shots/tall.png", &png(600, 900));
    let wide = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 24"/>"#;
    write(dir.path(), "icon.svg", wide.as_bytes());
    let report = run(dir.path(), false).await.unwrap();
    let [icon, gif] = &report.problems[..] else {
        panic!("{report:?}")
    };
    // check 的外层说明仍是核心写死的中文，内层原因走目录（默认英文）。
    let viewbox = crate::l10n::text(
        &super::langs(),
        "plugin-install-img-svg-viewbox",
        &[("width", "48".into()), ("height", "24".into())],
    );
    assert!(
        icon.starts_with("icon icon.svg: ") && icon.contains(&viewbox),
        "{icon}"
    );
    let gif_format = crate::l10n::text(
        &super::langs(),
        "plugin-install-img-is-format",
        &[("format", "Gif".into())],
    );
    assert!(
        gif.starts_with("screenshot shots/gif.png: ") && gif.contains(&gif_format),
        "{gif}"
    );
    for expected in ["shots/tall.png is 600×900", "landscape and portrait"] {
        assert!(
            report.warnings.iter().any(|w| w.contains(expected)),
            "{expected}：{report:?}"
        );
    }

    let round = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2 2"><circle cx="1" cy="1" r="1" fill="#000"/></svg>"##;
    write(dir.path(), "icon.svg", round.as_bytes());
    let report = run(dir.path(), false).await.unwrap();
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("transparent corners")),
        "{report:?}"
    );
}

#[tokio::test]
async fn plugin_check_processes_tab_png_and_svg_and_rejects_opaque_png() {
    let tabs = "  webui: {service: web, port: 8080}\n  tabs:\n    - id: feed\n      title: {zh: 动态, en: Feed}\n      icon: tabs/feed.png\n      icon_selected: tabs/feed.svg\n      path: /feed\n    - id: opaque\n      title: {zh: 实心, en: Opaque}\n      icon: tabs/opaque.png\n      path: /opaque";
    let dir = repo(&compose(UNPINNED, tabs));
    write(dir.path(), "tabs/feed.png", &tab_png());
    write(dir.path(), "tabs/feed.svg", TAB_ICON.as_bytes());
    write(dir.path(), "tabs/opaque.png", &png(256, 256));

    let report = run(dir.path(), false).await.unwrap();
    assert!(
        report.problems.iter().any(|problem| {
            problem.contains("tabs[1].icon")
                && problem.contains(&crate::l10n::text(
                    &super::langs(),
                    "plugin-install-img-tab-icon-alpha",
                    &[],
                ))
        }),
        "{report:?}"
    );
    assert!(
        !report
            .problems
            .iter()
            .any(|problem| problem.contains("tabs[0]")),
        "{report:?}"
    );
}

#[tokio::test]
async fn plugin_check_warns_about_long_tab_titles() {
    let tabs = "  webui: {service: web, port: 8080}\n  tabs:\n    - id: feed\n      title: {zh: 动态页面标签, en: FeedAndNews}\n      icon: tabs/feed.svg\n      path: /feed";
    let dir = repo(&compose(UNPINNED, tabs));
    write(dir.path(), "tabs/feed.svg", TAB_ICON.as_bytes());

    let report = run(dir.path(), false).await.unwrap();
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("tabs[0].title")),
        "{report:?}"
    );
}

#[tokio::test]
async fn a_package_that_does_not_parse_is_reported_like_at_install() {
    let dir = tempfile::tempdir().unwrap();
    let err = run(dir.path(), false).await.unwrap_err();
    let missing = crate::l10n::text(
        &super::langs(),
        "plugin-install-pkg-compose-file",
        &[(
            "error",
            crate::plugin::package_files::FileError::Missing
                .to_string()
                .into(),
        )],
    );
    assert!(err.contains(&missing), "{err}");
    write(
        dir.path(),
        "docker-compose.yml",
        compose(UNPINNED, "").as_bytes(),
    );
    let err = run(dir.path(), false).await.unwrap_err();
    assert!(err.contains("icon.svg"), "{err}");
}

#[test]
fn packing_keeps_only_the_package_and_installs_the_same_digest() {
    let dir = repo(&compose(UNPINNED, ""));
    let package = PluginPackage::load_dir(dir.path(), &super::langs()).unwrap();
    let archive = package.archive();
    let mut packed = tar::Archive::new(archive.as_slice());
    let paths: Vec<String> = packed
        .entries()
        .unwrap()
        .map(|entry| entry.unwrap().path().unwrap().display().to_string())
        .collect();
    assert_eq!(paths, ["docker-compose.yml", "icon.svg"]);
    let uploaded = PluginPackage::from_archive(&archive, &super::langs()).unwrap();
    assert_eq!(uploaded.digest(), package.digest());
}

#[tokio::test]
async fn apps_that_trust_more_than_the_core_are_flagged_by_how_they_can_be_reached() {
    const IDENTITY: &str =
        "  webui: {service: web, port: 80, sso: proxy, proxy: {headers: {Remote-User: '{user}'}}}";
    const VAR: &str = "WONTON_CORE_ADDRESS";
    let report = |permissions: &'static str, extra: &'static str, env: &'static str, official| {
        let compose = package(
            &format!("  web:\n    image: nginx:1.27\n    environment: {{TRUST: '{env}'}}"),
            &format!(
                "  icon: icon.svg\n  screenshots: [shots/1.png]\n  \
                 permissions: [{permissions}]\n{IDENTITY}\n{extra}"
            ),
        );
        async move {
            let dir = repo(&compose);
            run(dir.path(), official).await.unwrap()
        }
    };
    let flagged = |list: &[String]| list.iter().any(|line| line.contains(VAR));

    let egress = report("network.egress", "", "0.0.0.0/0", true).await;
    assert!(flagged(&egress.problems), "{:?}", egress.problems);
    let community = report("network.egress", "", "0.0.0.0/0", false).await;
    assert!(flagged(&community.warnings), "{:?}", community.warnings);
    assert!(!flagged(&community.problems));

    let lan = "  lan: {ports: [{service: web, port: 139}]}";
    let opened = report("network.lan", lan, "0.0.0.0/0", true).await;
    assert!(flagged(&opened.warnings), "{:?}", opened.warnings);
    assert!(!flagged(&opened.problems), "{:?}", opened.problems);

    let inside = report("", "", "0.0.0.0/0", true).await;
    assert!(flagged(&inside.warnings), "{:?}", inside.warnings);
    assert!(!flagged(&inside.problems));

    let good = report("network.egress", "", "${WONTON_CORE_ADDRESS}/32", true).await;
    assert!(!flagged(&good.warnings) && !flagged(&good.problems));
}

/// 插值出错时，检查结果里带字段路径（和安装时报的是同一条错误）。
#[tokio::test]
async fn interpolation_errors_name_the_field() {
    let dir = repo(&compose(
        "  web:\n    image: nginx\n    entrypoint: [sh, -c, \"echo $@\"]",
        "",
    ));
    let report = run(dir.path(), false).await.unwrap();
    assert!(
        report.problems.iter().any(|p| p.contains("entrypoint[2]")),
        "{report:?}"
    );
}

/// 显示方式的取值写错时和安装时报同一条错误。
#[tokio::test]
async fn an_unknown_chat_layout_is_reported() {
    let compose = compose("  bot:\n    image: echo", "  bot: {chat_layout: hologram}")
        .replace("permissions: []", "permissions: [matrix.bot]");
    let dir = repo(&compose);
    let err = run(dir.path(), false).await.unwrap_err();
    assert!(err.contains("bot.chat_layout"), "{err}");
}

#[tokio::test]
async fn environment_values_that_yaml_reads_as_booleans_are_called_out() {
    let bare_warnings = |report: &Report| -> Vec<String> {
        report
            .warnings
            .iter()
            .filter(|w| w.contains("is a bare"))
            .cloned()
            .collect()
    };
    // 映射写法里裸写的 on/off 会被 YAML 1.1 的工具当成布尔；加了引号和 true/false 的不管。
    let mapped = compose(
        "  web:\n    image: nginx:1.27\n    environment:\n      MOTD_ON: on  # 行尾注释\n\
         \x20     QUIET: off\n      LOUD: 'yes'\n      DEBUG: true\n",
        "",
    );
    let report = run(repo(&mapped).path(), false).await.unwrap();
    let called = bare_warnings(&report);
    assert_eq!(called.len(), 2, "{:?}", report.warnings);
    assert!(called[0].contains("environment.MOTD_ON"), "{}", called[0]);
    assert!(called[0].contains("`on`"), "{}", called[0]);
    assert!(called[1].contains("environment.QUIET"), "{}", called[1]);

    // 列表写法整条是字符串，不会被当成布尔，不提醒。
    let list = compose(
        "  web:\n    image: nginx:1.27\n    environment:\n      - SANDBOX=off\n",
        "",
    );
    let report = run(repo(&list).path(), false).await.unwrap();
    assert!(bare_warnings(&report).is_empty(), "{:?}", report.warnings);
}

#[tokio::test]
async fn depends_on_pointing_at_a_missing_service_is_called_out() {
    let services = "  web:\n    image: nginx:1.27\n    depends_on:\n      db:\n        condition: service_healthy\n\
                    \x20     gone: {condition: service_started}\n  db:\n    image: postgres:17\n";
    let report = run(repo(&compose(services, "")).path(), false)
        .await
        .unwrap();
    let called: Vec<&str> = report
        .warnings
        .iter()
        .filter(|w| w.contains("depends_on"))
        .map(String::as_str)
        .collect();
    let [missing] = called[..] else {
        panic!("{:?}", report.warnings)
    };
    assert!(missing.contains("services.web.depends_on"), "{missing}");
    assert!(missing.contains("`gone`"), "{missing}");

    // 短语法里指不存在的服务同样提醒；指到了的不提醒。
    let short = "  web:\n    image: nginx:1.27\n    depends_on: [db, cache]\n  db:\n    image: postgres:17\n";
    let report = run(repo(&compose(short, "")).path(), false).await.unwrap();
    let called: Vec<&str> = report
        .warnings
        .iter()
        .filter(|w| w.contains("depends_on"))
        .map(String::as_str)
        .collect();
    let [missing] = called[..] else {
        panic!("{:?}", report.warnings)
    };
    assert!(missing.contains("`cache`"), "{missing}");

    let ok = "  web:\n    image: nginx:1.27\n    depends_on: [db]\n  db:\n    image: postgres:17\n";
    let report = run(repo(&compose(ok, "")).path(), false).await.unwrap();
    assert!(
        !report.warnings.iter().any(|w| w.contains("depends_on")),
        "{:?}",
        report.warnings
    );
}

// check 认得出 Element Call 的页面（url 查询带 widgetId、baseUrl、intent）时，对照内置的所需能力
// 清单提醒漏声明的项；提醒不算不合格，退出码仍是 0。
fn pinned() -> String {
    format!(
        "  web:\n    image: nginx:1.27@sha256:{}\n    healthcheck:\n      test: [CMD, nginx, -t]",
        "a".repeat(64)
    )
}
const EC_URL: &str = "/room?widgetId=$matrix_widget_id&baseUrl=$org.matrix.msc4039.matrix_base_url&intent=join_existing";

// 声明 matrix_widget 的清单 spec 至少 0.14（spec_floor），夹具里的 '0.5' 要跟着抬。
fn widget_compose(services: &str, extra: &str) -> String {
    compose(services, extra).replace("spec: '0.5'", "spec: '0.14'")
}

fn element_call(capabilities: &[&str]) -> String {
    let mut yaml =
        format!("  webui: {{service: web, port: 8080}}\n  matrix_widget:\n    url: {EC_URL}\n");
    if capabilities.is_empty() {
        return yaml + "    capabilities: []\n";
    }
    yaml.push_str("    capabilities:\n");
    for capability in capabilities {
        yaml.push_str(&format!("      - {capability:?}\n"));
    }
    yaml
}

#[tokio::test]
async fn a_manifest_missing_an_element_call_capability_is_reminded_about_it() {
    use super::super::check_element_call::ELEMENT_CALL_CAPABILITIES;

    let missing = "org.matrix.msc2762.send.event:m.rtc.decline";
    let capabilities: Vec<&str> = ELEMENT_CALL_CAPABILITIES
        .iter()
        .copied()
        .filter(|capability| *capability != missing)
        .collect();
    let report = run(
        repo(&widget_compose(&pinned(), &element_call(&capabilities))).path(),
        false,
    )
    .await
    .unwrap();
    let expected = super::message(
        "plugin-check-element-call-capability-missing",
        &[("capability", missing)],
    );
    assert_eq!(report.warnings, [expected], "{report:?}");
    // 提醒不挡发布：不是不合格。
    assert!(report.problems.is_empty(), "{report:?}");
}

#[tokio::test]
async fn a_complete_element_call_manifest_gets_no_reminder() {
    use super::super::check_element_call::ELEMENT_CALL_CAPABILITIES;

    let report = run(
        repo(&widget_compose(
            &pinned(),
            &element_call(ELEMENT_CALL_CAPABILITIES),
        ))
        .path(),
        false,
    )
    .await
    .unwrap();
    assert!(
        report.warnings.is_empty() && report.problems.is_empty(),
        "{report:?}"
    );
}

#[tokio::test]
async fn a_widget_that_is_not_element_call_gets_no_reminder() {
    let report = run(
        repo(
            &widget_compose(&pinned(), &element_call(&[]))
                .replace(EC_URL, "/room?widgetId=$matrix_widget_id"),
        )
        .path(),
        false,
    )
    .await
    .unwrap();
    assert!(
        report.warnings.is_empty() && report.problems.is_empty(),
        "{report:?}"
    );
}

// 内置的 Element Call 能力清单是从 matrix-rust-sdk 抄的，SDK 升级时要跟着对：锁定版本号，对不上
// 就红，提醒去更新清单。
#[test]
fn the_element_call_list_tracks_the_pinned_sdk() {
    let lock =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock")).unwrap();
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line.trim_end() == "name = \"matrix-sdk\"" {
            let version = lines
                .next()
                .expect("the matrix-sdk version follows its name in Cargo.lock")
                .trim();
            assert_eq!(
                version, "version = \"0.19.1\"",
                "matrix-sdk 升级了：照新版 SDK 的 Element Call 所需能力（matrix-sdk-ffi 的 \
                 get_element_call_required_permissions，crates.io 发布的 crate 里没有这个函数）\
                 更新 check_element_call::ELEMENT_CALL_CAPABILITIES，再改这里的锁定版本"
            );
            return;
        }
    }
    panic!("Cargo.lock 里没有 matrix-sdk");
}
