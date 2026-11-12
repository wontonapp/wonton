//! Bot 服务的变量和规则，以及模型网关的变量。安装、卸载时 Bot 在 Matrix 上的副作用见
//! [`super::bot_lifecycle`]，模型网关的凭据见 [`super::models`]。

use super::super::PluginId;
use super::super::bot_services::DEFAULT_BOT;
use super::super::bots::{BotEntry, BotStore, PluginBots};
use super::super::package::{PackageError, PluginPackage};
use super::super::render::{ContainerPlan, Deployment, InstallContext, RenderError, dry_run};
use super::super::vars::{MatrixVars, ModelVars, OidcVars};
use super::{env_of, package};
use crate::matrix::BotAccount;
use wonton_protocol::BotKeeper;

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
    dry_run(
        &PluginPackage::parse(&package(services, manifest), &super::langs()).unwrap(),
        &super::langs(),
    )
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
    // 同一个 Bot 的服务之间用 `<服务名>-<Bot 名>` 互访；每个服务再有一个 `wonton.internal` 的稳定域名。
    assert_eq!(
        aliases(&plan, "gateway"),
        [
            "gateway",
            "gateway-default",
            "gateway.app-test.wonton.internal"
        ]
    );
    assert_eq!(
        aliases(&plan, "web"),
        ["web", "web.app-test.wonton.internal"]
    );

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

/// 服务拿到 `WONTON_SERVICE_DOMAIN`，compose 里写 `mcp.${WONTON_SERVICE_DOMAIN}` 得到上游白名单要的 Host。
#[test]
fn services_get_the_service_domain() {
    let plan = plan(
        "  web:\n    image: nginx\n    command: --allowed-hosts mcp.${WONTON_SERVICE_DOMAIN}",
        "  permissions: []",
    )
    .unwrap();
    let web = container(&plan, "web");
    assert_eq!(
        web.body.cmd.clone().unwrap(),
        ["--allowed-hosts", "mcp.app-test.wonton.internal"]
    );
}

#[test]
fn bot_declarations_need_matrix_bot_and_real_services() {
    let parse =
        |manifest: &str| PluginPackage::parse(&package(SERVICES, manifest), &super::langs());
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
        PluginPackage::parse(&agent, &super::langs()),
        Err(PackageError::Manifest(_))
    ));
    assert!(matches!(
        parse(&BOTS.replace("[init, gateway]", "[init, nope]")),
        Err(PackageError::UnknownService { .. })
    ));
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

#[test]
fn model_variables_are_injected_only_for_plugins_that_declare_models() {
    let web = "  web:\n    image: nginx\n    environment:\n      BASE: ${WONTON_MODEL_BASE_URL}\n      \
               KEY: ${WONTON_MODEL_API_KEY}\n      MODEL: ${WONTON_MODEL_DEFAULT}";
    let plan = plan(web, "  permissions: [models]").unwrap();
    let env = env_of(container(&plan, "web"));
    for expected in [
        "BASE=http://models.wonton.internal/v1",
        "KEY=dry-run",
        "MODEL=dry-run",
    ] {
        assert!(
            env.contains(&expected.to_owned()),
            "缺少 {expected}：{env:?}"
        );
    }
    // 没声明 models 却引用它的变量：是插件写错了，照常报变量未定义。
    assert!(matches!(
        plan_err(web, "  permissions: []"),
        RenderError::Package(PackageError::Interpolate { .. })
    ));
}

#[test]
fn install_contexts_print_no_tokens_or_secrets() {
    let ctx = InstallContext {
        slug: "app-test".into(),
        instance: Default::default(),
        hosted_root: std::path::PathBuf::from("/hosted"),
        app_data: "/apps/app-test".into(),
        relay_dir: "/run/wonton/relay".into(),
        config_store: "/data/core/configs".into(),
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
        owner: None,
        matrix: Some(MatrixVars {
            homeserver: "http://matrix.wonton.internal".into(),
            user_id: "@test:t".into(),
            access_token: "matrix-token-value".into(),
            allowed_users: "@owner:t".into(),
            home_room: "!room:t".into(),
        }),
        bots: Default::default(),
        models: Some(ModelVars {
            base_url: "http://models.wonton.internal/v1".into(),
            anthropic_url: None,
            gemini_url: None,
            plugin_key: Some("model-key-value".into()),
            bot_keys: Default::default(),
            default_model: "group/default".into(),
        }),
        capabilities: Default::default(),
        mcp_url: None,
        notify_url: None,
        secrets: [("web_key".to_owned(), "generated-secret-value".to_owned())].into(),
        tz: "UTC".into(),
        puid: 1000,
        pgid: 1000,
        install_limit: crate::config::DEFAULT_INSTALL_TIMEOUT,
        extra_hosts: Vec::new(),
        gpu: None,
        lan: None,
        egress: None,
        core_address: None,
        memory_total: None,
        host_name: "server-01".into(),
        limits: Default::default(),
        cgroup_parent: None,
        settings: Default::default(),
        granted: Vec::new(),
    };
    let printed = format!("{ctx:?}");
    for secret in [
        "oidc-secret-value",
        "matrix-token-value",
        "model-key-value",
        "generated-secret-value",
    ] {
        assert!(!printed.contains(secret), "{printed}");
    }
    assert!(printed.contains("web_key"), "只打印密钥的名字：{printed}");
}

#[test]
fn a_room_only_has_the_bots_whose_home_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let store = BotStore::new(dir.path().to_owned());
    let bot = |user_id: &str, home_room: &str| {
        (
            DEFAULT_BOT.to_owned(),
            BotEntry {
                account: BotAccount {
                    user_id: user_id.to_owned(),
                    home_room: home_room.to_owned(),
                },
                device_id: None,
                keeper: BotKeeper::User {
                    username: "owner".to_owned(),
                },
                bus_token: "token".to_owned(),
            },
        )
    };
    let record = |(name, entry): (String, BotEntry)| PluginBots {
        bots: [(name, entry)].into_iter().collect(),
    };
    let id = |name: &str| PluginId::parse(name).unwrap();
    store
        .save(&id("app.a"), &record(bot("@a:home", "!a:home")))
        .unwrap();
    store
        .save(&id("app.b"), &record(bot("@b:home", "!b:home")))
        .unwrap();
    assert_eq!(store.localparts_in_room("!a:home").unwrap(), ["a"]);
    assert!(store.localparts_in_room("!people:home").unwrap().is_empty());
}
