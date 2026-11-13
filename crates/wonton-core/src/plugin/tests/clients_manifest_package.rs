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
