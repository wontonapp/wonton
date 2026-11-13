//! 清单的 `clients` 和 compose 对得上：服务存在，引用 `${WONTON_EXTERNAL_URL}` 时要开得出入口。

use super::super::package::PluginPackage;

const HEAD: &str = "  spec: \"0.13\"\n  id: app.test\n  version: 1.0.0\n  kind: app\n  \
                    title: {zh: 测试}\n  description: {zh: 测试}\n";

fn head(permissions: &str) -> String {
    format!("{HEAD}  permissions: [{permissions}]\n")
}

fn compose(service: &str, manifest: &str) -> String {
    format!("services:\n  web:\n    image: nginx\n{service}x-wonton:\n{manifest}")
}

fn parse(services: &str, manifest: &str) -> Result<PluginPackage, String> {
    PluginPackage::parse(
        &compose(services, manifest),
        &super::super::languages::Languages::from_header(Some("zh-Hans")),
    )
    .map_err(|error| error.to_string())
}

const HTTP: &str = "  clients:\n    http:\n      service: web\n      port: 3000\n      auth:\n        app:\n          setup_done: [sh, /setup-done.sh]\n";

#[test]
fn client_service_must_exist() {
    let error = parse(
        "",
        &format!(
            "{}  clients:\n    tcp:\n      - {{ service: db, port: 22, name: ssh }}\n",
            head("network.clients")
        ),
    )
    .unwrap_err();
    assert!(error.contains("clients.tcp"), "{error}");
    assert!(error.contains("db"), "{error}");
}

#[test]
fn external_url_needs_an_entry() {
    let service = "    environment:\n      URL: ${WONTON_EXTERNAL_URL}\n";
    let error = parse(service, &head("")).unwrap_err();
    assert!(error.contains("WONTON_EXTERNAL_URL"), "{error}");
    parse(service, &format!("{}{HTTP}", head("network.clients"))).unwrap();
    parse(
        service,
        &format!(
            "{}  webui: {{service: web, port: 8080, guests: true}}\n",
            head("network.clients")
        ),
    )
    .unwrap();
}

#[test]
fn clients_http_rejects_per_bot_service_before_deployment() {
    let error = parse(
        "",
        &format!(
            "{}{HTTP}  bots: {{services: [web]}}\n",
            head("network.clients, matrix.bot")
        ),
    )
    .unwrap_err();
    assert!(error.contains("clients.http.service"), "{error}");
    assert!(error.contains("web"), "{error}");
}

#[test]
fn clients_http_rejects_per_bot_shared_network_service_before_deployment() {
    let error = parse(
        "    network_mode: service:target\n  target:\n    image: nginx\n",
        &format!(
            "{}{HTTP}  bots: {{services: [web, target]}}\n",
            head("network.clients, matrix.bot")
        ),
    )
    .unwrap_err();
    assert!(error.contains("clients.http.service"), "{error}");
    assert!(error.contains("web"), "{error}");
}

#[test]
fn clients_http_accepts_plugin_wide_shared_network_service() {
    let package = parse(
        "    network_mode: service:target\n  target:\n    image: nginx\n",
        &format!("{}{HTTP}", head("network.clients")),
    )
    .unwrap();
    let plan = super::super::render::dry_run(
        &package,
        &super::super::languages::Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    let http = package
        .manifest
        .clients
        .as_ref()
        .unwrap()
        .http
        .as_ref()
        .unwrap();
    assert_eq!(http.service, "web");
    let web = plan.containers.iter().find(|c| c.service == "web").unwrap();
    let target = plan
        .containers
        .iter()
        .find(|c| c.service == "target")
        .unwrap();
    assert_eq!(
        web.body.host_config.as_ref().unwrap().network_mode,
        Some(format!("container:{}", target.name))
    );
    assert!(web.body.networking_config.is_none());
    assert!(target.body.networking_config.is_some());
}

#[test]
fn clients_http_does_not_enable_chained_network_sharing() {
    let package = parse(
        "    network_mode: service:target\n  target:\n    image: nginx\n    network_mode: service:outer\n  outer:\n    image: nginx\n",
        &format!("{}{HTTP}", head("network.clients")),
    )
    .unwrap();
    assert!(
        super::super::render::dry_run(
            &package,
            &super::super::languages::Languages::from_header(Some("zh-Hans")),
        )
        .is_err()
    );
}

fn clients_plan(services: &str) -> super::super::render::Deployment {
    let package = parse(services, &format!("{}{HTTP}", head("network.clients"))).unwrap();
    super::super::render::dry_run(
        &package,
        &super::super::languages::Languages::from_header(Some("zh-Hans")),
    )
    .unwrap()
}

#[test]
fn clients_http_resolves_normal_and_shared_network_services() {
    let normal = clients_plan("");
    assert_eq!(normal.clients_http_network_service("web"), Some("web"));
    let shared = clients_plan("    network_mode: service:target\n  target:\n    image: nginx\n");
    assert_eq!(shared.clients_http_network_service("web"), Some("target"));
    assert_eq!(
        shared.clients_http_network_service("target"),
        Some("target")
    );
}

#[test]
fn clients_http_network_resolution_rejects_missing_or_ambiguous_containers() {
    let plan = clients_plan("    network_mode: service:target\n  target:\n    image: nginx\n");
    assert_eq!(plan.clients_http_network_service("missing"), None);
    let target_index = plan
        .containers
        .iter()
        .position(|c| c.service == "target")
        .unwrap();
    let web_index = plan
        .containers
        .iter()
        .position(|c| c.service == "web")
        .unwrap();
    let mut missing = plan.clone();
    missing.containers.remove(target_index);
    assert_eq!(missing.clients_http_network_service("web"), None);
    for index in [target_index, web_index] {
        let mut duplicate = plan.clone();
        duplicate.containers.push(plan.containers[index].clone());
        assert_eq!(duplicate.clients_http_network_service("web"), None);
        let mut per_bot = plan.clone();
        per_bot.containers[index].bot = Some("default".into());
        assert_eq!(per_bot.clients_http_network_service("web"), None);
    }
    for mode in ["host", "none", "container:missing"] {
        let mut invalid_target = plan.clone();
        invalid_target.containers[target_index]
            .body
            .host_config
            .as_mut()
            .unwrap()
            .network_mode = Some(mode.into());
        assert_eq!(invalid_target.clients_http_network_service("web"), None);
        let mut invalid_source = plan.clone();
        invalid_source.containers[web_index]
            .body
            .host_config
            .as_mut()
            .unwrap()
            .network_mode = Some(mode.into());
        assert_eq!(invalid_source.clients_http_network_service("web"), None);
    }
}
