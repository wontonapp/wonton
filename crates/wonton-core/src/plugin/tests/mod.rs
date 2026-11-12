//! 插件包与部署计划的契约测试：规范里写的规则在这里逐条验证。

use bollard::models::{MountType, RestartPolicyNameEnum};

use super::compose::DependCondition;
use super::hostdir::Owner;
use super::manifest::{Permission, Sso};
use super::package::{PackageError, PluginPackage};
use super::render::{RenderError, dry_run};

mod bot_avatars;
mod bot_lifecycle;
mod bots;
mod capability_deps_install;
mod capability_manifest;
mod chat_layout;
mod check;
mod configs;
mod configs_docker;
mod docker_timeouts;
mod egress;
mod egress_manifest;
mod egress_plugin;
mod egress_plugin_docker;
mod egress_probe;
mod egress_relay;
mod egress_switch;
mod github;
mod gpu;
mod health;
mod host_check;
mod host_facts;
mod host_name;
mod host_relay;
mod host_relay_docker;
mod install_l10n;
mod install_progress;
mod instances;
mod lan;
mod lan_announce;
mod lan_docker;
mod lan_ports;
mod lan_webui;
mod leftovers;
mod limits;
mod manager;
mod manifest_errors;
mod model_providers;
mod models;
mod models_route;
mod network_mode;
mod network_pool;
mod oidc;
mod optional_permissions;
mod owner;
mod people_languages;
mod preinstall;
mod privileges;
mod resources;
mod secrets;
mod settings_l10n;
mod sources;
mod stopped;
mod stopped_docker;
mod store;
mod store_refresh;
mod tabs;
mod upgrade;
mod views;
mod views_l10n;
mod watch;

/// 测试里的请求语言：默认（英文）。断言的期望值用 [`crate::l10n::CATALOG`] 生成。
fn langs() -> crate::plugin::languages::Languages {
    crate::plugin::languages::Languages::default()
}

/// 目录里的一条消息（默认语言）。
fn message(id: &str, args: &[(&str, &str)]) -> String {
    let lang = crate::l10n::lang(&langs());
    let args: Vec<(&str, wonton_l10n::Arg<'_>)> =
        args.iter().map(|(k, v)| (*k, (*v).into())).collect();
    crate::l10n::CATALOG.format(lang, id, &args)
}

fn example(name: &str) -> PluginPackage {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/examples")
        .join(name);
    PluginPackage::load_dir(&dir, &langs()).unwrap()
}

fn package(services: &str, extra_manifest: &str) -> String {
    format!(
        "services:\n{services}\nx-wonton:\n  spec: '0.5'\n  id: app.test\n  version: 1.0.0\n  kind: app\n  \
         title: {{zh: 测试, en: Test}}\n  description: {{zh: 测试, en: Test}}\n{extra_manifest}"
    )
}

fn env_of(plan: &super::render::ContainerPlan) -> Vec<String> {
    plan.body.env.clone().unwrap_or_default()
}

/// 只回答安装时用到的端点：发现文档、登记客户端。`sso: oidc` 的插件装上时要连身份提供方。
pub(super) async fn fake_hydra() -> crate::oidc::Hydra {
    // 和 Hydra v26.2.0 实测的一样：授权、登出端点给浏览器地址，签发方和后台端点给内部地址。
    let public = |path: &str| format!("http://auth.wonton.localhost:8080{path}");
    let internal = |path: &str| format!("http://auth.wonton.internal:8080{path}");
    let discovery = serde_json::json!({
        "issuer": internal(""),
        "authorization_endpoint": public("/oauth2/auth"),
        "token_endpoint": internal("/oauth2/token"),
        "userinfo_endpoint": internal("/userinfo"),
        "jwks_uri": internal("/.well-known/jwks.json"),
        "revocation_endpoint": internal("/oauth2/revoke"),
        "end_session_endpoint": public("/oauth2/sessions/logout"),
    });
    let app = axum::Router::new()
        .route(
            "/.well-known/openid-configuration",
            axum::routing::get(move || async move { axum::Json(discovery) }),
        )
        .route(
            "/admin/clients/{id}",
            axum::routing::put(|| async { hyper::StatusCode::OK }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let config = crate::config::OidcProviderConfig {
        admin_url: format!("http://{addr}"),
        public_url: format!("http://{addr}"),
    };
    crate::oidc::Hydra::new(crate::http::HttpClient::new(), &config)
}

/// 假 Docker 里容器创建时的环境变量：名字 → 值。
fn env_of_container(
    docker: &super::fake_docker::FakeDocker,
    container: &str,
) -> std::collections::BTreeMap<String, String> {
    docker
        .env(container)
        .iter()
        .filter_map(|e| e.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

fn render_err(yaml: &str) -> String {
    let pkg = PluginPackage::parse(yaml, &langs()).expect("包结构合法");
    match dry_run(&pkg, &langs()) {
        Ok(_) => panic!("渲染应失败"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn grafana_example_injects_oidc_and_app_url() {
    let pkg = example("grafana");
    let webui = pkg.manifest.webui.as_ref().unwrap();
    assert_eq!(pkg.manifest.id.as_str(), "app.grafana");
    assert_eq!(webui.sso, Sso::Oidc);
    let plan = dry_run(&pkg, &langs()).unwrap();
    assert_eq!(plan.network, "wonton-app-grafana");
    let grafana = &plan.containers[0];
    assert_eq!(grafana.name, "wonton-app-grafana--grafana");
    let env = env_of(grafana);
    for expected in [
        "GF_SERVER_ROOT_URL=http://app-grafana.wonton.invalid/",
        "GF_AUTH_GENERIC_OAUTH_CLIENT_ID=app.grafana",
        "GF_AUTH_GENERIC_OAUTH_CLIENT_SECRET=dry-run",
        "GF_AUTH_GENERIC_OAUTH_TOKEN_URL=http://auth.wonton.invalid/token",
    ] {
        assert!(
            env.contains(&expected.to_owned()),
            "缺少 {expected}：{env:?}"
        );
    }
    assert_eq!(grafana.body.user.as_deref(), Some("1000:1000"));
    let host = grafana.body.host_config.as_ref().unwrap();
    let mount = &host.mounts.as_ref().unwrap()[0];
    assert_eq!(mount.typ, Some(MountType::BIND));
    assert_eq!(
        mount.source.as_deref(),
        Some("/wonton-dry-run/apps/app-grafana/data")
    );
    assert_eq!(
        host.restart_policy.as_ref().unwrap().name,
        Some(RestartPolicyNameEnum::UNLESS_STOPPED)
    );
    assert!(host.port_bindings.is_none(), "插件不得向宿主暴露端口");
    // 应用数据目录要预先创建，交给 Grafana 运行的 ${PUID}:${PGID}。
    let data = std::path::Path::new("/wonton-dry-run/apps/app-grafana/data");
    assert_eq!(
        plan.dirs.get(data),
        Some(&Owner {
            uid: 1000,
            gid: 1000
        })
    );
}

#[test]
fn full_example_is_a_valid_plugin() {
    // 示例引用了包里的图标和设置表单。
    let files = ["icon.png", "settings.schema.json"].map(|f| (f.to_owned(), Vec::new()));
    let pkg = PluginPackage::with_files(include_str!("examples/full.yaml"), files.into(), &langs())
        .unwrap();
    assert_eq!(pkg.manifest.id.as_str(), "app.immich.photos");
    dry_run(&pkg, &langs()).unwrap();
}

#[test]
fn whoami_example_splits_command_like_compose() {
    let pkg = example("whoami");
    let plan = dry_run(&pkg, &langs()).unwrap();
    let cmd = plan.containers[0].body.cmd.clone().unwrap();
    assert_eq!(cmd, ["--port", "8000", "--name", "app.whoami"]);
    let networks = plan.containers[0].body.networking_config.clone().unwrap();
    let endpoint = &networks.endpoints_config.unwrap()["wonton-app-whoami"];
    assert_eq!(
        endpoint.aliases.as_deref(),
        Some(&["web", "web.app-whoami.wonton.internal"].map(String::from)[..])
    );
}

#[test]
fn host_paths_are_confined_to_app_data_and_granted_shares() {
    for volume in [
        "/etc:/host-etc",
        "/var/run/docker.sock:/var/run/docker.sock",
        "${WONTON_APP_DATA}/../escape:/x",
        "./data:/data",
        "undeclared:/data",
    ] {
        let yaml = package(
            &format!("  web:\n    image: nginx\n    volumes: ['{volume}']"),
            "  permissions: []",
        );
        assert!(!render_err(&yaml).is_empty(), "{volume} 应被拒绝");
    }
    let share = |volume: &str, access: &str| {
        package(
            &format!("  web:\n    image: nginx\n    volumes: ['{volume}']"),
            &format!("  permissions: ['storage.share.photos:{access}']"),
        )
    };
    let ro = PluginPackage::parse(
        &share("${WONTON_SHARE_PHOTOS}:/photos:ro", "read"),
        &langs(),
    )
    .unwrap();
    assert_eq!(
        ro.manifest.permissions,
        [Permission::StorageShare {
            name: "photos".into(),
            access: super::manifest::Access::Read
        }]
    );
    dry_run(&ro, &langs()).unwrap();
    assert!(render_err(&share("${WONTON_SHARE_PHOTOS}:/photos", "read")).contains(":ro"));
    dry_run(
        &PluginPackage::parse(&share("${WONTON_SHARE_PHOTOS}:/photos", "write"), &langs()).unwrap(),
        &langs(),
    )
    .unwrap();
    let undeclared = package(
        "  web:\n    image: nginx\n    volumes: ['${WONTON_SHARE_MUSIC}:/music:ro']",
        "  permissions: []",
    );
    assert!(render_err(&undeclared).contains("WONTON_SHARE_MUSIC"));
}

/// 两个服务，各自挂载给定的卷。
fn two_services(web: &str, worker: &str) -> PluginPackage {
    let yaml = package(
        &format!(
            "  web:\n    image: nginx\n    volumes: [{web}]\n  \
             worker:\n    image: nginx\n    volumes: [{worker}]"
        ),
        "  permissions: []",
    );
    PluginPackage::parse(&yaml, &langs()).unwrap()
}

#[test]
fn bind_mounts_of_one_plugin_must_not_nest() {
    // #20 的复现：能写外层目录的容器把内层的源换成指向 / 的软链接，重启后就挂上了宿主根目录。
    let nested = [
        (
            "'${WONTON_APP_DATA}:/a', '${WONTON_APP_DATA}/x:/x'",
            "",
            "web",
        ),
        (
            "'${WONTON_APP_DATA}/./:/a', '${WONTON_APP_DATA}//x/:/x'",
            "",
            "web",
        ),
        (
            "'${WONTON_APP_DATA}/data:/data'",
            "'${WONTON_APP_DATA}/data/x:/x:ro'",
            "worker",
        ),
    ];
    for (web, worker, inner) in nested {
        let err = dry_run(&two_services(web, worker), &langs()).unwrap_err();
        assert!(
            matches!(&err, RenderError::Service { service, .. } if service == inner),
            "{web} / {worker}: {err}"
        );
    }
    // 同一个目录挂进多个服务、并列的目录（包括名字是前缀的）都可以。
    let shared = two_services(
        "'${WONTON_APP_DATA}/data:/data', '${WONTON_APP_DATA}/data2:/data2'",
        "'${WONTON_APP_DATA}/data/:/data:ro'",
    );
    let plan = dry_run(&shared, &langs()).unwrap();
    let app_data = std::path::Path::new("/wonton-dry-run/apps/app-test");
    let dirs: Vec<_> = plan.dirs.keys().cloned().collect();
    assert_eq!(dirs, [app_data.join("data"), app_data.join("data2")]);
}

#[test]
fn data_dirs_belong_to_the_user_of_the_service_mounting_them() {
    let owner_of = |user: &str| {
        let yaml = package(
            &format!(
                "  web:\n    image: nginx\n{user}    volumes: ['${{WONTON_APP_DATA}}/data:/data']"
            ),
            "  permissions: []",
        );
        let plan = dry_run(&PluginPackage::parse(&yaml, &langs()).unwrap(), &langs())?;
        Ok::<_, RenderError>(plan.dirs.values().copied().collect::<Vec<_>>())
    };
    // 没写或是 root：以 root 启动、再按 PUID/PGID 降权的镜像，交给 PUID:PGID（演练里是 1000:1000）。
    let puid = Owner {
        uid: 1000,
        gid: 1000,
    };
    for user in ["", "    user: root\n", "    user: '0:0'\n"] {
        assert_eq!(owner_of(user).unwrap(), [puid], "{user}");
    }
    let magpie = Owner {
        uid: 65532,
        gid: 65532,
    };
    assert_eq!(owner_of("    user: '65532:65532'\n").unwrap(), [magpie]);
    // 用户名或只有 uid：核心看不出是谁。
    for user in ["    user: nobody\n", "    user: '1000'\n"] {
        let err = owner_of(user).unwrap_err();
        assert!(matches!(err, RenderError::Service { .. }), "{user}: {err}");
    }
    // 同一个目录挂进两个服务，属主必须一样。
    let same = two_services(
        "'${WONTON_APP_DATA}/data:/data'",
        "'${WONTON_APP_DATA}/data:/data:ro'",
    );
    dry_run(&same, &langs()).unwrap();
    let yaml = package(
        "  web:\n    image: nginx\n    volumes: ['${WONTON_APP_DATA}/data:/data']\n  \
         worker:\n    image: nginx\n    user: '65532:65532'\n    volumes: ['${WONTON_APP_DATA}/data:/data']",
        "  permissions: []",
    );
    let err = dry_run(&PluginPackage::parse(&yaml, &langs()).unwrap(), &langs()).unwrap_err();
    assert!(matches!(err, RenderError::Service { .. }), "{err}");
}

#[test]
fn shares_are_mounted_only_at_their_root() {
    // 共享文件夹由多个插件共用，有写权限的插件能把其中的子目录换成软链接。
    let share = |volume: &str| {
        let yaml = package(
            &format!("  web:\n    image: nginx\n    volumes: ['{volume}']"),
            "  permissions: ['storage.share.photos:write']",
        );
        dry_run(&PluginPackage::parse(&yaml, &langs()).unwrap(), &langs())
    };
    for volume in [
        "${WONTON_SHARE_PHOTOS}/2024:/photos",
        "${WONTON_SHARE_PHOTOS}/a/b:/photos:ro",
    ] {
        let err = share(volume).unwrap_err();
        assert!(
            matches!(err, RenderError::Service { .. }),
            "{volume}: {err}"
        );
    }
    let plan = share("${WONTON_SHARE_PHOTOS}/:/photos").unwrap();
    let host = plan.containers[0].body.host_config.as_ref().unwrap();
    let mount = &host.mounts.as_ref().unwrap()[0];
    assert_eq!(
        mount.source.as_deref(),
        Some("/wonton-dry-run/shares/photos")
    );
}

#[test]
fn container_and_volume_names_differ_across_plugins() {
    // 只用一个 `-` 连接时，app.foo 的 bar-web、bar-data 和 app.foo.bar 的 web、data 重名。
    let plan = |id: &str, service: &str, volume: &str| {
        let services = format!("  {service}:\n    image: nginx\n    volumes: ['{volume}:/data']");
        let yaml = format!(
            "volumes:\n  {volume}: {{}}\n{}",
            package(&services, "  permissions: []")
        )
        .replace("id: app.test", &format!("id: {id}"));
        dry_run(&PluginPackage::parse(&yaml, &langs()).unwrap(), &langs()).unwrap()
    };
    let foo = plan("app.foo", "bar-web", "bar-data");
    let foo_bar = plan("app.foo.bar", "web", "data");
    assert_ne!(foo.containers[0].name, foo_bar.containers[0].name);
    assert_ne!(foo.volumes, foo_bar.volumes);
}

#[test]
fn oidc_variables_exist_only_for_sso_plugins() {
    let yaml = package(
        "  web:\n    image: nginx\n    environment:\n      SECRET: ${WONTON_OIDC_CLIENT_SECRET}",
        "  permissions: []\n  webui: {service: web, port: 80}",
    );
    assert!(render_err(&yaml).contains("WONTON_OIDC_CLIENT_SECRET"));
    let no_value = package(
        "  web:\n    image: nginx\n    environment: [HOST_SECRET]",
        "  permissions: []",
    );
    assert!(render_err(&no_value).contains("HOST_SECRET"));
}

#[test]
fn services_start_in_dependency_order_and_cycles_fail() {
    let yaml = package(
        "  app:\n    image: a\n    depends_on: {db: {condition: service_healthy}}\n  \
         db:\n    image: postgres\n    volumes: ['pg:/var/lib/postgresql/data']",
        "  permissions: []",
    );
    let yaml = format!("volumes:\n  pg: {{}}\n{yaml}");
    let plan = dry_run(&PluginPackage::parse(&yaml, &langs()).unwrap(), &langs()).unwrap();
    let names: Vec<&str> = plan.containers.iter().map(|c| c.service.as_str()).collect();
    assert_eq!(names, ["db", "app"]);
    assert_eq!(
        plan.containers[1].depends_on,
        [(
            "wonton-app-test--db".to_owned(),
            DependCondition::ServiceHealthy
        )]
    );
    assert_eq!(plan.volumes, ["wonton-app-test--pg"]);

    let cycle = package(
        "  a:\n    image: a\n    depends_on: [b]\n  b:\n    image: b\n    depends_on: [a]",
        "  permissions: []",
    );
    assert!(render_err(&cycle).contains(&message("plugin-install-render-dependency-cycle", &[])));
}

#[test]
fn reserved_labels_and_yaml_merge_keys() {
    let reserved = package(
        "  web:\n    image: nginx\n    labels: {wonton.plugin: other}",
        "  permissions: []",
    );
    assert!(render_err(&reserved).contains("wonton."));
    let merged = format!(
        "x-env: &env {{A: '1'}}\n{}",
        package(
            "  web:\n    image: nginx\n    environment:\n      <<: *env\n      B: '2'",
            "  permissions: []"
        )
    );
    let plan = dry_run(&PluginPackage::parse(&merged, &langs()).unwrap(), &langs()).unwrap();
    assert_eq!(env_of(&plan.containers[0]), ["A=1", "B=2"]);
}

#[test]
fn manifest_rules_are_enforced() {
    let cases = [
        ("  permissions: [root.access]", "权限"),
        (
            "  permissions: []\n  webui: {service: web, port: 80, sso: oidc}",
            "redirect_paths",
        ),
        (
            "  permissions: []\n  webui: {service: nope, port: 80}",
            "nope",
        ),
    ];
    for (manifest, needle) in cases {
        let err = PluginPackage::parse(&package("  web:\n    image: nginx", manifest), &langs())
            .unwrap_err();
        assert!(err.to_string().contains(needle), "{manifest}: {err}");
    }
    let with_title = |title: &str| {
        format!(
            "services:\n  web:\n    image: nginx\nx-wonton:\n  spec: '0.4'\n  id: app.test\n  version: 1.0.0\n  \
             kind: app\n  title: {title}\n  description: {{zh: 测, en: T}}\n  permissions: []"
        )
    };
    assert!(PluginPackage::parse(&with_title("{en: Test}"), &langs()).is_ok());
    assert!(PluginPackage::parse(&with_title("{}"), &langs()).is_err());
}

/// `mcp.headers`：引用不存在的密钥、禁止的头名、身份占位符都报错；generate 和 collect 的密钥都能引用。
#[test]
fn mcp_header_rules_are_enforced() {
    let mcp = |header: &str| {
        package(
            "  mcp:\n    image: nginx",
            &format!(
                "  permissions: []\n  secrets:\n    pat:\n      generate: {{bytes: 32}}\n    \
                 app_token:\n      collect: {{service: mcp, path: /run/token}}\n  mcp:\n    service: mcp\n    \
                 port: 8080\n    transport: streamable-http\n    headers:\n      {header}"
            ),
        )
    };
    let err = |header: &str| {
        PluginPackage::parse(&mcp(header), &langs())
            .unwrap_err()
            .to_string()
    };
    assert!(
        err("Authorization: Bearer {secret:missing}").contains("missing"),
        "{}",
        err("Authorization: Bearer {secret:missing}")
    );
    let forbidden = |name: &str| {
        message(
            "plugin-install-proxy-mcp-prefix",
            &[(
                "error",
                &message(
                    "plugin-install-proxy-mcp-header-forbidden",
                    &[("name", name)],
                ),
            )],
        )
    };
    assert_eq!(err("Host: '{secret:pat}'"), forbidden("Host"));
    assert_eq!(
        err("Mcp-Session-Id: '{secret:pat}'"),
        forbidden("Mcp-Session-Id")
    );
    assert_eq!(
        err("X-Wonton-User: '{user}'"),
        message(
            "plugin-install-proxy-mcp-prefix",
            &[(
                "error",
                &message(
                    "plugin-install-proxy-mcp-identity-placeholder",
                    &[("placeholder", "user"), ("template", "{user}")]
                )
            )]
        )
    );
    assert!(
        PluginPackage::parse(&mcp("Authorization: Bearer {secret:app_token}"), &langs()).is_ok()
    );
    assert!(PluginPackage::parse(&mcp("X-Api-Key: '{secret:pat}'"), &langs()).is_ok());
}
