//! Bot 服务的变量和规则，以及核心还兑现不了的权限的变量。安装、卸载时 Bot 在 Matrix 上的副作用见
//! [`super::bot_lifecycle`]。

use super::super::docker::Engine;
use super::super::package::{PackageError, PluginPackage};
use super::super::render::{
    ContainerPlan, Deployment, InstallContext, MatrixVars, OidcVars, RenderError, dry_run,
};
use super::manager::manager_with;
use super::{env_of, package};

/// `init` 和 `gateway` 每个 Bot 一份，`web` 全插件一份。
const SERVICES: &str = "  init:\n    image: busybox\n    restart: 'no'\n    \
    volumes: ['${WONTON_BOT_DATA}:/data']\n  \
    gateway:\n    image: hermes\n    environment:\n      BOT: ${WONTON_BOT_NAME}\n    \
    volumes: ['${WONTON_BOT_DATA}:/data']\n    depends_on:\n      \
    init: {condition: service_completed_successfully}\n      web: {condition: service_started}\n  \
    web:\n    image: nginx";

const BOTS: &str = "  bot: {display_name: {zh: 测试, en: Test}}\n  bots: {services: [init, gateway]}\n  \
    permissions: [matrix.bot]";

fn plan(services: &str, manifest: &str) -> Result<Deployment, RenderError> {
    dry_run(&PluginPackage::parse(&package(services, manifest)).unwrap())
}

fn container<'a>(plan: &'a Deployment, service: &str) -> &'a ContainerPlan {
    plan.containers
        .iter()
        .find(|c| c.service == service)
        .unwrap()
}

fn aliases(plan: &Deployment, service: &str) -> Vec<String> {
    let networking = container(plan, service).body.networking_config.clone();
    let endpoint = &networking.unwrap().endpoints_config.unwrap()[&plan.network];
    endpoint.aliases.clone().unwrap()
}

#[test]
fn bot_services_get_the_bot_name_and_their_own_data_dir() {
    let plan = plan(SERVICES, BOTS).unwrap();
    let gateway = container(&plan, "gateway");
    assert!(env_of(gateway).contains(&"BOT=default".to_owned()));
    let mounts = gateway.body.host_config.clone().unwrap().mounts.unwrap();
    let bot_data = "/wonton-dry-run/apps/app-test/bots/default";
    assert_eq!(mounts[0].source.as_deref(), Some(bot_data));
    let dirs: Vec<_> = plan.dirs.keys().cloned().collect();
    assert_eq!(dirs, [std::path::PathBuf::from(bot_data)]);
    // 同一个 Bot 的服务之间用 `<服务名>-<Bot 名>` 互访。
    assert_eq!(aliases(&plan, "gateway"), ["gateway", "gateway-default"]);
    assert_eq!(aliases(&plan, "web"), ["web"]);

    // 全插件一份的服务没有 Bot 名，也不能依赖每个 Bot 一份的服务。
    let web_uses_bot = SERVICES.replace(
        "image: nginx",
        "image: nginx\n    command: ${WONTON_BOT_NAME}",
    );
    assert!(matches!(
        plan_err(&web_uses_bot, BOTS),
        RenderError::Package(PackageError::Interpolate { .. })
    ));
    let web_needs_bot = SERVICES.replace("image: nginx", "image: nginx\n    depends_on: [init]");
    assert!(
        matches!(plan_err(&web_needs_bot, BOTS), RenderError::Service { ref service, .. } if service == "web")
    );
}

fn plan_err(services: &str, manifest: &str) -> RenderError {
    plan(services, manifest).unwrap_err()
}

#[test]
fn bot_declarations_need_matrix_bot_and_real_services() {
    let parse = |manifest: &str| PluginPackage::parse(&package(SERVICES, manifest));
    for manifest in [
        BOTS.replace("[matrix.bot]", "[]"),
        "  bot: {display_name: {zh: 测试, en: Test}}\n  permissions: []".to_owned(),
        BOTS.replace("[init, gateway]", "[]"),
        BOTS.replace("[init, gateway]", "[init, init]"),
    ] {
        assert!(
            matches!(parse(&manifest), Err(PackageError::Manifest(_))),
            "{manifest}"
        );
    }
    let agent = package(SERVICES, "  agent: {protocol: matrix}\n  permissions: []")
        .replace("kind: app", "kind: agent");
    assert!(matches!(
        PluginPackage::parse(&agent),
        Err(PackageError::Manifest(_))
    ));
    assert!(matches!(
        parse(&BOTS.replace("[init, gateway]", "[init, nope]")),
        Err(PackageError::UnknownService { .. })
    ));
    // 网页入口全插件一份，不能指向 Bot 服务。
    let webui = format!("{BOTS}\n  webui: {{service: gateway, port: 80}}");
    assert!(matches!(parse(&webui), Err(PackageError::Service { .. })));
}

#[test]
fn the_bot_identity_goes_only_to_bot_services_when_bots_are_declared() {
    let gateway_env = SERVICES.replace(
        "BOT: ${WONTON_BOT_NAME}",
        "BOT: ${WONTON_BOT_NAME}\n      TOKEN: ${WONTON_MATRIX_ACCESS_TOKEN}",
    );
    let bots = plan(&gateway_env, BOTS).unwrap();
    assert!(env_of(container(&bots, "gateway")).contains(&"TOKEN=dry-run".to_owned()));
    // 全插件一份的服务拿不到哪个 Bot 的令牌：每个 Bot 一个。
    let web_env = SERVICES.replace(
        "image: nginx",
        "image: nginx\n    environment:\n      USER: ${WONTON_MATRIX_USER_ID}",
    );
    assert!(matches!(
        plan_err(&web_env, BOTS),
        RenderError::Package(PackageError::Interpolate { .. })
    ));
    // 没有 `bots` 时插件只有一个 Bot，所有服务都拿得到。
    let web = "  web:\n    image: nginx\n    environment:\n      USER: ${WONTON_MATRIX_USER_ID}";
    let single = "  bot: {display_name: {zh: 测试, en: Test}}\n  permissions: [matrix.bot]";
    let one = plan(web, single).unwrap();
    let user = "USER=@dry-run:wonton.invalid".to_owned();
    assert!(env_of(container(&one, "web")).contains(&user));
}

#[tokio::test]
async fn variables_of_permissions_the_core_cannot_honor_yet_are_reported_as_unsupported() {
    let models = package(
        "  web:\n    image: nginx\n    environment:\n      BASE: ${WONTON_MODEL_BASE_URL}",
        "  permissions: [models]",
    );
    let err = dry_run(&PluginPackage::parse(&models).unwrap()).unwrap_err();
    assert!(matches!(err, RenderError::Unsupported(_)), "{err}");
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(Engine::unreachable_for_test(), &dir);
    let granted = vec!["models".parse().unwrap()];
    let err = m.install(&models, &granted).await.unwrap_err();
    assert_eq!(err.kind(), "unsupported", "{err}");

    // 没声明权限却引用它的变量：是插件写错了，照常报变量未定义。
    let undeclared = models.replace("[models]", "[]");
    let err = dry_run(&PluginPackage::parse(&undeclared).unwrap()).unwrap_err();
    assert!(matches!(err, RenderError::Package(_)), "{err}");

    // 内联配置里引用模型网关的变量也一样。
    let content = format!(
        "configs:\n  key:\n    content: ${{WONTON_MODEL_API_KEY}}\n{}",
        package(
            &SERVICES.replace("image: hermes", "image: hermes\n    configs: [key]"),
            &BOTS.replace("[matrix.bot]", "[matrix.bot, models]")
        )
    );
    let err = dry_run(&PluginPackage::parse(&content).unwrap()).unwrap_err();
    assert!(matches!(err, RenderError::Unsupported(_)), "{err}");
}

#[test]
fn install_contexts_print_no_tokens_or_secrets() {
    let ctx = InstallContext {
        slug: "app-test".into(),
        app_data: "/apps/app-test".into(),
        shares: Default::default(),
        app_url: None,
        oidc: Some(OidcVars {
            issuer: "http://auth".into(),
            client_id: "app.test".into(),
            client_secret: "oidc-secret-value".into(),
            auth_url: String::new(),
            token_url: String::new(),
            userinfo_url: String::new(),
        }),
        matrix: Some(MatrixVars {
            homeserver: "http://matrix.wonton.internal".into(),
            user_id: "@test:t".into(),
            access_token: "matrix-token-value".into(),
            owner: "@owner:t".into(),
            home_room: "!room:t".into(),
        }),
        secrets: [("web_key".to_owned(), "generated-secret-value".to_owned())].into(),
        tz: "UTC".into(),
        puid: 1000,
        pgid: 1000,
        extra_hosts: Vec::new(),
    };
    let printed = format!("{ctx:?}");
    for secret in [
        "oidc-secret-value",
        "matrix-token-value",
        "generated-secret-value",
    ] {
        assert!(!printed.contains(secret), "{printed}");
    }
    assert!(printed.contains("web_key"), "只打印密钥的名字：{printed}");
}
