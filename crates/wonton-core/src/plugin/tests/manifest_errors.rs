//! 清单和服务反序列化失败时，报错带字段路径和行号（#564）：插件作者按报错直接改到字段，不用二分。

use super::super::package::PluginPackage;
use super::render_err;

const WEB: &str = "services:\n  web:\n    image: nginx\n";

fn compose(services: &str, manifest: &str) -> String {
    format!("{services}x-wonton:\n{manifest}")
}

fn parse_err(manifest: &str) -> String {
    PluginPackage::parse(&compose(WEB, manifest))
        .expect_err("清单应不合法")
        .to_string()
}

#[test]
fn stray_comma_in_flow_mapping_names_the_field_and_line() {
    // issue 里的写法：英文逗号让流式映射多拆出一个空值的键，报错要指到 description 和那一行。
    let error = parse_err(
        "  spec: '0.6'\n  id: app.test\n  version: 1.0.0\n  kind: app\n  \
         title: {zh: 测试, en: Test}\n  description: {zh: 测试, en: Run X, Y and Z}\n  \
         permissions: []\n",
    );
    assert!(error.contains("x-wonton.description"), "{error}");
    assert!(error.contains("line 10"), "{error}");
}

#[test]
fn unknown_manifest_field_names_its_path() {
    let error = parse_err(
        "  spec: '0.6'\n  id: app.test\n  version: 1.0.0\n  kind: app\n  \
         title: {zh: 测试, en: Test}\n  description: {zh: 测试, en: Test}\n  \
         permissions: []\n  webui: {service: web, port: 8080, wat: 1}\n",
    );
    assert!(error.contains("x-wonton.webui.wat"), "{error}");
}

#[test]
fn wrong_type_names_its_path() {
    let error = parse_err(
        "  spec: '0.6'\n  id: app.test\n  version: 1.0.0\n  kind: app\n  \
         title: {zh: 测试, en: Test}\n  description: {zh: 测试, en: Test}\n  \
         permissions: 只读\n",
    );
    assert!(error.contains("x-wonton.permissions"), "{error}");
}

#[test]
fn missing_required_field_says_which_and_where() {
    let error = parse_err(
        "  spec: '0.6'\n  version: 1.0.0\n  kind: app\n  \
         title: {zh: 测试, en: Test}\n  description: {zh: 测试, en: Test}\n  \
         permissions: []\n",
    );
    assert!(error.contains("missing field `id`"), "{error}");
    assert!(error.contains("line"), "{error}");
}

#[test]
fn service_field_errors_carry_a_path() {
    // 服务不在 parse 时反序列化，渲染时才报；报错要带服务内的字段路径。
    let error = render_err(&compose(
        "services:\n  web:\n    image: nginx\n    restart: []\n",
        "  spec: '0.6'\n  id: app.test\n  version: 1.0.0\n  kind: app\n  \
         title: {zh: 测试, en: Test}\n  description: {zh: 测试, en: Test}\n  \
         permissions: []\n",
    ));
    assert!(error.contains("`restart`"), "{error}");
}
